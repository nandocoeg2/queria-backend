//! Shared agent bearer auth for HTTP agent routes (retrieve, list, index-local).
//!
//! Thin glue only: parse `qria_*` Bearer, hash + DB authenticate. Callers keep
//! their own `ErrorResponse` shape and permission checks.

use axum::http::{HeaderMap, StatusCode, header};
use queria_core::auth::agent_token::AgentTokenIssuer;
use queria_core::parse_agent_bearer_token;
use queria_db::repositories::{AuthenticatedAgentToken, PgProjectRepository};

/// `(status, stable error code)` for mapping into each handler's JSON error type.
pub type AgentAuthError = (StatusCode, &'static str);

/// Extract raw `qria_*` token from `Authorization: Bearer …`, or 401.
pub fn require_raw_bearer(headers: &HeaderMap) -> Result<&str, AgentAuthError> {
    bearer_token(headers).ok_or((StatusCode::UNAUTHORIZED, "agent_token_required"))
}

/// Authenticate a raw agent token against the project repository.
pub async fn authenticate_raw(
    repository: &PgProjectRepository,
    raw: &str,
) -> Result<AuthenticatedAgentToken, AgentAuthError> {
    let token_hash = AgentTokenIssuer::hash_token(raw);
    repository
        .authenticate_agent_token(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "agent token authentication failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "repository_failed")
        })?
        .ok_or((StatusCode::UNAUTHORIZED, "agent_token_required"))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_agent_bearer_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn require_raw_bearer_accepts_qria_only() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer qria_test_token"),
        );
        assert_eq!(require_raw_bearer(&headers).ok(), Some("qria_test_token"));
    }

    #[test]
    fn require_raw_bearer_rejects_missing_and_non_qria() {
        let empty = HeaderMap::new();
        assert_eq!(
            require_raw_bearer(&empty).unwrap_err(),
            (StatusCode::UNAUTHORIZED, "agent_token_required")
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer not_a_queria_token"),
        );
        assert_eq!(
            require_raw_bearer(&headers).unwrap_err(),
            (StatusCode::UNAUTHORIZED, "agent_token_required")
        );
    }
}
