use anyhow::Context;
use serde_json::json;

pub async fn run(url: &str) -> anyhow::Result<()> {
    println!("mcp_url={url}");
    let response = reqwest::Client::new()
        .post(url)
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
    Ok(())
}
