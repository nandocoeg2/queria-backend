use axum::{Json, Router, routing::get};
use serde::Serialize;

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route("/healthz", get(healthz))
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "queria-api",
    })
}
