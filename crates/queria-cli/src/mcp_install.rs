//! Fetch live MCP snippets from edge and apply safely to local client configs.

use crate::credentials::ResolvedCredentials;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct Snippet {
    pub client: String,
    pub path_hint: String,
    pub format: String,
    pub content: String,
}

pub async fn fetch_snippet(edge: &str, client: &str) -> Result<Snippet> {
    let client = normalize_client(client)?;
    let base = edge.trim().trim_end_matches('/');
    let url = format!("{base}/api/v1/setup/mcp-snippet?client={client}");
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let resp = http
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("mcp-snippet HTTP {status}: {body}");
    }
    let v: Value = resp.json().await.context("parse mcp-snippet JSON")?;
    Ok(Snippet {
        client: v
            .get("client")
            .and_then(|x| x.as_str())
            .unwrap_or(&client)
            .to_owned(),
        path_hint: v
            .get("path_hint")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_owned(),
        format: v
            .get("format")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_owned(),
        content: v
            .get("content")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_owned(),
    })
}

fn normalize_client(raw: &str) -> Result<String> {
    let c = raw.trim().to_ascii_lowercase();
    match c.as_str() {
        "droid" | "factory" => Ok("droid".into()),
        "claude" => Ok("claude".into()),
        "cursor" => Ok("cursor".into()),
        "codex" => Ok("codex".into()),
        _ => bail!("unknown client {raw:?}; use droid|claude|cursor|codex"),
    }
}

pub fn expand_path_hint(hint: &str) -> Result<PathBuf> {
    let hint = hint.trim();
    if hint.is_empty() {
        bail!("empty path_hint from snippet");
    }
    // Prefer first alternative before " or "
    let primary = hint
        .split(" or ")
        .next()
        .unwrap_or(hint)
        .split('(')
        .next()
        .unwrap_or(hint)
        .trim();
    if let Some(rest) = primary.strip_prefix("~/") {
        let home = std::env::var("HOME").context("HOME required to expand ~")?;
        return Ok(PathBuf::from(home).join(rest));
    }
    if primary.starts_with('~') {
        let home = std::env::var("HOME").context("HOME required to expand ~")?;
        return Ok(PathBuf::from(home));
    }
    Ok(PathBuf::from(primary))
}

fn backup_file(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let bak = PathBuf::from(format!("{}.queria-bak-{ts}", path.display()));
    fs::copy(path, &bak)
        .with_context(|| format!("backup {} → {}", path.display(), bak.display()))?;
    Ok(Some(bak))
}

/// Upsert mcpServers.queria into a Cursor/Claude-style JSON file.
pub fn upsert_json_mcp_servers(existing: Option<&str>, snippet_content: &str) -> Result<String> {
    let snippet: Value =
        serde_json::from_str(snippet_content).context("snippet content is not JSON")?;
    let queria = snippet
        .pointer("/mcpServers/queria")
        .cloned()
        .or_else(|| {
            snippet
                .get("mcpServers")
                .and_then(|m| m.get("queria"))
                .cloned()
        })
        .context("snippet missing mcpServers.queria")?;

    let mut root: Value = if let Some(raw) = existing.filter(|s| !s.trim().is_empty()) {
        serde_json::from_str(raw).context("existing file is not JSON; refuse merge")?
    } else {
        json!({ "mcpServers": {} })
    };

    if !root.is_object() {
        bail!("existing JSON root is not an object");
    }
    let servers = root
        .as_object_mut()
        .unwrap()
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if !servers.is_object() {
        bail!("mcpServers is not an object");
    }
    servers
        .as_object_mut()
        .unwrap()
        .insert("queria".into(), queria);
    Ok(serde_json::to_string_pretty(&root)? + "\n")
}

/// Best-effort Codex TOML: if file missing, write snippet content; if present, refuse full clobber and require empty or already-only-queria.
pub fn apply_codex_toml(existing: Option<&str>, snippet_content: &str) -> Result<String> {
    if existing.map(|s| s.trim().is_empty()).unwrap_or(true) {
        return Ok(if snippet_content.ends_with('\n') {
            snippet_content.to_owned()
        } else {
            format!("{snippet_content}\n")
        });
    }
    let ex = existing.unwrap();
    if ex.contains("[mcp_servers.queria]") || ex.contains("mcp_servers.queria") {
        // replace simple block: write snippet only if file has no other mcp_servers
        let other = ex
            .lines()
            .any(|l| l.trim().starts_with("[mcp_servers.") && !l.contains("queria"));
        if other {
            bail!(
                "codex config has other mcp_servers; dry-run and merge manually\n--- snippet ---\n{snippet_content}"
            );
        }
        return Ok(if snippet_content.ends_with('\n') {
            snippet_content.to_owned()
        } else {
            format!("{snippet_content}\n")
        });
    }
    // append snippet
    let mut out = ex.to_owned();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(snippet_content);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

pub async fn install(
    creds: &ResolvedCredentials,
    client: &str,
    dry_run: bool,
    yes: bool,
) -> Result<()> {
    let snippet = fetch_snippet(&creds.edge_url, client).await?;
    println!("client: {}", snippet.client);
    println!("path_hint: {}", snippet.path_hint);
    println!("format: {}", snippet.format);

    let format = snippet.format.to_ascii_lowercase();
    let content = snippet.content.clone();

    if format == "shell" || content.trim_start().starts_with("droid ") || content.contains("claude mcp add")
    {
        println!("--- shell snippet ---\n{content}");
        if dry_run {
            println!("dry-run: not executing shell");
            return Ok(());
        }
        if !yes {
            bail!("shell snippet: re-run with --yes to execute, or run the printed commands manually");
        }
        // Execute via sh -c only the non-comment lines carefully — safer: write temp script
        let tmp = std::env::temp_dir().join(format!(
            "queria-mcp-install-{}.sh",
            uuid::Uuid::now_v7()
        ));
        let mut script = String::from("#!/usr/bin/env bash\nset -euo pipefail\n");
        if let Some(t) = creds.agent_token.as_deref() {
            script.push_str(&format!(
                "export QUERIA_AGENT_TOKEN='{}'\n",
                t.replace('\'', r"'\''")
            ));
        }
        script.push_str(&format!(
            "export QUERIA_MCP_URL='{}'\n",
            creds.mcp_url.replace('\'', r"'\''")
        ));
        script.push_str(&content);
        script.push('\n');
        fs::write(&tmp, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o700))?;
        }
        let status = Command::new("bash").arg(&tmp).status()?;
        let _ = fs::remove_file(&tmp);
        if !status.success() {
            bail!("shell install exited {status}");
        }
        println!("shell install finished");
        return Ok(());
    }

    let path = expand_path_hint(&snippet.path_hint)?;
    let existing = if path.exists() {
        Some(fs::read_to_string(&path)?)
    } else {
        None
    };

    let new_body = if format == "json" || path.extension().and_then(|e| e.to_str()) == Some("json")
    {
        upsert_json_mcp_servers(existing.as_deref(), &content)?
    } else if format == "toml" || path.extension().and_then(|e| e.to_str()) == Some("toml") {
        apply_codex_toml(existing.as_deref(), &content)?
    } else {
        bail!(
            "unsupported format {:?}; path {:?}. Use --dry-run content:\n{content}",
            snippet.format,
            path
        );
    };

    if dry_run {
        println!("would write {}", path.display());
        println!("{new_body}");
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if let Some(bak) = backup_file(&path)? {
        println!("backup: {}", bak.display());
    }
    fs::write(&path, new_body.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    println!("wrote {}", path.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_preserves_sibling_servers() {
        let existing = r#"{
  "mcpServers": {
    "other": { "url": "http://x" }
  }
}"#;
        let snippet = r#"{
  "mcpServers": {
    "queria": {
      "url": "https://edge/mcp",
      "headers": { "Authorization": "Bearer ${QUERIA_AGENT_TOKEN}" }
    }
  }
}"#;
        let out = upsert_json_mcp_servers(Some(existing), snippet).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["mcpServers"]["other"].is_object());
        assert_eq!(v["mcpServers"]["queria"]["url"], "https://edge/mcp");
    }

    #[test]
    fn expand_tilde() {
        unsafe {
            std::env::set_var("HOME", "/tmp/home-test-queria");
        }
        let p = expand_path_hint("~/.cursor/mcp.json").unwrap();
        assert!(p.ends_with(".cursor/mcp.json"));
        unsafe {
            std::env::remove_var("HOME");
        }
    }
}
