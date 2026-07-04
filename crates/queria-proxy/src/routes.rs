use crate::health;
use axum::{Router, routing::get};
use tower_http::trace::TraceLayer;

pub fn build_router() -> Router {
    Router::new()
        .route("/healthz", get(health::healthz))
        .layer(TraceLayer::new_for_http())
}
