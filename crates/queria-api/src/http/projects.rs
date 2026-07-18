use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::{DateTime, Utc};
use queria_core::QueriaError;
use queria_db::repositories::{CreateProjectParams, ProjectRecord};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct CreateProjectRequest {
    slug: String,
    name: String,
    description: Option<String>,
    default_embedding_model: Option<String>,
    include_global_default: Option<bool>,
}

#[derive(Debug, Serialize)]
struct ProjectResponse {
    id: String,
    slug: String,
    name: String,
    description: Option<String>,
    default_embedding_model: String,
    include_global_default: bool,
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
        .route("/", get(list_projects).post(create_project))
        .route("/{slug}", get(get_project))
}

async fn list_projects(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Vec<ProjectResponse>> {
    let session = require_session(&state, &headers).await?;
    // Tenant gate: super-admin without membership must not get empty global list.
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    let repository = project_repository(&state)?;
    let projects = repository
        .list_projects(session.user_id)
        .await
        .map_err(map_error)?;

    Ok(Json(
        projects.into_iter().map(ProjectResponse::from).collect(),
    ))
}

async fn create_project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<CreateProjectRequest>,
) -> ApiResult<ProjectResponse> {
    let session = require_session(&state, &headers).await?;
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    let params = payload.into_params()?;
    let repository = project_repository(&state)?;
    let project = repository
        .create_project(session.user_id, params)
        .await
        .map_err(map_error)?;

    Ok(Json(ProjectResponse::from(project)))
}

async fn get_project(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> ApiResult<ProjectResponse> {
    let session = require_session(&state, &headers).await?;
    let _home_org = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;
    if !valid_slug(&slug) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_project_slug"));
    }

    let repository = project_repository(&state)?;
    let Some(project) = repository
        .get_project_by_slug(session.user_id, &slug)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "project_not_found"));
    };

    Ok(Json(ProjectResponse::from(project)))
}

impl CreateProjectRequest {
    fn into_params(self) -> Result<CreateProjectParams, (StatusCode, Json<ErrorResponse>)> {
        let slug = self.slug.trim().to_owned();
        let name = self.name.trim().to_owned();
        let default_embedding_model = self
            .default_embedding_model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("voyage-4")
            .to_owned();

        if !valid_slug(&slug) {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_project_slug"));
        }

        if name.is_empty() || name.len() > 128 {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_project_name"));
        }

        Ok(CreateProjectParams {
            slug,
            name,
            description: self.description.map(|value| value.trim().to_owned()),
            default_embedding_model,
            include_global_default: self.include_global_default.unwrap_or(true),
        })
    }
}

impl From<ProjectRecord> for ProjectResponse {
    fn from(value: ProjectRecord) -> Self {
        Self {
            id: value.id.to_string(),
            slug: value.slug,
            name: value.name,
            description: value.description,
            default_embedding_model: value.default_embedding_model,
            include_global_default: value.include_global_default,
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
            "project_store_not_configured",
        )
    })
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

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::UNAUTHORIZED, "session_required")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "project repository failed");
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
