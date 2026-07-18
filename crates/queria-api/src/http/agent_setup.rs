//! Agent-driven onboarding: public setup docs and MCP/AGENTS snippets.
//!
//! Unlike enowx-rag, QuerIa does **not** write MCP configs onto remote agent
//! machines from the API host. The LLM fetches these endpoints, then applies
//! config on the agent workstation with local tools.

use crate::app::ApiState;
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::Deserialize;
use serde_json::{Value, json};

const AGENTS_MARKER_START: &str = "<!-- queria:start -->";
const AGENTS_MARKER_END: &str = "<!-- queria:end -->";

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/docs/agent-setup", get(agent_setup_docs))
        .route("/docs/setup", get(agent_setup_docs))
        .route("/setup/mcp-snippet", get(mcp_snippet))
        .route("/setup/agents-block", get(agents_block_handler))
}

/// Prefer reverse-proxy headers so markdown links use the public edge base.
fn request_base(headers: &HeaderMap) -> String {
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:17674");
    format!("{proto}://{host}")
}

async fn agent_setup_docs(headers: HeaderMap, State(_state): State<ApiState>) -> Response {
    let base = request_base(&headers);
    let body = agent_setup_markdown(&base);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/markdown; charset=utf-8")],
        body,
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
struct McpSnippetQuery {
    /// claude | codex | cursor | droid | factory
    client: String,
    /// Optional absolute MCP URL (defaults to edge /mcp on this base).
    mcp_url: Option<String>,
}

async fn mcp_snippet(
    headers: HeaderMap,
    Query(query): Query<McpSnippetQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let base = request_base(&headers);
    let mcp_url = query
        .mcp_url
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("{base}/mcp"));
    let client = normalize_client(&query.client);
    let snippet = match client.as_str() {
        "claude" => claude_snippet(&mcp_url),
        "codex" => codex_snippet(&mcp_url),
        "cursor" => cursor_snippet(&mcp_url),
        "droid" | "factory" => droid_snippet(&mcp_url),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "unknown_client",
                    "allowed": ["claude", "codex", "cursor", "droid", "factory"]
                })),
            ));
        }
    };
    Ok(Json(snippet))
}

#[derive(Debug, Deserialize)]
struct AgentsBlockQuery {
    project_slug: String,
    project_id: Option<String>,
}

async fn agents_block_handler(
    Query(query): Query<AgentsBlockQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let slug = query.project_slug.trim();
    if slug.is_empty() || slug.len() > 128 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "invalid_project_slug" })),
        ));
    }
    let markdown = agents_block_markdown(slug, query.project_id.as_deref());
    Ok(Json(json!({
        "project_slug": slug,
        "marker_start": AGENTS_MARKER_START,
        "marker_end": AGENTS_MARKER_END,
        "markdown": markdown,
    })))
}

fn normalize_client(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

fn agent_setup_markdown(base: &str) -> String {
    format!(
        r#"# QuerIa agent setup

Connect a coding agent to the centralized QuerIa knowledge hub (MCP over HTTP).
Do the steps in order. Skip steps the human already finished (token issued, MCP already configured).

**Server base (this request):** {base}

Public edge (typical):
- Admin: `{base}/admin`
- MCP: `{base}/mcp`
- API: `{base}/api/v1/...`
- Health: `{base}/healthz`

Do **not** use stale port `67671` or service name `queria-proxy`. Edge is Caddy on host port **17674** in local/prod compose.

---

## Prerequisites (human / operator)

1. QuerIa stack is up (`GET {base}/healthz` → 200).
2. A **project** exists (Admin `/admin/projects`) and knowledge is indexed or embeddings are in progress.
3. An **agent token** was created (`POST {base}/api/v1/agent-tokens` with session cookie). Copy the raw token once (`qria_…`).
   - Include tools you need. Default write path is propose-only; add `index_memory` for scratch.
   - Scope `project_slugs` to the projects this agent may access.
   - Set `allow_global_knowledge: true` if global trusted knowledge should be retrievable.

The agent **cannot** mint tokens. Ask the operator for `QUERIA_AGENT_TOKEN` if missing.

---

## 1. Export token (agent machine)

```bash
export QUERIA_AGENT_TOKEN='qria_…'   # from operator
export QUERIA_MCP_URL='{base}/mcp'
```

Never commit the raw token.

---

## 2. Install MCP client config (on the agent machine)

QuerIa is centralized: install remote HTTP MCP with bearer auth. Do **not** expect the QuerIa server to write `~/.cursor` on a remote laptop.

Fetch a ready snippet:

```http
GET {base}/api/v1/setup/mcp-snippet?client=<CLIENT>
```

`CLIENT` one of: `claude`, `codex`, `cursor`, `droid` (or `factory`).

Optional: `mcp_url=` absolute override.

Then write the returned `content` to `path_hint` (merge carefully; keep backups).

### Claude Code

```bash
claude mcp add --transport http --scope user queria "$QUERIA_MCP_URL" \
  --header "Authorization: Bearer ${{QUERIA_AGENT_TOKEN}}"
```

Or project `.mcp.json` using `headersHelper` so the token stays in env (see snippet endpoint).

### Codex

`~/.codex/config.toml`:

```toml
[mcp_servers.queria]
url = "{base}/mcp"
bearer_token_env_var = "QUERIA_AGENT_TOKEN"
startup_timeout_sec = 20
tool_timeout_sec = 60
enabled = true
```

### Factory Droid

Prefer HTTP MCP with Authorization header to `$QUERIA_MCP_URL`. Use `droid mcp add` when your CLI supports remote headers; otherwise paste the droid snippet from the endpoint.

### Cursor

Use `~/.cursor/mcp.json` or project `.cursor/mcp.json` with url + headers Authorization Bearer (snippet endpoint).

---

## 3. Merge project AGENTS.md (agent machine)

```http
GET {base}/api/v1/setup/agents-block?project_slug=<SLUG>&project_id=<OPTIONAL_UUID>
```

Merge the returned `markdown` into the repo `AGENTS.md` **idempotently**:
- If markers `<!-- queria:start -->` … `<!-- queria:end -->` exist, replace that block.
- Else if `AGENTS.md` exists, append the block after a `---` separator.
- Else create `AGENTS.md` with the block.

Do not delete the rest of the file.

---

## 4. Smoke tools

After MCP connects:

1. `list_projects` — only projects allowed by the token.
2. Note each project's **UUID** (`project_id` for retrieve/search/index_memory).
3. `retrieve_context` with `project_id` + a real query.
4. Optional: `index_memory` (scratch) if the token includes that tool.
5. Optional: `propose_memory` for team truth (needs Admin approval).

Operator Playground: `{base}/admin/playground`.

---

## Workflow contract

```text
Before work: retrieve_context(project_id, query)   # include_scratch default true
After (fast): index_memory(...)                   # scratch, project only
After (team): propose_memory(...)                 # proposed → human approve → trusted
```

Trusted code knowledge still enters via Git ingestion + approval, not agent overwrite.

---

## API helpers (this setup pack)

| Method | Path | Auth |
|---|---|---|
| GET | `{base}/api/v1/docs/agent-setup` | none |
| GET | `{base}/api/v1/docs/setup` | none (alias) |
| GET | `{base}/api/v1/setup/mcp-snippet?client=` | none |
| GET | `{base}/api/v1/setup/agents-block?project_slug=` | none |

Creating projects, sources, and agent tokens remains **session Admin HTTP** (see full onboarding runbook).

---

## Troubleshooting

| Symptom | Check |
|---|---|
| Edge not 200 | Stack / wrong port |
| MCP 401 | Token env, `Bearer ` prefix |
| Empty retrieve | Embeddings not ready; wrong project UUID |
| Missing `index_memory` | Token tools list |
"#,
        base = base
    )
}

fn agents_block_markdown(project_slug: &str, project_id: Option<&str>) -> String {
    let id_line = match project_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => format!(
            "- Project slug: `{slug}`\n- Project UUID (for retrieve/search/index_memory): `{id}`\n",
            slug = project_slug,
            id = id
        ),
        None => format!(
            "- Project slug: `{slug}`\n- Resolve UUID via MCP `list_projects` before retrieve.\n",
            slug = project_slug
        ),
    };

    format!(
        r#"{start}
## QuerIa knowledge (project: {slug})

This project uses the central QuerIa MCP gateway for dual-lane knowledge (trusted + scratch).

{id_line}
- **Before coding:** call `retrieve_context` with the project UUID and the user's query; prefer cited trusted facts; use scratch as ephemeral notes.
- **After coding (fast):** call `index_memory` for project-scoped scratch (not team truth).
- **After coding (team truth):** call `propose_memory` with slug `{slug}`; do not treat it as approved until a human approves.
- Never write global or cross-project scratch. Never overwrite trusted knowledge via MCP.
- Git ingestion and Admin approvals remain the path for official codebase memory.
{end}
"#,
        start = AGENTS_MARKER_START,
        end = AGENTS_MARKER_END,
        slug = project_slug,
        id_line = id_line,
    )
}

fn claude_snippet(mcp_url: &str) -> Value {
    let content = format!(
        r#"{{
  "mcpServers": {{
    "queria": {{
      "type": "http",
      "url": "{url}",
      "headersHelper": "printf '{{\"Authorization\":\"Bearer %s\"}}' \"$QUERIA_AGENT_TOKEN\"",
      "timeout": 60000
    }}
  }}
}}
"#,
        url = mcp_url
    );
    json!({
        "client": "claude",
        "path_hint": ".mcp.json (project) or use `claude mcp add --transport http`",
        "format": "json",
        "env": ["QUERIA_AGENT_TOKEN"],
        "content": content,
    })
}

fn codex_snippet(mcp_url: &str) -> Value {
    let content = format!(
        r#"[mcp_servers.queria]
url = "{url}"
bearer_token_env_var = "QUERIA_AGENT_TOKEN"
startup_timeout_sec = 20
tool_timeout_sec = 60
enabled = true
enabled_tools = [
  "retrieve_context",
  "search_knowledge",
  "propose_memory",
  "index_memory",
  "list_projects",
  "get_source"
]
"#,
        url = mcp_url
    );
    json!({
        "client": "codex",
        "path_hint": "~/.codex/config.toml or .codex/config.toml",
        "format": "toml",
        "env": ["QUERIA_AGENT_TOKEN"],
        "content": content,
    })
}

fn cursor_snippet(mcp_url: &str) -> Value {
    let content = format!(
        r#"{{
  "mcpServers": {{
    "queria": {{
      "url": "{url}",
      "headers": {{
        "Authorization": "Bearer ${{QUERIA_AGENT_TOKEN}}"
      }}
    }}
  }}
}}
"#,
        url = mcp_url
    );
    json!({
        "client": "cursor",
        "path_hint": "~/.cursor/mcp.json or .cursor/mcp.json",
        "format": "json",
        "env": ["QUERIA_AGENT_TOKEN"],
        "content": content,
        "note": "If headers env expansion is unsupported, paste the raw token only in a private user config (never commit)."
    })
}

fn droid_snippet(mcp_url: &str) -> Value {
    let content = format!(
        r#"# Factory Droid — remote MCP (pattern; adjust to your droid mcp CLI)
# export QUERIA_AGENT_TOKEN=...
droid mcp add queria {url} \
  --header "Authorization: Bearer ${{QUERIA_AGENT_TOKEN}}"
"#,
        url = mcp_url
    );
    json!({
        "client": "droid",
        "path_hint": "run droid mcp add (or paste into Factory MCP settings as HTTP + Authorization header)",
        "format": "shell",
        "env": ["QUERIA_AGENT_TOKEN"],
        "content": content,
        "mcp_url": mcp_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::build_app;
    use axum::body::Body;
    use http::Request;
    use queria_core::AppConfig;
    use tower::ServiceExt;

    async fn body_string(response: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }

    #[tokio::test]
    async fn agent_setup_docs_are_public_markdown() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/docs/agent-setup")
                    .header("host", "127.0.0.1:17674")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let ct = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(ct.contains("markdown"), "content-type={ct}");
        let body = body_string(response).await;
        assert!(body.contains("QuerIa agent setup"));
        assert!(body.contains("127.0.0.1:17674"));
        assert!(body.contains("QUERIA_AGENT_TOKEN"));
        assert!(body.contains("retrieve_context"));
        assert!(body.contains("Do **not** use stale port"));
    }

    #[tokio::test]
    async fn setup_docs_alias_matches() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/docs/setup")
                    .header("host", "example.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("http://example.test/mcp"));
    }

    #[tokio::test]
    async fn mcp_snippet_codex_ok() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/setup/mcp-snippet?client=codex")
                    .header("host", "127.0.0.1:17674")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("bearer_token_env_var"));
        assert!(body.contains("127.0.0.1:17674/mcp"));
    }

    #[tokio::test]
    async fn mcp_snippet_unknown_client_400() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/setup/mcp-snippet?client=nope")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn agents_block_contains_markers() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/setup/agents-block?project_slug=fjulian-me&project_id=abc-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains(AGENTS_MARKER_START));
        assert!(body.contains("fjulian-me"));
        assert!(body.contains("abc-123"));
        assert!(body.contains("retrieve_context"));
    }
}
