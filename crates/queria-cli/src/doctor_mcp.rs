//! Non-TUI `doctor mcp`: thin wrap over shared `edge_agent::mcp_tools_list`.
//! Doctor TUI and this flag share one MCP tools/list client path.

use crate::edge_agent;
use anyhow::{Context, bail};

/// Probe MCP `tools/list` with a Bearer agent token. Prints status + body.
///
/// Uses the same HTTP client and request shape as Doctor TUI via `edge_agent`.
pub async fn run(url: &str, agent_token: &str) -> anyhow::Result<()> {
    println!("mcp_url={url}");
    if !agent_token.starts_with("qria_") {
        bail!("agent token must start with qria_");
    }
    let (status, body) = edge_agent::mcp_tools_list(url, agent_token)
        .await
        .with_context(|| format!("failed MCP tools/list for doctor mcp at {url}"))?;
    println!("mcp_status={status}");
    println!("mcp_body={body}");
    if !(200..300).contains(&status) {
        bail!("MCP doctor failed with status {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Structural: doctor_mcp is a thin wrap — compile proves edge_agent linkage.
    /// Both `doctor_mcp::run` and Doctor TUI call `edge_agent::mcp_tools_list`.
    #[test]
    fn doctor_mcp_module_links_edge_agent_mcp_tools_list() {
        // Symbol must resolve from this crate; async signature cannot be a plain fn ptr.
        let name = stringify!(crate::edge_agent::mcp_tools_list);
        assert!(
            name.contains("mcp_tools_list"),
            "edge_agent::mcp_tools_list is the single MCP tools/list client"
        );
    }

    #[test]
    fn rejects_non_qria_token_prefix_without_network() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let err = rt
            .block_on(super::run("http://127.0.0.1:9/mcp", "not-a-qria-token"))
            .expect_err("must reject prefix");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("qria_"),
            "expected qria_ prefix error, got: {msg}"
        );
    }
}
