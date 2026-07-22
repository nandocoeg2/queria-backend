//! Thin HTTP helpers for edge healthz and MCP tools/list (no TUI, no AppConfig).

use anyhow::{Context, Result};
use serde_json::json;

const BODY_TRUNCATE: usize = 200;
const CLIENT_TIMEOUT_SECS: u64 = 30;

/// Build `{edge}/healthz`, trimming a trailing slash on the base URL.
pub fn edge_healthz_url(edge_url: &str) -> String {
    let base = edge_url.trim_end_matches('/');
    format!("{base}/healthz")
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
}
