//! Thin HTTP helpers for edge healthz, MCP tools/list, and agent projects-status
//! (no TUI, no AppConfig).

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::json;

const BODY_TRUNCATE: usize = 200;
const CLIENT_TIMEOUT_SECS: u64 = 30;

/// Laptop status payload from `GET /api/v1/agent/projects-status`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProjectsStatusResponse {
    pub embedding_profile_version: String,
    pub permissions: Vec<String>,
    pub projects: Vec<ProjectStatusRow>,
}

/// One project row in the projects-status response.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ProjectStatusRow {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub embed: EmbedCounts,
    pub needs_review_count: i64,
}

/// Folded embed counters (processing/stale already merged into pending on server).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct EmbedCounts {
    pub ready: i64,
    pub pending: i64,
    pub failed: i64,
}

/// Build `{edge}/healthz`, trimming a trailing slash on the base URL.
pub fn edge_healthz_url(edge_url: &str) -> String {
    let base = edge_url.trim_end_matches('/');
    format!("{base}/healthz")
}

/// Build `{edge}/api/v1/agent/projects-status`.
pub fn projects_status_url(edge_url: &str) -> String {
    let base = edge_url.trim_end_matches('/');
    format!("{base}/api/v1/agent/projects-status")
}

fn truncate_body(body: String) -> String {
    if body.len() <= BODY_TRUNCATE {
        body
    } else {
        body.chars().take(BODY_TRUNCATE).collect()
    }
}

fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(CLIENT_TIMEOUT_SECS))
        .build()
        .context("failed to build HTTP client")
}

/// GET `{edge}/healthz`. Returns status code and body text (truncated to 200 chars).
pub async fn edge_health(edge_url: &str) -> Result<(u16, String)> {
    let url = edge_healthz_url(edge_url);
    let response = client()?
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to connect to edge healthz {url}"))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .context("failed to read edge healthz body")?;
    Ok((status, truncate_body(body)))
}

/// POST JSON-RPC `tools/list` to MCP URL with Bearer token.
/// Returns status + body without requiring HTTP success (same body shape as doctor_mcp).
pub async fn mcp_tools_list(mcp_url: &str, token: &str) -> Result<(u16, String)> {
    let response = client()?
        .post(mcp_url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        }))
        .send()
        .await
        .with_context(|| format!("failed to connect to MCP endpoint {mcp_url}"))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .context("failed to read MCP tools/list body")?;
    Ok((status, body))
}

/// GET `{edge}/api/v1/agent/projects-status` with Bearer agent token.
///
/// On HTTP 200: returns `(200, parsed)`.
/// On other status: `Err` whose message starts with `projects-status HTTP {status}:`
/// so callers (Status TUI, Index preflight) can detect 404 redeploy/degrade paths.
pub async fn fetch_projects_status(
    edge_url: &str,
    token: &str,
) -> Result<(u16, ProjectsStatusResponse)> {
    let url = projects_status_url(edge_url);
    let response = client()?
        .get(&url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .with_context(|| format!("failed to connect to projects-status {url}"))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .context("failed to read projects-status body")?;
    if status != 200 {
        bail!(
            "projects-status HTTP {status}: {}",
            truncate_body(body)
        );
    }
    let parsed: ProjectsStatusResponse =
        serde_json::from_str(&body).with_context(|| {
            format!(
                "failed to parse projects-status body: {}",
                truncate_body(body.clone())
            )
        })?;
    Ok((status, parsed))
}

/// True when `err` is the non-200 path for HTTP 404 (old edge missing the route).
pub fn is_projects_status_404(err: &anyhow::Error) -> bool {
    err.to_string().contains("projects-status HTTP 404")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthz_url_trims_slash() {
        assert_eq!(
            edge_healthz_url("https://queria.fjulian.id/"),
            "https://queria.fjulian.id/healthz"
        );
    }

    #[test]
    fn healthz_url_without_trailing_slash() {
        assert_eq!(
            edge_healthz_url("https://queria.fjulian.id"),
            "https://queria.fjulian.id/healthz"
        );
    }

    #[test]
    fn projects_status_url_trims_slash() {
        assert_eq!(
            projects_status_url("https://queria.fjulian.id/"),
            "https://queria.fjulian.id/api/v1/agent/projects-status"
        );
    }

    #[test]
    fn deserialize_projects_status_sample_json() {
        let sample = r#"{
          "embedding_profile_version": "voyage-4-1024-v1",
          "permissions": ["index_local", "list_projects", "retrieve_context"],
          "projects": [
            {
              "id": "11111111-1111-1111-1111-111111111111",
              "slug": "queria-backend",
              "name": "QuerIa Backend",
              "embed": { "ready": 80, "pending": 3, "failed": 0 },
              "needs_review_count": 12
            }
          ]
        }"#;
        let parsed: ProjectsStatusResponse = serde_json::from_str(sample).expect("parse sample");
        assert_eq!(parsed.embedding_profile_version, "voyage-4-1024-v1");
        assert!(parsed.permissions.iter().any(|p| p == "index_local"));
        assert_eq!(parsed.projects.len(), 1);
        assert_eq!(parsed.projects[0].slug, "queria-backend");
        assert_eq!(parsed.projects[0].embed.ready, 80);
        assert_eq!(parsed.projects[0].embed.pending, 3);
        assert_eq!(parsed.projects[0].embed.failed, 0);
        assert_eq!(parsed.projects[0].needs_review_count, 12);
    }

    #[test]
    fn is_404_detects_message() {
        let err = anyhow::anyhow!("projects-status HTTP 404: not found");
        assert!(is_projects_status_404(&err));
        let other = anyhow::anyhow!("projects-status HTTP 403: permission_denied");
        assert!(!is_projects_status_404(&other));
    }
}
