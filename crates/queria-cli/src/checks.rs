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
}
