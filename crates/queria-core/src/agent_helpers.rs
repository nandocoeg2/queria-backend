//! Pure agent-path helpers shared by HTTP hooks and MCP tools.
//!
//! No I/O: slug shape, bearer token string parse, and HTTP retrieve limit clamp.
//! Business retrieval lives in `queria-search` (`retrieve_for_agent`).

/// HTTP agent retrieve default limit (hooks keep inject budgets small).
pub const AGENT_HTTP_RETRIEVE_LIMIT_DEFAULT: u32 = 5;

/// HTTP agent retrieve max limit (MCP still allows up to 20 via contract validate).
pub const AGENT_HTTP_RETRIEVE_LIMIT_MAX: u32 = 10;

/// MCP / shared agent default limit when the client omits `limit`.
pub const AGENT_RETRIEVE_LIMIT_DEFAULT: u32 = 5;

/// Project slug shape used by agent tokens and index/propose paths.
///
/// Rules: length 3..=64, lowercase alphanumeric + `-`, first/last alphanumeric.
#[must_use]
pub fn is_valid_project_slug(value: &str) -> bool {
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

/// Parse `Authorization` header value into a raw `qria_*` agent token.
///
/// Expects `Bearer qria_…`. Returns `None` for missing prefix, non-agent tokens,
/// or malformed values. Callers map `None` to 401 `agent_token_required`.
#[must_use]
pub fn parse_agent_bearer_token(authorization: &str) -> Option<&str> {
    authorization
        .strip_prefix("Bearer ")
        .filter(|token| token.starts_with("qria_"))
}

/// Clamp HTTP agent retrieve `limit` to `1..=AGENT_HTTP_RETRIEVE_LIMIT_MAX`.
///
/// Use after applying the default: `clamp_agent_http_retrieve_limit(limit.unwrap_or(DEFAULT))`.
#[must_use]
pub fn clamp_agent_http_retrieve_limit(limit: u32) -> u32 {
    limit.clamp(1, AGENT_HTTP_RETRIEVE_LIMIT_MAX)
}

/// Resolve optional agent flags to shared lane defaults (scratch true, needs_review false).
#[must_use]
pub fn agent_include_scratch(include_scratch: Option<bool>) -> bool {
    include_scratch.unwrap_or(true)
}

/// Resolve optional agent `include_needs_review` (default false on both transports).
#[must_use]
pub fn agent_include_needs_review(include_needs_review: Option<bool>) -> bool {
    include_needs_review.unwrap_or(false)
}

/// Resolve optional agent `include_global` (default true).
#[must_use]
pub fn agent_include_global(include_global: Option<bool>) -> bool {
    include_global.unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_project_slug_rules() {
        assert!(is_valid_project_slug("fjulian-me"));
        assert!(is_valid_project_slug("abc"));
        assert!(!is_valid_project_slug(""));
        assert!(!is_valid_project_slug("ab"));
        assert!(!is_valid_project_slug("BAD_SLUG"));
        assert!(!is_valid_project_slug("-leading"));
        assert!(!is_valid_project_slug("trailing-"));
    }

    #[test]
    fn parse_agent_bearer_accepts_qria_only() {
        assert_eq!(
            parse_agent_bearer_token("Bearer qria_abc"),
            Some("qria_abc")
        );
        assert_eq!(parse_agent_bearer_token("Bearer not_a_queria_token"), None);
        assert_eq!(parse_agent_bearer_token("qria_abc"), None);
        assert_eq!(parse_agent_bearer_token("Bearer "), None);
    }

    /// VAL-AGENT-004: HTTP limit clamp 1..=10; omitted path uses default 5 at call site.
    #[test]
    fn clamp_agent_http_retrieve_limit_bounds() {
        assert_eq!(clamp_agent_http_retrieve_limit(0), 1);
        assert_eq!(clamp_agent_http_retrieve_limit(5), 5);
        assert_eq!(clamp_agent_http_retrieve_limit(10), 10);
        assert_eq!(clamp_agent_http_retrieve_limit(20), 10);
        assert_eq!(clamp_agent_http_retrieve_limit(100), 10);
        assert_eq!(AGENT_HTTP_RETRIEVE_LIMIT_DEFAULT, 5);
        assert_eq!(AGENT_RETRIEVE_LIMIT_DEFAULT, 5);
    }

    /// VAL-AGENT-005: lane defaults scratch true / needs_review false.
    #[test]
    fn agent_lane_defaults() {
        assert!(agent_include_scratch(None));
        assert!(!agent_include_needs_review(None));
        assert!(agent_include_global(None));
        assert!(!agent_include_scratch(Some(false)));
        assert!(agent_include_needs_review(Some(true)));
        assert!(!agent_include_global(Some(false)));
    }
}
