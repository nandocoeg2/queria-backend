//! Agent-bearer retrieve path for client-side auto-retrieve hooks.
//!
//! Shell hooks (Droid/Claude SessionStart + UserPromptSubmit) cannot call
//! Streamable HTTP MCP cleanly. This surface reuses the same hybrid pipeline
//! and `RetrievalPrincipal::Agent` as MCP `retrieve_context`.
//!
//! Session-cookie routes in `retrieval.rs` are unchanged.

use crate::app::ApiState;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    routing::{get, post},
};
use queria_core::auth::agent_token::AgentTokenIssuer;
use queria_core::auth::permissions::AgentToolPermission;
use queria_core::contracts::{RetrieveContextRequest, RetrieveContextResponse};
use queria_core::ids::ProjectId;
use queria_core::QueriaError;
use queria_db::repositories::{AuthenticatedAgentToken, PgProjectRepository, ProjectRecord};
use queria_search::retrieval::RetrievalPrincipal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Hook path clamps tighter than MCP max 20 to keep inject budgets small.
const HOOK_LIMIT_MAX: u32 = 10;
const HOOK_LIMIT_DEFAULT: u32 = 5;

#[derive(Debug, Deserialize)]
struct AgentRetrieveRequest {
    #[serde(default)]
    project_id: Option<ProjectId>,
    #[serde(default)]
    project_slug: Option<String>,
    query: String,
    include_global: Option<bool>,
    /// Agent/hook default true (dual-lane).
    include_scratch: Option<bool>,
    limit: Option<u32>,
    rerank: Option<bool>,
    compress: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/agent/retrieve-context", post(agent_retrieve_context))
        .route("/agent/projects", get(agent_list_projects))
}

async fn agent_list_projects(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Value> {
    // Auth header first so missing bearer is 401 even without a configured pool.
    let raw = require_raw_bearer(&headers)?;
    let repository = project_repository(&state)?;
    let agent = authenticate_raw(&repository, raw).await?;
    // Listing projects does not require RetrieveContext; ListProjects permission if present,
    // else allow any authenticated non-revoked token (hooks need bootstrap).
    if !agent.permissions.can_call(&AgentToolPermission::ListProjects)
        && !agent
            .permissions
            .can_call(&AgentToolPermission::RetrieveContext)
    {
        return Err(error(StatusCode::FORBIDDEN, "permission_denied"));
    }

    let projects = repository
        .list_projects_for_agent(&agent)
        .await
        .map_err(map_infra)?;
    Ok(Json(json!({
        "projects": projects.into_iter().map(project_json).collect::<Vec<_>>()
    })))
}

async fn agent_retrieve_context(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<AgentRetrieveRequest>,
) -> ApiResult<RetrieveContextResponse> {
    let raw = require_raw_bearer(&headers)?;
    let repository = project_repository(&state)?;
    let agent = authenticate_raw(&repository, raw).await?;
    if !agent
        .permissions
        .can_call(&AgentToolPermission::RetrieveContext)
    {
        return Err(error(StatusCode::FORBIDDEN, "permission_denied"));
    }

    let project_id = resolve_project_id(&repository, &agent, &payload).await?;
    let limit = clamp_hook_limit(payload.limit.unwrap_or(HOOK_LIMIT_DEFAULT));
    let request = RetrieveContextRequest {
        project_id,
        query: payload.query,
        include_global: payload.include_global.unwrap_or(true),
        include_scratch: payload.include_scratch.unwrap_or(true),
        limit,
        rerank: payload.rerank,
        compress: payload.compress,
    };
    request.validate().map_err(map_validate)?;

    let service = state.retrieval.as_ref().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "knowledge_store_not_configured",
        )
    })?;

    service
        .retrieve_context(
            &RetrievalPrincipal::Agent {
                organization_id: agent.organization_id,
                project_slugs: agent.permissions.project_slugs.clone(),
                allow_global_knowledge: agent.permissions.allow_global_knowledge,
            },
            request,
        )
        .await
        .map(Json)
        .map_err(map_retrieve)
}

async fn resolve_project_id(
    repository: &PgProjectRepository,
    agent: &AuthenticatedAgentToken,
    payload: &AgentRetrieveRequest,
) -> Result<ProjectId, (StatusCode, Json<ErrorResponse>)> {
    match (
        payload.project_id,
        payload
            .project_slug
            .as_ref()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
    ) {
        (Some(id), _) => {
            // Ensure the agent may see this project (same list path as MCP list_projects).
            let projects = repository
                .list_projects_for_agent(agent)
                .await
                .map_err(map_infra)?;
            if projects.iter().any(|p| p.id == id.as_uuid()) {
                Ok(id)
            } else {
                Err(error(StatusCode::FORBIDDEN, "project_not_allowed"))
            }
        }
        (None, Some(slug)) => {
            if !valid_slug(&slug) {
                return Err(error(StatusCode::BAD_REQUEST, "invalid_project_slug"));
            }
            if !agent.permissions.project_slugs.iter().any(|s| s == &slug) {
                return Err(error(StatusCode::FORBIDDEN, "project_not_allowed"));
            }
            let projects = repository
                .list_projects_for_agent(agent)
                .await
                .map_err(map_infra)?;
            let project = projects
                .into_iter()
                .find(|p| p.slug == slug)
                .ok_or_else(|| error(StatusCode::NOT_FOUND, "project_not_found"))?;
            Ok(ProjectId::from_uuid(project.id))
        }
        (None, None) => Err(error(
            StatusCode::BAD_REQUEST,
            "project_id_or_project_slug_required",
        )),
    }
}

fn clamp_hook_limit(limit: u32) -> u32 {
    limit.clamp(1, HOOK_LIMIT_MAX)
}

fn project_repository(
    state: &ApiState,
) -> Result<PgProjectRepository, (StatusCode, Json<ErrorResponse>)> {
    state.project_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "knowledge_store_not_configured",
        )
    })
}

fn require_raw_bearer(
    headers: &HeaderMap,
) -> Result<&str, (StatusCode, Json<ErrorResponse>)> {
    bearer_token(headers).ok_or_else(|| error(StatusCode::UNAUTHORIZED, "agent_token_required"))
}

async fn authenticate_raw(
    repository: &PgProjectRepository,
    raw: &str,
) -> Result<AuthenticatedAgentToken, (StatusCode, Json<ErrorResponse>)> {
    let token_hash = AgentTokenIssuer::hash_token(raw);
    repository
        .authenticate_agent_token(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "agent token authentication failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "repository_failed")
        })?
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "agent_token_required"))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| token.starts_with("qria_"))
}

fn project_json(project: ProjectRecord) -> Value {
    json!({
        "id": project.id,
        "slug": project.slug,
        "name": project.name,
        "description": project.description,
        "default_embedding_model": project.default_embedding_model,
        "include_global_default": project.include_global_default,
        "created_at": project.created_at,
        "updated_at": project.updated_at
    })
}

fn map_infra(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %err, "agent retrieval repository failed");
    error(StatusCode::INTERNAL_SERVER_ERROR, "repository_failed")
}

fn map_validate(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        other => {
            tracing::error!(error = %other, "unexpected validate error");
            error(StatusCode::BAD_REQUEST, "invalid_request")
        }
    }
}

fn map_retrieve(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::FORBIDDEN, "permission_denied")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "agent retrieve pipeline failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "repository_failed")
        }
    }
}

fn error(status: StatusCode, message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error: message.to_owned(),
        }),
    )
}

fn valid_slug(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    let Some(last) = bytes.last() else {
        return false;
    };

    (3..=64).contains(&bytes.len())
        && first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

/// Resolve project id from agent request inputs (testable pure helper for slug path rules).
#[cfg(test)]
fn resolve_selector(
    project_id: Option<uuid::Uuid>,
    project_slug: Option<&str>,
) -> Result<(), &'static str> {
    match (
        project_id,
        project_slug.map(str::trim).filter(|s| !s.is_empty()),
    ) {
        (Some(_), _) => Ok(()),
        (None, Some(slug)) if valid_slug(slug) => Ok(()),
        (None, Some(_)) => Err("invalid_project_slug"),
        (None, None) => Err("project_id_or_project_slug_required"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::build_app;
    use axum::body::Body;
    use http::Request;
    use queria_core::AppConfig;
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn body_string(response: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }

    #[test]
    fn clamp_hook_limit_bounds() {
        assert_eq!(clamp_hook_limit(0), 1);
        assert_eq!(clamp_hook_limit(5), 5);
        assert_eq!(clamp_hook_limit(10), 10);
        assert_eq!(clamp_hook_limit(20), 10);
        assert_eq!(clamp_hook_limit(100), 10);
    }

    #[test]
    fn agent_retrieve_request_defaults() {
        let payload: AgentRetrieveRequest = serde_json::from_str(
            r#"{
                "project_slug": "fjulian-me",
                "query": "how does deploy work"
            }"#,
        )
        .expect("deserialize");
        assert!(payload.include_scratch.unwrap_or(true));
        assert!(payload.include_global.unwrap_or(true));
        assert_eq!(payload.limit.unwrap_or(HOOK_LIMIT_DEFAULT), 5);
        assert!(payload.rerank.is_none());
        assert!(payload.compress.is_none());
    }

    #[test]
    fn selector_requires_id_or_slug() {
        assert_eq!(
            resolve_selector(None, None),
            Err("project_id_or_project_slug_required")
        );
        assert_eq!(
            resolve_selector(None, Some("BAD_SLUG")),
            Err("invalid_project_slug")
        );
        assert!(resolve_selector(None, Some("fjulian-me")).is_ok());
        assert!(resolve_selector(Some(Uuid::nil()), None).is_ok());
    }

    #[tokio::test]
    async fn agent_retrieve_requires_bearer() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/agent/retrieve-context")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"project_slug":"fjulian-me","query":"x"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = body_string(response).await;
        assert!(body.contains("agent_token_required"), "body={body}");
    }

    #[tokio::test]
    async fn agent_projects_requires_bearer() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/v1/agent/projects")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn agent_retrieve_rejects_non_qria_bearer() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/agent/retrieve-context")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer not_a_queria_token")
                    .body(Body::from(
                        r#"{"project_slug":"fjulian-me","query":"x"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

}
