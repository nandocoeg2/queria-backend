use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use chrono::{DateTime, Duration, Utc};
use queria_core::QueriaError;
use queria_core::auth::agent_token::{AgentTokenIssuer, default_agent_tools};
use queria_core::auth::permissions::{AgentTokenPermissions, AgentToolPermission};
use queria_core::ids::AgentTokenId;
use queria_db::repositories::{AgentTokenRecord, CreateAgentTokenParams};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
struct CreateAgentTokenRequest {
    name: String,
    project_slugs: Vec<String>,
    allow_global_knowledge: Option<bool>,
    tools: Option<Vec<AgentToolPermission>>,
    expires_in: Option<TokenExpiry>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
enum TokenExpiry {
    #[serde(rename = "1_day")]
    OneDay,
    #[serde(rename = "7_days")]
    SevenDays,
    #[serde(rename = "30_days")]
    ThirtyDays,
    #[serde(rename = "1_year")]
    OneYear,
    #[serde(rename = "no_expire")]
    NoExpire,
}

#[derive(Debug, Serialize)]
struct AgentTokenResponse {
    id: String,
    name: String,
    token_prefix: String,
    allow_global_knowledge: bool,
    project_slugs: Vec<String>,
    tools: Vec<AgentToolPermission>,
    expires_at: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct CreateAgentTokenResponse {
    token: String,
    agent_token: AgentTokenResponse,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/", get(list_agent_tokens).post(create_agent_token))
        .route(
            "/{agent_token_id}",
            get(get_agent_token).delete(revoke_agent_token),
        )
}

async fn list_agent_tokens(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Vec<AgentTokenResponse>> {
    let session = require_session(&state, &headers).await?;
    let repository = project_repository(&state)?;
    let tokens = repository
        .list_agent_tokens(session.user_id)
        .await
        .map_err(map_error)?;

    Ok(Json(
        tokens.into_iter().map(AgentTokenResponse::from).collect(),
    ))
}

async fn create_agent_token(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<CreateAgentTokenResponse> {
    let session = require_session(&state, &headers).await?;
    let payload: CreateAgentTokenRequest = serde_json::from_slice(&body)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid_agent_token_payload"))?;
    let expires_at = payload.expires_at()?;
    let permissions = payload.permissions()?;
    let issued = AgentTokenIssuer
        .issue(permissions.clone(), expires_at)
        .map_err(map_error)?;

    let repository = project_repository(&state)?;
    let record = repository
        .create_agent_token(
            session.user_id,
            CreateAgentTokenParams {
                name: payload.normalized_name()?,
                token_prefix: issued.token_prefix,
                token_hash: issued.token_hash,
                permissions,
                expires_at,
            },
        )
        .await
        .map_err(map_error)?;

    Ok(Json(CreateAgentTokenResponse {
        token: issued.raw_token,
        agent_token: AgentTokenResponse::from(record),
    }))
}

async fn get_agent_token(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(agent_token_id): Path<AgentTokenId>,
) -> ApiResult<AgentTokenResponse> {
    let session = require_session(&state, &headers).await?;
    let repository = project_repository(&state)?;
    let Some(token) = repository
        .get_agent_token(session.user_id, agent_token_id)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "agent_token_not_found"));
    };

    Ok(Json(AgentTokenResponse::from(token)))
}

async fn revoke_agent_token(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(agent_token_id): Path<AgentTokenId>,
) -> ApiResult<AgentTokenResponse> {
    let session = require_session(&state, &headers).await?;
    let repository = project_repository(&state)?;
    let Some(token) = repository
        .revoke_agent_token(session.user_id, agent_token_id)
        .await
        .map_err(map_error)?
    else {
        return Err(error(StatusCode::NOT_FOUND, "agent_token_not_found"));
    };

    Ok(Json(AgentTokenResponse::from(token)))
}

impl CreateAgentTokenRequest {
    fn normalized_name(&self) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
        let name = self.name.trim().to_owned();
        if name.is_empty() || name.len() > 128 {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_agent_token_name"));
        }

        Ok(name)
    }

    fn permissions(&self) -> Result<AgentTokenPermissions, (StatusCode, Json<ErrorResponse>)> {
        if self.project_slugs.is_empty() || self.project_slugs.len() > 50 {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_project_slugs"));
        }

        let mut project_slugs = Vec::with_capacity(self.project_slugs.len());
        for slug in &self.project_slugs {
            let trimmed = slug.trim().to_owned();
            if !valid_slug(&trimmed) || project_slugs.contains(&trimmed) {
                return Err(error(StatusCode::BAD_REQUEST, "invalid_project_slugs"));
            }
            project_slugs.push(trimmed);
        }

        let tools = self.tools.clone().unwrap_or_else(default_agent_tools);
        if tools.is_empty() {
            return Err(error(StatusCode::BAD_REQUEST, "invalid_agent_tools"));
        }

        Ok(AgentTokenPermissions {
            allow_global_knowledge: self.allow_global_knowledge.unwrap_or(false),
            project_slugs,
            tools,
        })
    }

    fn expires_at(&self) -> Result<Option<DateTime<Utc>>, (StatusCode, Json<ErrorResponse>)> {
        let now = Utc::now();
        match self.expires_in.unwrap_or(TokenExpiry::SevenDays) {
            TokenExpiry::OneDay => Ok(Some(now + Duration::days(1))),
            TokenExpiry::SevenDays => Ok(Some(now + Duration::days(7))),
            TokenExpiry::ThirtyDays => Ok(Some(now + Duration::days(30))),
            TokenExpiry::OneYear => Ok(Some(now + Duration::days(365))),
            TokenExpiry::NoExpire => Ok(None),
        }
    }
}

impl From<AgentTokenRecord> for AgentTokenResponse {
    fn from(value: AgentTokenRecord) -> Self {
        Self {
            id: value.id.to_string(),
            name: value.name,
            token_prefix: value.token_prefix,
            allow_global_knowledge: value.allow_global_knowledge,
            project_slugs: value.permissions.project_slugs,
            tools: value.permissions.tools,
            expires_at: value.expires_at,
            revoked_at: value.revoked_at,
            last_used_at: value.last_used_at,
            created_at: value.created_at,
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
            "token_store_not_configured",
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
            tracing::error!(error = %message, "agent token repository failed");
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
