use crate::http;
use axum::Router;
use tower_http::trace::TraceLayer;

pub fn build_app() -> Router {
    Router::new()
        .merge(http::router())
        .layer(TraceLayer::new_for_http())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::http::{Request, StatusCode};
    use axum::body::Body;
    use tower::ServiceExt;

    #[tokio::test]
    async fn mcp_health_endpoint_returns_ok() {
        let response = build_app()
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::OK);
    }
}
