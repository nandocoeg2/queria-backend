//! Fetch live MCP snippets from edge and apply safely to local client configs.

use crate::credentials::ResolvedCredentials;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
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

pub async fn fetch_snippet(edge: &str, client: &str, mcp_url: Option<&str>) -> Result<Snippet> {
    let client = normalize_client(client)?;
    let base = edge.trim().trim_end_matches('/');
    let mut url = format!("{base}/api/v1/setup/mcp-snippet?client={client}");
    if let Some(mcp) = mcp_url.map(str::trim).filter(|s| !s.is_empty()) {
        url.push_str(&format!("&mcp_url={}", urlencoding_lite(mcp)));
    }
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

/// Minimal query-escape for mcp_url (no extra deps).
fn urlencoding_lite(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'/' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
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

fn droid_install_script(mcp_url: &str) -> String {
    format!(
        r#"droid mcp add queria {url} \
  --type http \
  --no-oauth \
  --header "Authorization: Bearer ${{QUERIA_AGENT_TOKEN}}"
"#,
        url = mcp_url
    )
}

fn run_droid_mcp_add(creds: &ResolvedCredentials) -> Result<()> {
    let token = creds
        .agent_token
        .as_deref()
        .filter(|t| !t.is_empty())
        .context("agent token required for droid MCP install")?;
    let header = format!("Authorization: Bearer {token}");
    let output = Command::new("droid")
        .args([
            "mcp",
            "add",
            "queria",
            &creds.mcp_url,
            "--type",
            "http",
            "--no-oauth",
            "--header",
            &header,
        ])
        .output()
        .context("run droid mcp add (is `droid` on PATH?)")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        // Idempotent-ish: already configured is OK for re-run.
        let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();
        if combined.contains("already") || combined.contains("exists") {
            println!("droid: queria MCP already configured");
            return Ok(());
        }
        bail!(
            "droid mcp add failed ({})\n{}\n{}",
            output.status,
            stdout.trim(),
            stderr.trim()
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        println!("{}", stdout.trim());
    }
    Ok(())
}

/// Prefer configured MCP URL if snippet still contains localhost defaults.
fn rewrite_snippet_urls(content: &str, mcp_url: &str) -> String {
    content
        .replace("http://127.0.0.1:17674/mcp", mcp_url)
        .replace("http://localhost:17674/mcp", mcp_url)
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
    let client_norm = normalize_client(client)?;
    // Prefer local resolved MCP URL so install works even when edge still advertises localhost.
    let snippet = fetch_snippet(&creds.edge_url, &client_norm, Some(&creds.mcp_url)).await?;
    println!("client: {}", snippet.client);
    println!("mcp_url: {}", creds.mcp_url);
    println!("path_hint: {}", snippet.path_hint);
    println!("format: {}", snippet.format);

    let format = snippet.format.to_ascii_lowercase();
    let content = rewrite_snippet_urls(&snippet.content, &creds.mcp_url);

    // Droid shell install: build a known-good command (snippet alone may lack --type http).
    if client_norm == "droid"
        || format == "shell"
        || content.trim_start().starts_with("droid ")
        || content.contains("claude mcp add")
    {
        let shell_body = if client_norm == "droid" {
            droid_install_script(&creds.mcp_url)
        } else {
            content.clone()
        };
        println!("--- shell snippet ---\n{shell_body}");
        if dry_run {
            println!("dry-run: not executing shell");
            return Ok(());
        }
        if !yes {
            bail!(
                "shell snippet: re-run with --yes to execute, or run the printed commands manually"
            );
        }
        if client_norm == "droid" {
            run_droid_mcp_add(creds)?;
            println!("MCP install (droid) finished OK");
            return Ok(());
        }
        let tmp =
            std::env::temp_dir().join(format!("queria-mcp-install-{}.sh", uuid::Uuid::now_v7()));
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
        script.push_str(&shell_body);
        script.push('\n');
        fs::write(&tmp, script)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp, fs::Permissions::from_mode(0o700))?;
        }
        let output = Command::new("bash").arg(&tmp).output()?;
        let _ = fs::remove_file(&tmp);
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            bail!(
                "shell install failed ({})\n{}\n{}",
                output.status,
                stdout.trim(),
                stderr.trim()
            );
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
