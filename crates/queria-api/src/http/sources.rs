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
use queria_core::ids::SourceDocumentId;
use queria_db::repositories::{RegisterSourceDocumentParams, SourceDocumentRecord};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
struct ListSourcesQuery {
    project_slug: String,
}

#[derive(Debug, Deserialize)]
struct RegisterSourceRequest {
    project_slug: String,
    kind: String,
    uri: String,
    title: String,
    source_path: Option<String>,
    branch: Option<String>,
    commit_sha: Option<String>,
    content_hash: String,
    metadata: Option<Value>,
}

#[derive(Debug, Serialize)]
struct SourceDocumentResponse {
    id: String,
    project_id: Option<String>,
    kind: String,
    uri: String,
    title: String,
    source_path: Option<String>,
    branch: Option<String>,
    commit_sha: Option<String>,
    content_hash: String,
    metadata: Value,
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
        .route("/", get(list_sources).post(register_source))
        .route("/{source_document_id}", get(get_source))
}

async fn list_sources(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Query(query): Query<ListSourcesQuery>,
) -> ApiResult<Vec<SourceDocumentResponse>> {
    let session = require_session(&state, &headers).await?;
    if !valid_slug(query.project_slug.trim()) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_project_slug"));
    }

    let repository = project_repository(&state)?;
    let sources = repository
        .list_source_documents(session.user_id, query.project_slug.trim())
        .await
        .map_err(map_error)?;

    Ok(Json(
        sources
            .into_iter()
            .map(SourceDocumentResponse::from)
            .collect(),
    ))
}

async fn register_source(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<RegisterSourceRequest>,
) -> ApiResult<SourceDocumentResponse> {
    let session = require_session(&state, &headers).await?;
    let params = payload.into_params()?;
    let repository = project_repository(&state)?;
    let source = repository
        .register_source_document(session.user_id, params)
        .await
        .map_err(map_error)?;

    Ok(Json(SourceDocumentResponse::from(source)))
}

async fn get_source(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(source_document_id): Path<SourceDocumentId>,
) -> ApiResult<SourceDocumentResponse> {
    let session = require_session(&state, &headers).await?;
    let repository = project_repository(&state)?;
    let Some(source) = repository
        .get_source_document(session.user_id, source_document_id)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "source_document_not_found"));
    };

    Ok(Json(SourceDocumentResponse::from(source)))
}

impl RegisterSourceRequest {
    fn into_params(
        self,
    ) -> Result<RegisterSourceDocumentParams, (StatusCode, Json<ErrorResponse>)> {
        let project_slug = self.project_slug.trim().to_owned();
        let kind = self.kind.trim().to_owned();
        let uri = self.uri.trim().to_owned();
        let title = self.title.trim().to_owned();
        let content_hash = self.content_hash.trim().to_owned();

        if !valid_slug(&project_slug) {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_project_slug"));
        }

        if !valid_source_kind(&kind) {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_source_kind"));
        }

        if uri.is_empty() || uri.len() > 2048 {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_source_uri"));
        }

        if title.is_empty() || title.len() > 256 {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_source_title"));
        }

        if content_hash.is_empty() || content_hash.len() > 256 {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_content_hash"));
        }

        Ok(RegisterSourceDocumentParams {
            project_slug,
            kind,
            uri,
            title,
            source_path: normalize_optional(self.source_path),
            branch: normalize_optional(self.branch),
            commit_sha: normalize_optional(self.commit_sha),
            content_hash,
            metadata: self
                .metadata
                .unwrap_or_else(|| Value::Object(Default::default())),
        })
    }
}

impl From<SourceDocumentRecord> for SourceDocumentResponse {
    fn from(value: SourceDocumentRecord) -> Self {
        Self {
            id: value.id.to_string(),
            project_id: value.project_id.map(|id| id.to_string()),
            kind: value.kind,
            uri: value.uri,
            title: value.title,
            source_path: value.source_path,
            branch: value.branch,
            commit_sha: value.commit_sha,
            content_hash: value.content_hash,
            metadata: value.metadata,
            created_at: value.created_at,
            updated_at: value.updated_at,
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
            "source_store_not_configured",
        )
    })
}

fn valid_source_kind(value: &str) -> bool {
    matches!(
        value,
        "git_repo" | "markdown_docs" | "manual_note" | "incident_report" | "sop" | "config"
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

fn normalize_optional(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_owned())
        .filter(|trimmed| !trimmed.is_empty())
}

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::UNAUTHORIZED, "session_required")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "source repository failed");
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
