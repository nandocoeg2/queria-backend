use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::{DateTime, Utc};
use queria_core::QueriaError;
use queria_core::ids::KnowledgeItemId;
use queria_db::repositories::KnowledgeItemRecord;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, serde::Deserialize)]
struct ListKnowledgeItemsQuery {
    scope: Option<String>,
    project_slug: Option<String>,
    category: Option<String>,
    status: Option<String>,
    tag: Option<String>,
    cursor: Option<Uuid>,
    limit: Option<u32>,
}

#[derive(Debug, Serialize)]
struct ListKnowledgeItemsResponse {
    items: Vec<KnowledgeItemResponse>,
    next_cursor: Option<String>,
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
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/", get(list_knowledge_items))
        .route("/{knowledge_item_id}", get(get_knowledge_item))
}

async fn list_knowledge_items(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListKnowledgeItemsQuery>,
) -> ApiResult<ListKnowledgeItemsResponse> {
    let session = auth::require_session(&state, &headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))?;
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    let Some(repository) = state.admin_queries_repository() else {
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "admin_queries_store_not_configured",
        ));
    };

    let limit = query.limit.unwrap_or(20);

    let items = repository
        .list_knowledge_items(
            session.user_id,
            query.scope.as_deref(),
            query.project_slug.as_deref(),
            query.category.as_deref(),
            query.status.as_deref(),
            query.tag.as_deref(),
            query.cursor,
            limit,
        )
        .await
        .map_err(map_error)?;

    let response_items: Vec<KnowledgeItemResponse> =
        items.into_iter().map(KnowledgeItemResponse::from).collect();

    let next_cursor = if response_items.len() == limit.min(100) as usize {
        response_items.last().map(|item| item.id.clone())
    } else {
        None
    };

    Ok(Json(ListKnowledgeItemsResponse {
        items: response_items,
        next_cursor,
    }))
}

async fn get_knowledge_item(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(knowledge_item_id): Path<KnowledgeItemId>,
) -> ApiResult<KnowledgeItemResponse> {
    let session = auth::require_session(&state, &headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))?;
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    let Some(repository) = state.project_repository() else {
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "knowledge_store_not_configured",
        ));
    };

    let Some(knowledge_item) = repository
        .get_knowledge_item(session.user_id, knowledge_item_id)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "knowledge_item_not_found"));
    };

    Ok(Json(KnowledgeItemResponse::from(knowledge_item)))
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

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::UNAUTHORIZED, "session_required")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "knowledge item repository failed");
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
