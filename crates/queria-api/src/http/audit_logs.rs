use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::{DateTime, Utc};
use queria_core::QueriaError;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
struct ListAuditLogsQuery {
    actor_id: Option<String>,
    action: Option<String>,
    resource_type: Option<String>,
    resource_id: Option<String>,
    cursor: Option<Uuid>,
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct AuditLogResponse {
    id: String,
    actor_type: String,
    actor_id: Option<String>,
    action: String,
    resource_type: String,
    resource_id: Option<String>,
    ip_hash: Option<String>,
    user_agent_hash: Option<String>,
    metadata: serde_json::Value,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ListAuditLogsResponse {
    items: Vec<AuditLogResponse>,
    next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new().route("/", get(list_audit_logs))
}

async fn list_audit_logs(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListAuditLogsQuery>,
) -> ApiResult<ListAuditLogsResponse> {
    let session = auth::require_session(&state, &headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))?;
    let Some(repository) = state.admin_queries_repository() else {
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "admin_queries_store_not_configured",
        ));
    };

    let limit = query.limit.unwrap_or(20);

    let items = repository
        .list_audit_logs(
            session.user_id,
            query.actor_id.as_deref(),
            query.action.as_deref(),
            query.resource_type.as_deref(),
            query.resource_id.as_deref(),
            query.cursor,
            limit,
        )
        .await
        .map_err(map_error)?;

    let response_items: Vec<AuditLogResponse> =
        items.into_iter().map(AuditLogResponse::from).collect();

    let next_cursor = if response_items.len() == limit.min(100) as usize {
        response_items.last().map(|item| item.id.clone())
    } else {
        None
    };

    Ok(Json(ListAuditLogsResponse {
        items: response_items,
        next_cursor,
    }))
}

impl From<queria_db::admin_queries::AuditLogRecord> for AuditLogResponse {
    fn from(value: queria_db::admin_queries::AuditLogRecord) -> Self {
        Self {
            id: value.id.to_string(),
            actor_type: value.actor_type,
            actor_id: value.actor_id,
            action: value.action,
            resource_type: value.resource_type,
            resource_id: value.resource_id,
            ip_hash: value.ip_hash,
            user_agent_hash: value.user_agent_hash,
            metadata: value.metadata,
            created_at: value.created_at,
        }
    }
}

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::UNAUTHORIZED, "session_required")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "audit log repository failed");
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
