use anyhow::{Context, bail};
use serde_json::json;

pub async fn run(url: &str, agent_token: &str) -> anyhow::Result<()> {
    println!("mcp_url={url}");
    if !agent_token.starts_with("qria_") {
        bail!("agent token must start with qria_");
    }
    let response = reqwest::Client::new()
        .post(url)
        .header("Authorization", format!("Bearer {agent_token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list"
        }))
        .send()
        .await
        .with_context(|| format!("failed to connect to MCP endpoint {url}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .context("failed to read MCP response")?;
    println!("mcp_status={status}");
    println!("mcp_body={body}");
    if !status.is_success() {
        bail!("MCP doctor failed with status {status}");
    }
    Ok(())
}
