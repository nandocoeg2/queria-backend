//! Admin session routes for the needs_review queue (IMP-L4 / Task 6).
//!
//! - GET  /api/v1/needs-review
//! - POST /api/v1/needs-review/{id}/promote
//! - POST /api/v1/needs-review/{id}/reject
//! - POST /api/v1/needs-review/promote-bulk
//! - POST /api/v1/needs-review/reject-bulk

use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use queria_core::QueriaError;
use queria_core::ids::KnowledgeItemId;
use queria_db::repositories::{
    KnowledgeItemRecord, NeedsReviewActionRecord, NeedsReviewItemRecord,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct ListNeedsReviewQuery {
    project_slug: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct BulkNeedsReviewRequest {
    project_slug: String,
    #[serde(default)]
    origin_url: Option<String>,
    #[serde(default)]
    commit_sha: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    /// Allow bulk when both origin_url and commit_sha are empty (project-wide). Default false.
    #[serde(default)]
    force_project_all: bool,
}

#[derive(Debug, Deserialize)]
struct RejectNeedsReviewRequest {
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct NeedsReviewItemResponse {
    knowledge_item_id: String,
    project_id: Option<String>,
    project_slug: Option<String>,
    source_document_id: Option<String>,
    title: String,
    path: Option<String>,
    origin_url: Option<String>,
    commit_sha: Option<String>,
    branch: Option<String>,
    category: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ListNeedsReviewResponse {
    items: Vec<NeedsReviewItemResponse>,
}

#[derive(Debug, Serialize)]
struct KnowledgeItemResponse {
    id: String,
    project_id: Option<String>,
    source_document_id: Option<String>,
    scope: String,
    status: String,
    title: String,
    body: String,
    category: String,
    tags: Vec<String>,
    approved_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct NeedsReviewActionResponse {
    knowledge_item: KnowledgeItemResponse,
    chunk_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BulkNeedsReviewResponse {
    count: usize,
    items: Vec<NeedsReviewActionResponse>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/", get(list_needs_review))
        .route("/promote-bulk", post(promote_bulk))
        .route("/reject-bulk", post(reject_bulk))
        .route("/{knowledge_item_id}/promote", post(promote_one))
        .route("/{knowledge_item_id}/reject", post(reject_one))
}

async fn list_needs_review(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListNeedsReviewQuery>,
) -> ApiResult<ListNeedsReviewResponse> {
    let session = require_session(&state, &headers).await?;
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    let repository = project_repository(&state)?;

    let items = repository
        .list_needs_review(
            session.user_id,
            query.project_slug.as_deref(),
            query.limit.unwrap_or(100),
        )
        .await
        .map_err(map_error)?;

    Ok(Json(ListNeedsReviewResponse {
        items: items
            .into_iter()
            .map(NeedsReviewItemResponse::from)
            .collect(),
    }))
}

async fn promote_one(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(knowledge_item_id): Path<KnowledgeItemId>,
) -> ApiResult<NeedsReviewActionResponse> {
    let session = require_session(&state, &headers).await?;
    let home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    require_org_admin_or_super(&state, &session, home_org).await?;
    let repository = project_repository(&state)?;

    let Some(record) = repository
        .promote_needs_review(session.user_id, knowledge_item_id)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "knowledge_item_not_found"));
    };

    Ok(Json(NeedsReviewActionResponse::from(record)))
}

async fn reject_one(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(knowledge_item_id): Path<KnowledgeItemId>,
    body: Bytes,
) -> ApiResult<NeedsReviewActionResponse> {
    let session = require_session(&state, &headers).await?;
    let home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    require_org_admin_or_super(&state, &session, home_org).await?;

    let reason = if body.is_empty() {
        None
    } else {
        let payload: RejectNeedsReviewRequest = serde_json::from_slice(&body)
            .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_reject_payload"))?;
        payload.reason
    };

    let repository = project_repository(&state)?;
    let Some(record) = repository
        .reject_needs_review(session.user_id, knowledge_item_id, reason)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "knowledge_item_not_found"));
    };

    Ok(Json(NeedsReviewActionResponse::from(record)))
}

async fn promote_bulk(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<BulkNeedsReviewResponse> {
    let session = require_session(&state, &headers).await?;
    let home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    require_org_admin_or_super(&state, &session, home_org).await?;
    let payload: BulkNeedsReviewRequest = serde_json::from_slice(&body)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_bulk_payload"))?;

    let project_slug = payload.project_slug.trim();
    if project_slug.is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "project_slug_required"));
    }

    let repository = project_repository(&state)?;
    let records = repository
        .promote_needs_review_by_origin_commit(
            session.user_id,
            project_slug,
            payload.origin_url.as_deref(),
            payload.commit_sha.as_deref(),
            payload.force_project_all,
        )
        .await
        .map_err(map_error)?;

    let items: Vec<NeedsReviewActionResponse> = records
        .into_iter()
        .map(NeedsReviewActionResponse::from)
        .collect();
    Ok(Json(BulkNeedsReviewResponse {
        count: items.len(),
        items,
    }))
}

async fn reject_bulk(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<BulkNeedsReviewResponse> {
    let session = require_session(&state, &headers).await?;
    let home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    require_org_admin_or_super(&state, &session, home_org).await?;
    let payload: BulkNeedsReviewRequest = serde_json::from_slice(&body)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_bulk_payload"))?;

    let project_slug = payload.project_slug.trim();
    if project_slug.is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "project_slug_required"));
    }

    let repository = project_repository(&state)?;
    let records = repository
        .reject_needs_review_by_origin_commit(
            session.user_id,
            project_slug,
            payload.origin_url.as_deref(),
            payload.commit_sha.as_deref(),
            payload.reason,
            payload.force_project_all,
        )
        .await
        .map_err(map_error)?;

    let items: Vec<NeedsReviewActionResponse> = records
        .into_iter()
        .map(NeedsReviewActionResponse::from)
        .collect();
    Ok(Json(BulkNeedsReviewResponse {
        count: items.len(),
        items,
    }))
}

impl From<NeedsReviewItemRecord> for NeedsReviewItemResponse {
    fn from(value: NeedsReviewItemRecord) -> Self {
        Self {
            knowledge_item_id: value.knowledge_item_id.to_string(),
            project_id: value.project_id.map(|id| id.to_string()),
            project_slug: value.project_slug,
            source_document_id: value.source_document_id.map(|id| id.to_string()),
            title: value.title,
            path: value.path,
            origin_url: value.origin_url,
            commit_sha: value.commit_sha,
            branch: value.branch,
            category: value.category,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<KnowledgeItemRecord> for KnowledgeItemResponse {
    fn from(value: KnowledgeItemRecord) -> Self {
        Self {
            id: value.id.to_string(),
            project_id: value.project_id.map(|id| id.to_string()),
            source_document_id: value.source_document_id.map(|id| id.to_string()),
            scope: value.scope,
            status: value.status,
            title: value.title,
            body: value.body,
            category: value.category,
            tags: value.tags,
            approved_at: value.approved_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl From<NeedsReviewActionRecord> for NeedsReviewActionResponse {
    fn from(value: NeedsReviewActionRecord) -> Self {
        Self {
            knowledge_item: KnowledgeItemResponse::from(value.knowledge_item),
            chunk_ids: value.chunk_ids.into_iter().map(|id| id.to_string()).collect(),
        }
    }
}

async fn require_session(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<queria_db::repositories::AuthenticatedSession, (StatusCode, Json<ErrorResponse>)> {
    auth::require_session(state, headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))
}

/// Pure gate: session promote/reject requires org_admin role OR platform super-admin.
#[cfg_attr(test, allow(dead_code))]
fn session_may_manage_needs_review(is_platform_super_admin: bool, membership_role: Option<&str>) -> bool {
    is_platform_super_admin || membership_role == Some("org_admin")
}

/// Session promote/reject (not agent MCP): org_admin of home org OR platform super-admin.
async fn require_org_admin_or_super(
    state: &ApiState,
    session: &queria_db::repositories::AuthenticatedSession,
    organization_id: uuid::Uuid,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if session.is_platform_super_admin {
        return Ok(());
    }
    let Some(org_repo) = state.org_repository() else {
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "needs_review_store_not_configured",
        ));
    };
    let role = org_repo
        .membership_role(session.user_id, organization_id)
        .await
        .map_err(map_error)?;
    if !session_may_manage_needs_review(false, role.as_deref()) {
        return Err(error(StatusCode::FORBIDDEN, "org_admin_required"));
    }
    Ok(())
}

fn project_repository(
    state: &ApiState,
) -> Result<queria_db::repositories::PgProjectRepository, (StatusCode, Json<ErrorResponse>)> {
    state.project_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "needs_review_store_not_configured",
        )
    })
}

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::UNAUTHORIZED, "session_required")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "needs_review repository failed");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::build_app;
    use axum::body::Body;
    use http::Request;
    use queria_core::AppConfig;
    use tower::ServiceExt;

    async fn body_string(response: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }

    #[tokio::test]
    async fn list_requires_session() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/needs-review")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn promote_requires_session() {
        let app = build_app(AppConfig::default_local());
        let id = "11111111-1111-1111-1111-111111111111";
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/needs-review/{id}/promote"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn reject_requires_session() {
        let app = build_app(AppConfig::default_local());
        let id = "11111111-1111-1111-1111-111111111111";
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/needs-review/{id}/reject"))
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"reason":"nope"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn promote_bulk_requires_session() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/needs-review/promote-bulk")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"project_slug":"api","origin_url":"git@h:a.git","commit_sha":"abc"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = body_string(response).await;
        assert!(body.contains("session_required") || body.contains("error"), "body={body}");
    }

    #[test]
    fn bulk_request_deserializes() {
        let payload: BulkNeedsReviewRequest = serde_json::from_str(
            r#"{"project_slug":"api","origin_url":"git@h:a.git","commit_sha":"abc"}"#,
        )
        .expect("deserialize");
        assert_eq!(payload.project_slug, "api");
        assert_eq!(payload.origin_url.as_deref(), Some("git@h:a.git"));
        assert_eq!(payload.commit_sha.as_deref(), Some("abc"));
        assert!(!payload.force_project_all);
    }

    #[test]
    fn bulk_request_force_project_all_defaults_false() {
        let payload: BulkNeedsReviewRequest =
            serde_json::from_str(r#"{"project_slug":"api"}"#).expect("deserialize");
        assert!(!payload.force_project_all);
        assert!(payload.origin_url.is_none());
        assert!(payload.commit_sha.is_none());
    }

    #[test]
    fn bulk_request_force_project_all_true() {
        let payload: BulkNeedsReviewRequest = serde_json::from_str(
            r#"{"project_slug":"api","force_project_all":true}"#,
        )
        .expect("deserialize");
        assert!(payload.force_project_all);
    }

    /// Promote/reject session gate: org_admin or platform super-admin only (not org_member).
    #[test]
    fn promote_requires_org_admin_or_super() {
        assert!(!session_may_manage_needs_review(false, None));
        assert!(!session_may_manage_needs_review(false, Some("org_member")));
        assert!(session_may_manage_needs_review(false, Some("org_admin")));
        assert!(session_may_manage_needs_review(true, None));
        assert!(session_may_manage_needs_review(true, Some("org_member")));
    }

    #[test]
    fn item_response_shape() {
        let item = NeedsReviewItemResponse {
            knowledge_item_id: "k1".into(),
            project_id: Some("p1".into()),
            project_slug: Some("api".into()),
            source_document_id: None,
            title: "src/main.ts".into(),
            path: Some("src/main.ts".into()),
            origin_url: Some("git@h:a.git".into()),
            commit_sha: Some("abc".into()),
            branch: Some("main".into()),
            category: "local_git".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let v = serde_json::to_value(&item).expect("ser");
        assert_eq!(v["project_slug"], "api");
        assert_eq!(v["commit_sha"], "abc");
        assert!(v.get("knowledge_item_id").is_some());
    }
}
