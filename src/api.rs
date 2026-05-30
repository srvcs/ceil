use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use utoipa::{OpenApi, ToSchema};

use crate::client::{self, DepError};

pub const SERVICE: &str = "srvcs-ceil";
pub const CONCERN: &str = "rounding: ceiling (toward +infinity)";
pub const DEPENDS_ON: &[&str] = &["srvcs-isnumber"];

/// Dependency endpoints, injected as router state so tests can point them at
/// mock services.
#[derive(Clone)]
pub struct Deps {
    pub isnumber_url: String,
}

#[derive(Serialize, ToSchema)]
pub struct Info {
    pub service: &'static str,
    pub concern: &'static str,
    pub depends_on: Vec<&'static str>,
}

/// `GET /` — service identity (srvcs service standard).
#[utoipa::path(get, path = "/", responses((status = 200, body = Info)))]
pub async fn index() -> Json<Info> {
    Json(Info {
        service: SERVICE,
        concern: CONCERN,
        depends_on: DEPENDS_ON.to_vec(),
    })
}

#[derive(Deserialize, ToSchema)]
pub struct EvalRequest {
    #[schema(value_type = Object)]
    pub value: Value,
}

#[derive(Serialize, ToSchema)]
pub struct ResultResponse {
    #[schema(value_type = Object)]
    pub value: Value,
    pub result: i64,
}

/// The single concern: the ceiling of a real number (round toward +infinity).
///
/// Integers are unchanged (`ceil(5) == 5`); fractional values round up
/// (`ceil(4.2) == 5`, `ceil(-4.7) == -4`).
pub fn ceil(x: f64) -> i64 {
    x.ceil() as i64
}

fn ok(value: Value, result: i64) -> Response {
    (
        StatusCode::OK,
        Json(json!({ "value": value, "result": result })),
    )
        .into_response()
}

fn invalid(reason: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({ "error": reason })),
    )
        .into_response()
}

fn degraded(dependency: &str) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({ "error": "dependency unavailable", "dependency": dependency })),
    )
        .into_response()
}

fn forward(status: u16, body: Value) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
    (code, Json(body)).into_response()
}

/// Ask `srvcs-isnumber` whether `value` is a number, mapping its failures to the
/// response this service should return verbatim.
///
/// Returns `Ok(())` if the dependency confirms `value` is a number; otherwise an
/// error `Response` (422 forwarded / 503 degraded) the caller should return.
async fn ask_isnumber(url: &str, value: &Value) -> Result<(), Response> {
    match client::call(url, &json!({ "value": value })).await {
        Err(DepError::Unreachable) => Err(degraded("srvcs-isnumber")),
        Ok((200, body)) => {
            let is_number = body.get("result").and_then(Value::as_bool).unwrap_or(false);
            if is_number {
                Ok(())
            } else {
                Err(invalid("value is not a number"))
            }
        }
        // Invalid input propagates from the leaf dependency; forward it.
        Ok((422, body)) => Err(forward(422, body)),
        Ok(_) => Err(degraded("srvcs-isnumber")),
    }
}

/// `POST /` — round `value` up to the nearest integer (toward +infinity).
///
/// Input validation is delegated to `srvcs-isnumber` over HTTP (the single
/// source of truth for "is this a number"). If that dependency is unreachable,
/// this service reports itself degraded rather than guessing. Unlike parity
/// services, `ceil` accepts fractional floats — rounding them is the whole job.
#[utoipa::path(
    post,
    path = "/",
    request_body = EvalRequest,
    responses(
        (status = 200, body = ResultResponse),
        (status = 422, description = "value is not a number"),
        (status = 500, description = "value is numeric but not representable as f64"),
        (status = 503, description = "a dependency is unavailable")
    )
)]
pub async fn evaluate(State(deps): State<Deps>, Json(req): Json<EvalRequest>) -> Response {
    // 1. Delegate "is this a number" to srvcs-isnumber.
    if let Err(resp) = ask_isnumber(&deps.isnumber_url, &req.value).await {
        return resp;
    }

    // 2. Coerce to f64 and round up. isnumber has confirmed this is a JSON
    //    number; if it somehow isn't representable as f64, that's an internal
    //    inconsistency, not bad user input.
    let Some(x) = req.value.as_f64() else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "value is not representable as a real number" })),
        )
            .into_response();
    };

    ok(req.value, ceil(x))
}

#[derive(OpenApi)]
#[openapi(
    paths(index, evaluate),
    components(schemas(Info, EvalRequest, ResultResponse))
)]
pub struct ApiDoc;

/// Serve OpenAPI document
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_documents_routes() {
        let doc = ApiDoc::openapi();
        let root = doc.paths.paths.get("/").expect("path / present");
        assert!(root.get.is_some());
        assert!(root.post.is_some());
    }

    #[test]
    fn ceil_rounds_toward_positive_infinity() {
        assert_eq!(ceil(4.2), 5);
        assert_eq!(ceil(-4.7), -4);
        assert_eq!(ceil(5.0), 5);
        assert_eq!(ceil(-5.0), -5);
        assert_eq!(ceil(0.0), 0);
        assert_eq!(ceil(0.1), 1);
        assert_eq!(ceil(-0.1), 0);
    }

    #[tokio::test]
    async fn index_reports_concern_and_dependency() {
        let Json(info) = index().await;
        assert_eq!(info.service, "srvcs-ceil");
        assert_eq!(info.concern, "rounding: ceiling (toward +infinity)");
        assert_eq!(info.depends_on, vec!["srvcs-isnumber"]);
    }
}
