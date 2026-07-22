//! Pure doctor-check helpers for hub TUI (no I/O, no network).

/// Severity of a single doctor check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckLevel {
    Pass,
    Warn,
    Fail,
}

/// One doctor check result item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckItem {
    pub id: &'static str,
    pub level: CheckLevel,
    pub detail: String,
    pub hint: String,
}

/// True when `url` host is loopback (localhost or 127.0.0.1).
pub fn is_loopback_host(url: &str) -> bool {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .split('@')
        .next_back()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    host == "localhost" || host == "127.0.0.1"
}

/// True when token is non-empty and starts with `qria_`.
pub fn token_looks_valid(token: Option<&str>) -> bool {
    token
        .map(|t| !t.is_empty() && t.starts_with("qria_"))
        .unwrap_or(false)
}

/// Parse JSON-RPC tools/list body; return `result.tools[].name` strings.
/// Missing or invalid JSON yields an empty vector.
pub fn mcp_tool_names_from_tools_list_body(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    value
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| {
                    tool.get("name")
                        .and_then(|n| n.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Assembled doctor checklist for hub TUI (pure; no I/O).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorSnapshot {
    pub items: Vec<CheckItem>,
    pub version: String,
    pub profile: Option<String>,
    pub edge_url: String,
    pub mcp_url: String,
}

fn url_host(url: &str) -> String {
    let host = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .split('@')
        .next_back()
        .unwrap_or("")
        .split(':')
        .next()
        .unwrap_or("")
        .to_string();
    if host.is_empty() {
        "(unknown host)".into()
    } else {
        host
    }
}

fn short_body(body: &str) -> String {
    const MAX: usize = 120;
    let trimmed = body.trim();
    if trimmed.chars().count() <= MAX {
        trimmed.to_string()
    } else {
        trimmed.chars().take(MAX).collect::<String>() + "…"
    }
}

/// Pure assembly of ordered doctor checks from pre-fetched health/MCP outcomes.
///
/// `permissions` is the optional agent permissions list (P2+ projects-status).
/// Pre-P2 callers pass `None`; IndexLocal is never inferred from MCP tool names.
#[allow(clippy::too_many_arguments)]
pub fn assemble_doctor_snapshot(
    version: &str,
    profile: Option<&str>,
    edge_url: &str,
    mcp_url: &str,
    token: Option<&str>,
    health: Result<(u16, String), String>,
    mcp: Result<(u16, String), String>,
    permissions: Option<&[String]>,
) -> DoctorSnapshot {
    let mut items = Vec::with_capacity(7);

    // 1. Version — always Pass
    items.push(CheckItem {
        id: "version",
        level: CheckLevel::Pass,
        detail: version.to_string(),
        hint: String::new(),
    });

    // 2. Token
    if token_looks_valid(token) {
        items.push(CheckItem {
            id: "token",
            level: CheckLevel::Pass,
            detail: "agent token present (qria_…)".into(),
            hint: String::new(),
        });
    } else {
        items.push(CheckItem {
            id: "token",
            level: CheckLevel::Fail,
            detail: "no valid agent token".into(),
            hint: "Open Config and set a qria_… agent token".into(),
        });
    }

    // 3. Edge / MCP URLs
    let edge_empty = edge_url.trim().is_empty();
    let mcp_empty = mcp_url.trim().is_empty();
    if edge_empty || mcp_empty {
        let mut missing = Vec::new();
        if edge_empty {
            missing.push("edge");
        }
        if mcp_empty {
            missing.push("mcp");
        }
        items.push(CheckItem {
            id: "urls",
            level: CheckLevel::Fail,
            detail: format!("missing URL: {}", missing.join(", ")),
            hint: "Open Config and set edge/MCP URLs".into(),
        });
    } else if is_loopback_host(edge_url) || is_loopback_host(mcp_url) {
        items.push(CheckItem {
            id: "urls",
            level: CheckLevel::Warn,
            detail: format!(
                "edge={} mcp={} (loopback)",
                url_host(edge_url),
                url_host(mcp_url)
            ),
            hint: "Use prod URL (e.g. https://queria.fjulian.id) if you expect public edge"
                .into(),
        });
    } else {
        items.push(CheckItem {
            id: "urls",
            level: CheckLevel::Pass,
            detail: format!("edge={} mcp={}", url_host(edge_url), url_host(mcp_url)),
            hint: String::new(),
        });
    }

    // 4. Edge health
    match health {
        Ok((200, _)) => {
            items.push(CheckItem {
                id: "health",
                level: CheckLevel::Pass,
                detail: format!("edge health ok ({})", url_host(edge_url)),
                hint: String::new(),
            });
        }
        Ok((status, body)) => {
            items.push(CheckItem {
                id: "health",
                level: CheckLevel::Fail,
                detail: format!(
                    "edge {} returned {status}: {}",
                    url_host(edge_url),
                    short_body(&body)
                ),
                hint: "Check edge is up and edge_url is correct".into(),
            });
        }
        Err(err) => {
            items.push(CheckItem {
                id: "health",
                level: CheckLevel::Fail,
                detail: format!("edge {} unreachable: {}", url_host(edge_url), short_body(&err)),
                hint: "Check edge is up and edge_url is correct".into(),
            });
        }
    }

    // 5. MCP tools/list (capture body for permissions when 200)
    let mut mcp_tools: Vec<String> = Vec::new();
    match mcp {
        Ok((200, body)) => {
            mcp_tools = mcp_tool_names_from_tools_list_body(&body);
            items.push(CheckItem {
                id: "mcp",
                level: CheckLevel::Pass,
                detail: format!("MCP tools/list ok ({})", url_host(mcp_url)),
                hint: String::new(),
            });
        }
        Ok((401, _)) => {
            items.push(CheckItem {
                id: "mcp",
                level: CheckLevel::Fail,
                detail: "Auth failed — token missing/invalid/revoked".into(),
                hint: "Open Config and set a valid qria_… agent token".into(),
            });
        }
        Ok((status, body)) => {
            items.push(CheckItem {
                id: "mcp",
                level: CheckLevel::Fail,
                detail: format!("MCP status {status}: {}", short_body(&body)),
                hint: "Check mcp_url and token".into(),
            });
        }
        Err(err) => {
            items.push(CheckItem {
                id: "mcp",
                level: CheckLevel::Fail,
                detail: format!("MCP unreachable: {}", short_body(&err)),
                hint: "Check mcp_url and network".into(),
            });
        }
    }

    // 6. Permissions (MCP tool names pre-P2) + IndexLocal warn
    let has_agent_tools = mcp_tools.iter().any(|n| n == "list_projects" || n == "retrieve_context");
    if has_agent_tools {
        items.push(CheckItem {
            id: "permissions",
            level: CheckLevel::Pass,
            detail: "MCP tools include list_projects and/or retrieve_context".into(),
            hint: String::new(),
        });
    } else {
        items.push(CheckItem {
            id: "permissions",
            level: CheckLevel::Fail,
            detail: "MCP tools missing list_projects/retrieve_context".into(),
            hint: "Confirm agent token scopes and MCP endpoint".into(),
        });
    }

    let has_index_local = permissions
        .map(|perms| perms.iter().any(|p| p == "index_local"))
        .unwrap_or(false);
    if has_index_local {
        items.push(CheckItem {
            id: "index_local",
            level: CheckLevel::Pass,
            detail: "token has index_local".into(),
            hint: String::new(),
        });
    } else {
        items.push(CheckItem {
            id: "index_local",
            level: CheckLevel::Warn,
            detail: "index-here needs Custom token with index_local (Daily cannot upload)".into(),
            hint: "Mint a Custom agent token with index_local for uploads".into(),
        });
    }

    DoctorSnapshot {
        items,
        version: version.to_string(),
        profile: profile.map(|s| s.to_string()),
        edge_url: edge_url.to_string(),
        mcp_url: mcp_url.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_detects_localhost_and_127() {
        assert!(is_loopback_host("http://127.0.0.1:17674/mcp"));
        assert!(is_loopback_host("https://localhost/mcp"));
        assert!(!is_loopback_host("https://queria.fjulian.id/mcp"));
    }

    #[test]
    fn token_validation() {
        assert!(token_looks_valid(Some("qria_abc")));
        assert!(!token_looks_valid(Some("")));
        assert!(!token_looks_valid(Some("Bearer x")));
        assert!(!token_looks_valid(None));
    }

    #[test]
    fn parse_tools_list_names() {
        let body = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"list_projects"},{"name":"retrieve_context"}]}}"#;
        let names = mcp_tool_names_from_tools_list_body(body);
        assert!(names.iter().any(|n| n == "list_projects"));
        assert!(names.iter().any(|n| n == "retrieve_context"));
    }

    #[test]
    fn assemble_fails_without_token() {
        let snap = assemble_doctor_snapshot(
            "0.2.7",
            Some("default"),
            "https://queria.fjulian.id",
            "https://queria.fjulian.id/mcp",
            None,
            Ok((200, "ok".into())),
            Ok((200, r#"{"result":{"tools":[{"name":"list_projects"}]}}"#.into())),
            None,
        );
        assert!(snap
            .items
            .iter()
            .any(|i| i.id == "token" && matches!(i.level, CheckLevel::Fail)));
    }

    #[test]
    fn assemble_warns_loopback_mcp() {
        let snap = assemble_doctor_snapshot(
            "0.2.7",
            Some("default"),
            "http://127.0.0.1:17674",
            "http://127.0.0.1:17674/mcp",
            Some("qria_testtoken"),
            Ok((200, "ok".into())),
            Ok((200, r#"{"result":{"tools":[{"name":"retrieve_context"}]}}"#.into())),
            None,
        );
        assert!(snap
            .items
            .iter()
            .any(|i| i.id == "urls" && matches!(i.level, CheckLevel::Warn)));
    }
}
