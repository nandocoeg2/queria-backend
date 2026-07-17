use crate::http;
use axum::Router;
use queria_core::AppConfig;
use queria_search::retrieval::{PgRetrievalService, build_pg_retrieval_service};
use sqlx::PgPool;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

/// Process-level MCP state with a long-lived retrieval service (shared clients).
#[derive(Clone)]
pub struct McpState {
    pub config: AppConfig,
    pub pool: Option<PgPool>,
    pub retrieval: Option<Arc<PgRetrievalService>>,
}

impl McpState {
    #[must_use]
    pub fn project_repository(&self) -> Option<queria_db::repositories::PgProjectRepository> {
        self.pool
            .clone()
            .map(queria_db::repositories::PgProjectRepository::new)
    }
}

pub fn build_app() -> Router {
    build_app_with_state(McpState {
        config: AppConfig::default_local(),
        pool: None,
        retrieval: None,
    })
}

pub fn build_app_with_pool(config: AppConfig, pool: PgPool) -> Router {
    let retrieval = match build_pg_retrieval_service(&config, pool.clone()) {
        Ok(service) => Some(Arc::new(service)),
        Err(error) => {
            tracing::error!(
                error = %error,
                "failed to construct retrieval service at MCP startup"
            );
            None
        }
    };
    build_app_with_state(McpState {
        config,
        pool: Some(pool),
        retrieval,
    })
}

fn build_app_with_state(state: McpState) -> Router {
    Router::new()
        .merge(http::router())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
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

    #[tokio::test]
    async fn mcp_endpoint_requires_agent_token() {
        let response = build_app()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
