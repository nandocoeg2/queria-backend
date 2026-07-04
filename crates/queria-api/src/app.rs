use crate::http::{auth, health, projects, retrieval, setup, sources, tokens};
use axum::Router;
use queria_core::AppConfig;
use queria_db::repositories::{PgAuthRepository, PgProjectRepository};
use sqlx::PgPool;
use tower_http::trace::TraceLayer;

#[derive(Clone, Debug)]
pub struct ApiState {
    pub config: AppConfig,
    pub pool: Option<PgPool>,
}

impl ApiState {
    #[must_use]
    pub fn auth_repository(&self) -> Option<PgAuthRepository> {
        self.pool.clone().map(PgAuthRepository::new)
    }

    #[must_use]
    pub fn project_repository(&self) -> Option<PgProjectRepository> {
        self.pool.clone().map(PgProjectRepository::new)
    }
}

pub fn build_app(config: AppConfig) -> Router {
    build_app_with_state(ApiState { config, pool: None })
}

pub fn build_app_with_pool(config: AppConfig, pool: PgPool) -> Router {
    build_app_with_state(ApiState {
        config,
        pool: Some(pool),
    })
}

fn build_app_with_state(state: ApiState) -> Router {
    Router::new()
        .merge(health::router())
        .nest("/api/v1/setup", setup::router())
        .nest("/api/v1/auth", auth::router())
        .nest("/api/v1/projects", projects::router())
        .nest("/api/v1/sources", sources::router())
        .nest("/api/v1/retrieval", retrieval::router())
        .nest("/api/v1/agent-tokens", tokens::router())
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
    async fn health_endpoint_returns_ok() {
        let app = build_app(AppConfig::default_local());

        let response = app
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
    async fn login_endpoint_fails_closed_until_user_store_is_wired() {
        let app = build_app(AppConfig::default_local());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"email":"nando@fjulian.id","password":"correct horse battery staple"}"#,
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn token_creation_fails_closed_without_authenticated_admin() {
        let app = build_app(AppConfig::default_local());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/agent-tokens")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn token_listing_fails_closed_without_authenticated_admin() {
        let app = build_app(AppConfig::default_local());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/agent-tokens")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn token_revoke_fails_closed_without_authenticated_admin() {
        let app = build_app(AppConfig::default_local());

        let response = app
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/agent-tokens/019083a0-0000-7000-8000-000000000002")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn projects_endpoint_requires_session_cookie() {
        let app = build_app(AppConfig::default_local());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/projects")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn sources_endpoint_requires_session_cookie() {
        let app = build_app(AppConfig::default_local());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sources?project_slug=fjulian-me")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn retrieval_endpoint_requires_session_cookie() {
        let app = build_app(AppConfig::default_local());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/retrieval/retrieve-context")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{
                            "project_id": "019083a0-0000-7000-8000-000000000001",
                            "query": "deployment notes",
                            "include_global": true,
                            "limit": 5
                        }"#,
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
