use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::post;
use axum::{Json, Router as AxumRouter};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use srvcs_ceil::{api::Deps, health, router, telemetry};
use tower::ServiceExt;

/// Spin up a mock `srvcs-isnumber` that actually COMPUTES its answer: it reads
/// the request body's `value` and reports whether that JSON value is genuinely a
/// number. This makes the orchestration test exercise the real validation path
/// rather than a canned response.
async fn spawn_isnumber() -> String {
    let app = AxumRouter::new().route(
        "/",
        post(|Json(body): Json<Value>| async move {
            let is_number = body.get("value").map(Value::is_number).unwrap_or(false);
            (StatusCode::OK, Json(json!({ "result": is_number })))
        }),
    );
    spawn(app).await
}

/// Spin up a mock dependency answering `POST /` with a fixed `(status, body)`.
async fn spawn_fixed(status: StatusCode, body: Value) -> String {
    let app = AxumRouter::new().route(
        "/",
        post(move || {
            let body = body.clone();
            async move { (status, Json(body)) }
        }),
    );
    spawn(app).await
}

async fn spawn(app: AxumRouter) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn app(isnumber_url: &str) -> axum::Router {
    router(
        telemetry::metrics_handle_for_tests(),
        Deps {
            isnumber_url: isnumber_url.to_string(),
        },
    )
}

async fn eval(isnumber_url: &str, value: Value) -> (StatusCode, Value) {
    let res = app(isnumber_url)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "value": value }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

// A base URL with nothing listening — exercises the degraded path.
const DEAD_URL: &str = "http://127.0.0.1:1";

async fn status_of(uri: &str) -> StatusCode {
    app(DEAD_URL)
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn index_ok() {
    assert_eq!(status_of("/").await, StatusCode::OK);
}

#[tokio::test]
async fn healthz_ok() {
    assert_eq!(status_of("/healthz").await, StatusCode::OK);
}

#[tokio::test]
async fn readyz_reflects_state() {
    health::set_ready(true);
    assert_eq!(status_of("/readyz").await, StatusCode::OK);
}

#[tokio::test]
async fn openapi_ok() {
    assert_eq!(status_of("/openapi.json").await, StatusCode::OK);
}

#[tokio::test]
async fn ceils_positive_fraction_up() {
    let isnumber = spawn_isnumber().await;
    let (status, body) = eval(&isnumber, json!(4.2)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 5);
    assert_eq!(body["value"], 4.2);
}

#[tokio::test]
async fn ceils_negative_fraction_toward_zero() {
    let isnumber = spawn_isnumber().await;
    let (status, body) = eval(&isnumber, json!(-4.7)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], -4);
}

#[tokio::test]
async fn integer_is_unchanged() {
    let isnumber = spawn_isnumber().await;
    let (status, body) = eval(&isnumber, json!(5)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], 5);
}

#[tokio::test]
async fn negative_integer_is_unchanged() {
    let isnumber = spawn_isnumber().await;
    let (_, body) = eval(&isnumber, json!(-5)).await;
    assert_eq!(body["result"], -5);
}

#[tokio::test]
async fn rejects_value_isnumber_says_is_not_a_number() {
    // The computing mock reports `false` for a string, so ceil returns 422.
    let isnumber = spawn_isnumber().await;
    let (status, _) = eval(&isnumber, json!("nope")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn forwards_422_from_isnumber() {
    // If isnumber rejects the input outright with a 422, ceil forwards it.
    let isnumber = spawn_fixed(
        StatusCode::UNPROCESSABLE_ENTITY,
        json!({ "error": "not a number" }),
    )
    .await;
    let (status, body) = eval(&isnumber, json!("nope")).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "not a number");
}

#[tokio::test]
async fn degrades_when_isnumber_is_unreachable() {
    let (status, body) = eval(DEAD_URL, json!(4.2)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["dependency"], "srvcs-isnumber");
}

#[tokio::test]
async fn degrades_when_isnumber_returns_unexpected_status() {
    let isnumber = spawn_fixed(StatusCode::INTERNAL_SERVER_ERROR, json!({})).await;
    let (status, _) = eval(&isnumber, json!(4.2)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}
