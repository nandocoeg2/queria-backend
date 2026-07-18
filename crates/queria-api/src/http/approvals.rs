use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::{DateTime, Utc};
use queria_core::QueriaError;
use queria_core::ids::ApprovalId;
use queria_db::repositories::{ApprovalRecord, ApprovedKnowledgeRecord};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct ListApprovalsQuery {
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RejectApprovalRequest {
    reason: String,
}

#[derive(Debug, Serialize)]
struct ApprovalResponse {
    id: String,
    knowledge_item_id: String,
    project_id: Option<String>,
    source_document_id: Option<String>,
    scope: String,
    knowledge_status: String,
    title: String,
    body: String,
    category: String,
    tags: Vec<String>,
    requested_by: String,
    reviewer_user_id: Option<String>,
    approval_status: String,
    reason: Option<String>,
    created_at: DateTime<Utc>,
    decided_at: Option<DateTime<Utc>>,
    approved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct ApprovedKnowledgeResponse {
    approval: ApprovalResponse,
    chunk_id: String,
    source_document_id: String,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/", get(list_approvals))
        .route("/{approval_id}", get(get_approval))
        .route(
            "/{approval_id}/approve",
            axum::routing::post(approve_approval),
        )
        .route(
            "/{approval_id}/reject",
            axum::routing::post(reject_approval),
        )
}

async fn list_approvals(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListApprovalsQuery>,
) -> ApiResult<Vec<ApprovalResponse>> {
    let session = require_session(&state, &headers).await?;
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    let status = query.status.as_deref().map(str::trim);
    if let Some(status) = status
        && !matches!(status, "pending" | "approved" | "rejected")
    {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_approval_status"));
    }

    let repository = project_repository(&state)?;
    let approvals = repository
        .list_approvals(session.user_id, status)
        .await
        .map_err(map_error)?;

    Ok(Json(
        approvals.into_iter().map(ApprovalResponse::from).collect(),
    ))
}

async fn get_approval(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(approval_id): Path<ApprovalId>,
) -> ApiResult<ApprovalResponse> {
    let session = require_session(&state, &headers).await?;
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    let repository = project_repository(&state)?;
    let Some(approval) = repository
        .get_approval(session.user_id, approval_id)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "approval_not_found"));
    };

    Ok(Json(ApprovalResponse::from(approval)))
}

async fn approve_approval(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(approval_id): Path<ApprovalId>,
) -> ApiResult<ApprovedKnowledgeResponse> {
    let session = require_session(&state, &headers).await?;
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    let repository = project_repository(&state)?;
    let Some(approved) = repository
        .approve_approval(session.user_id, approval_id)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "approval_not_found"));
    };

    Ok(Json(ApprovedKnowledgeResponse::from(approved)))
}

async fn reject_approval(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(approval_id): Path<ApprovalId>,
    body: Bytes,
) -> ApiResult<ApprovalResponse> {
    let session = require_session(&state, &headers).await?;
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    let payload: RejectApprovalRequest = serde_json::from_slice(&body)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_reject_payload"))?;

    let repository = project_repository(&state)?;
    let Some(approval) = repository
        .reject_approval(session.user_id, approval_id, payload.reason)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "approval_not_found"));
    };

    Ok(Json(ApprovalResponse::from(approval)))
}

impl From<ApprovalRecord> for ApprovalResponse {
    fn from(value: ApprovalRecord) -> Self {
        Self {
            id: value.id.to_string(),
            knowledge_item_id: value.knowledge_item_id.to_string(),
            project_id: value.project_id.map(|id| id.to_string()),
            source_document_id: value.source_document_id.map(|id| id.to_string()),
            scope: value.scope,
            knowledge_status: value.knowledge_status,
            title: value.title,
            body: value.body,
            category: value.category,
            tags: value.tags,
            requested_by: value.requested_by,
            reviewer_user_id: value.reviewer_user_id.map(|id| id.to_string()),
            approval_status: value.approval_status,
            reason: value.reason,
            created_at: value.created_at,
            decided_at: value.decided_at,
            approved_at: value.approved_at,
        }
    }
}

impl From<ApprovedKnowledgeRecord> for ApprovedKnowledgeResponse {
    fn from(value: ApprovedKnowledgeRecord) -> Self {
        Self {
            approval: ApprovalResponse::from(value.approval),
            chunk_id: value.chunk_id.to_string(),
            source_document_id: value.source_document_id.to_string(),
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

fn project_repository(
    state: &ApiState,
) -> Result<queria_db::repositories::PgProjectRepository, (StatusCode, Json<ErrorResponse>)> {
    state.project_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "approval_store_not_configured",
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
            tracing::error!(error = %message, "approval repository failed");
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
