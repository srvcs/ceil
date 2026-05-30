# srvcs-ceil

The ceiling primitive of the srvcs.cloud distributed standard library.

Its single concern: **round a number up toward +infinity.** It does not validate
input itself — it delegates "is this a number" to
[`srvcs-isnumber`](https://github.com/srvcs/isnumber) over HTTP, the single
source of truth for that question. The ceiling is then computed on the value as a
real number (`req.value.ceil() as i64`).

Unlike the parity services, `srvcs-ceil` accepts fractional floats — rounding
them is the whole job: `ceil(4.2) == 5`, `ceil(-4.7) == -4`, `ceil(5) == 5`.

If `srvcs-isnumber` is unreachable, `srvcs-ceil` reports itself **degraded
(503)** rather than guessing.

## API

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/` | Service identity, concern, and dependency list |
| `POST` | `/` | Round `value` up to the nearest integer |
| `GET` | `/healthz` `/readyz` `/metrics` `/openapi.json` | srvcs service standard surface |

```sh
curl -s -X POST localhost:8080/ -H 'content-type: application/json' -d '{"value": 4.2}'
# {"value":4.2,"result":5}
```

Responses:

- `200 {"value": n, "result": i64}` — evaluated.
- `422` — the value is not a number (per `srvcs-isnumber`).
- `500` — the value is numeric but not representable as a real number.
- `503` — a dependency is unavailable.

## Dependencies

- [`srvcs-isnumber`](https://github.com/srvcs/isnumber) — input validation.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `SRVCS_BIND_ADDR` | `0.0.0.0:8080` | Bind address |
| `SRVCS_ISNUMBER_URL` | `http://127.0.0.1:8081` | Base URL of `srvcs-isnumber` |
| `SRVCS_ENV` | `development` | Environment label for logs |
| `RUST_LOG` | `info,tower_http=info` | Tracing filter |

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Orchestration tests stand up a mock `srvcs-isnumber` in-process, so the suite
runs without the rest of the fleet. See
[`srvcs/platform`](https://github.com/srvcs/platform) for the shared standard.

> Note: the `cargoHash` in `flake.nix` is inherited from the template and must be
> refreshed with a `nix build` before the Nix gates pass.
