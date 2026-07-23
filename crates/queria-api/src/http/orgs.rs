//! Organization, invite, and membership HTTP surfaces (multi-org isolation MVP).
//!
//! Routes:
//! - POST/GET  /api/v1/orgs                      — platform super-admin
//! - POST      /api/v1/orgs/{slug}/invites        — org_admin of slug or super-admin
//! - GET       /api/v1/orgs/current/members      — active org
//! - POST      /api/v1/invites/accept            — public

use crate::app::ApiState;
use crate::http::auth;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use queria_core::QueriaError;
use queria_core::auth::org_invite::OrgInviteTokenIssuer;
use queria_core::auth::password::PasswordHasher;
use queria_db::repositories::{
    AcceptOrgInviteParams, AuthenticatedSession, CreateOrgInviteParams, CreateOrganizationParams,
    OrgInviteRecord, OrgMemberRecord, OrganizationRecord, PgOrgRepository,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const INVITE_TTL_DAYS: i64 = 7;

#[derive(Debug, Deserialize)]
struct CreateOrgRequest {
    slug: String,
    name: String,
    first_admin_email: String,
}

#[derive(Debug, Deserialize)]
struct CreateInviteRequest {
    email: String,
    role: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AcceptInviteRequest {
    token: String,
    password: String,
    /// Optional display name for new users (ignored by v1 storage; accepted for forward compat).
    name: Option<String>,
}

#[derive(Debug, Serialize)]
struct OrganizationResponse {
    id: String,
    slug: String,
    name: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct CreateOrgResponse {
    organization: OrganizationResponse,
    /// One-time raw invite token; never stored, never re-fetched.
    invite_token: String,
    invite: InviteMetaResponse,
}

#[derive(Debug, Serialize)]
struct CreateInviteResponse {
    invite_token: String,
    invite: InviteMetaResponse,
}

#[derive(Debug, Serialize)]
struct InviteMetaResponse {
    id: String,
    email: String,
    role: String,
    token_prefix: String,
    expires_at: String,
}

#[derive(Debug, Serialize)]
struct MemberResponse {
    user_id: String,
    email: String,
    role: String,
    created_at: String,
}

#[derive(Debug, Serialize)]
struct AcceptInviteResponse {
    accepted: bool,
    user_id: Option<String>,
    email: Option<String>,
    organization_id: Option<String>,
    organization_slug: Option<String>,
    role: Option<String>,
    created_user: Option<bool>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/api/v1/orgs", get(list_orgs).post(create_org))
        .route("/api/v1/orgs/{slug}/invites", post(create_invite))
        .route("/api/v1/orgs/current/members", get(list_current_members))
        .route("/api/v1/invites/accept", post(accept_invite))
}

async fn list_orgs(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Vec<OrganizationResponse>> {
    let session = require_session(&state, &headers).await?;
    auth::require_platform_super_admin(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;

    let repository = org_repository(&state)?;
    let orgs = repository.list_organizations().await.map_err(map_error)?;
    Ok(Json(
        orgs.into_iter().map(OrganizationResponse::from).collect(),
    ))
}

async fn create_org(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<CreateOrgRequest>,
) -> ApiResult<CreateOrgResponse> {
    let session = require_session(&state, &headers).await?;
    auth::require_platform_super_admin(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;

    let slug = payload.slug.trim().to_owned();
    let name = payload.name.trim().to_owned();
    let first_admin_email = normalize_email(&payload.first_admin_email);

    if !valid_org_slug(&slug) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_organization_slug"));
    }
    if name.is_empty() || name.len() > 128 {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_organization_name"));
    }
    if !valid_email(&first_admin_email) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_first_admin_email"));
    }

    let issued = OrgInviteTokenIssuer.issue().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invite_token_issue_failed",
        )
    })?;
    let expires_at = Utc::now() + Duration::days(INVITE_TTL_DAYS);

    let repository = org_repository(&state)?;
    let (organization, invite) = repository
        .create_organization_with_invite(CreateOrganizationParams {
            slug,
            name,
            first_admin_email,
            token_prefix: issued.token_prefix,
            token_hash: issued.token_hash,
            expires_at,
            invited_by_user_id: session.user_id,
        })
        .await
        .map_err(map_create_org_error)?;

    // Never log raw token at info — optional accept path only.
    tracing::info!(
        organization_slug = %organization.slug,
        email = %invite.email,
        invite_id = %invite.id,
        accept_path = "/admin/invites/accept",
        "org invite created"
    );

    Ok(Json(CreateOrgResponse {
        organization: OrganizationResponse::from(organization),
        invite_token: issued.raw_token,
        invite: InviteMetaResponse::from(invite),
    }))
}

async fn create_invite(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    Json(payload): Json<CreateInviteRequest>,
) -> ApiResult<CreateInviteResponse> {
    let session = require_session(&state, &headers).await?;
    let repository = org_repository(&state)?;

    let org = repository
        .get_organization_by_slug(slug.trim())
        .await
        .map_err(map_error)?
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "organization_not_found"))?;

    authorize_invite_create(&session, &repository, org.id).await?;

    let email = normalize_email(&payload.email);
    if !valid_email(&email) {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_invite_email"));
    }

    let role = payload
        .role
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("org_member")
        .to_owned();
    if role != "org_admin" && role != "org_member" {
        return Err(error(StatusCode::BAD_REQUEST, "invalid_invite_role"));
    }

    let issued = OrgInviteTokenIssuer.issue().map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invite_token_issue_failed",
        )
    })?;
    let expires_at = Utc::now() + Duration::days(INVITE_TTL_DAYS);

    let invite = repository
        .create_invite(CreateOrgInviteParams {
            organization_id: org.id,
            email,
            role,
            token_prefix: issued.token_prefix,
            token_hash: issued.token_hash,
            expires_at,
            invited_by_user_id: session.user_id,
        })
        .await
        .map_err(map_error)?;

    tracing::info!(
        organization_slug = %org.slug,
        email = %invite.email,
        invite_id = %invite.id,
        accept_path = "/admin/invites/accept",
        "org invite created"
    );

    Ok(Json(CreateInviteResponse {
        invite_token: issued.raw_token,
        invite: InviteMetaResponse::from(invite),
    }))
}

async fn list_current_members(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> ApiResult<Vec<MemberResponse>> {
    let session = require_session(&state, &headers).await?;
    let home = auth::require_active_org(&session)
        .map_err(|message| error(StatusCode::FORBIDDEN, message))?;

    let repository = org_repository(&state)?;
    let members = repository.list_members(home).await.map_err(map_error)?;
    Ok(Json(
        members.into_iter().map(MemberResponse::from).collect(),
    ))
}

async fn accept_invite(
    State(state): State<ApiState>,
    Json(payload): Json<AcceptInviteRequest>,
) -> (StatusCode, Json<AcceptInviteResponse>) {
    let Some(repository) = state.org_repository() else {
        return accept_error(StatusCode::SERVICE_UNAVAILABLE, "database_not_configured");
    };

    let token = payload.token.trim();
    if token.is_empty() {
        return accept_error(StatusCode::BAD_REQUEST, "invalid_invite_token");
    }

    // Product password rule matches setup: non-empty and >= 12 (PasswordHasher).
    if payload.password.is_empty() {
        return accept_error(StatusCode::BAD_REQUEST, "empty_password");
    }

    let password_hash = match PasswordHasher.hash_password(&payload.password) {
        Ok(hash) => hash,
        Err(QueriaError::Validation(_)) => {
            return accept_error(StatusCode::BAD_REQUEST, "weak_password");
        }
        Err(_) => return accept_error(StatusCode::INTERNAL_SERVER_ERROR, "password_hash_failed"),
    };

    let token_hash = OrgInviteTokenIssuer::hash_token(token);
    // Keep optional name from body used only so empty struct fields do not warn.
    let _name = payload.name;

    match repository
        .accept_invite(AcceptOrgInviteParams {
            token_hash,
            password_hash,
        })
        .await
    {
        Ok(accepted) => (
            StatusCode::OK,
            Json(AcceptInviteResponse {
                accepted: true,
                user_id: Some(accepted.user_id.to_string()),
                email: Some(accepted.email),
                organization_id: Some(accepted.organization_id.to_string()),
                organization_slug: Some(accepted.organization_slug),
                role: Some(accepted.role),
                created_user: Some(accepted.created_user),
                error: None,
            }),
        ),
        Err(QueriaError::Validation(message)) => {
            let status = match message.as_str() {
                "invite_invalid" | "invite_revoked" => StatusCode::BAD_REQUEST,
                "invite_already_used" => StatusCode::CONFLICT,
                "invite_expired" => StatusCode::GONE,
                "already_member_of_other_org" => StatusCode::CONFLICT,
                _ => StatusCode::BAD_REQUEST,
            };
            accept_error(status, &message)
        }
        Err(_) => accept_error(StatusCode::INTERNAL_SERVER_ERROR, "accept_failed"),
    }
}

async fn authorize_invite_create(
    session: &AuthenticatedSession,
    repository: &PgOrgRepository,
    organization_id: Uuid,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if session.is_platform_super_admin {
        return Ok(());
    }

    let Some(role) = repository
        .membership_role(session.user_id, organization_id)
        .await
        .map_err(map_error)?
    else {
        // Hide existence for foreign orgs when caller is not super-admin.
        return Err(error(StatusCode::FORBIDDEN, "invite_create_forbidden"));
    };

    // v1: humans with membership are org_admin-powered; still require org_admin role string
    // for invite create (org_member may exist later with different powers).
    if role != "org_admin" {
        return Err(error(StatusCode::FORBIDDEN, "invite_create_forbidden"));
    }

    Ok(())
}

fn normalize_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn valid_email(email: &str) -> bool {
    let at = match email.find('@') {
        Some(idx) if idx > 0 && idx < email.len() - 1 => idx,
        _ => return false,
    };
    email[at + 1..].contains('.')
}

fn valid_org_slug(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    let Some(last) = bytes.last() else {
        return false;
    };

    // Matches organization.slug check: ^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$
    (3..=64).contains(&bytes.len())
        && first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

async fn require_session(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<AuthenticatedSession, (StatusCode, Json<ErrorResponse>)> {
    auth::require_session(state, headers)
        .await
        .map_err(|message| error(StatusCode::UNAUTHORIZED, message))
}

fn org_repository(state: &ApiState) -> Result<PgOrgRepository, (StatusCode, Json<ErrorResponse>)> {
    state.org_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "org_store_not_configured",
        )
    })
}

fn map_create_org_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) if message == "organization_slug_exists" => {
            error(StatusCode::CONFLICT, "organization_slug_exists")
        }
        other => map_error(other),
    }
}

fn map_error(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication | QueriaError::PermissionDenied => {
            error(StatusCode::FORBIDDEN, "permission_denied")
        }
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "org repository failed");
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

fn accept_error(status: StatusCode, message: &str) -> (StatusCode, Json<AcceptInviteResponse>) {
    (
        status,
        Json(AcceptInviteResponse {
            accepted: false,
            user_id: None,
            email: None,
            organization_id: None,
            organization_slug: None,
            role: None,
            created_user: None,
            error: Some(message.to_owned()),
        }),
    )
}

impl From<OrganizationRecord> for OrganizationResponse {
    fn from(value: OrganizationRecord) -> Self {
        Self {
            id: value.id.to_string(),
            slug: value.slug,
            name: value.name,
            created_at: value.created_at.to_rfc3339(),
        }
    }
}

impl From<OrgInviteRecord> for InviteMetaResponse {
    fn from(value: OrgInviteRecord) -> Self {
        Self {
            id: value.id.to_string(),
            email: value.email,
            role: value.role,
            token_prefix: value.token_prefix,
            expires_at: value.expires_at.to_rfc3339(),
        }
    }
}

impl From<OrgMemberRecord> for MemberResponse {
    fn from(value: OrgMemberRecord) -> Self {
        Self {
            user_id: value.user_id.to_string(),
            email: value.email,
            role: value.role,
            created_at: value.created_at.to_rfc3339(),
        }
    }
}

#[cfg(test)]
#[path = "orgs_tests.rs"]
mod tests;
