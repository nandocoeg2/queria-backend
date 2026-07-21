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

/// Shared client-side auto-retrieve hook script (T4+R6+H1 fail-open).
const HOOK_SCRIPT: &str = include_str!("../../../../agent-tools/hooks/queria-retrieve-hook.sh");

pub fn router() -> Router<ApiState> {
    Router::new()
        .route("/docs/agent-setup", get(agent_setup_docs))
        .route("/docs/setup", get(agent_setup_docs))
        .route("/setup/mcp-snippet", get(mcp_snippet))
        .route("/setup/agents-block", get(agents_block_handler))
        .route("/setup/hooks-snippet", get(hooks_snippet))
        .route("/setup/hook-script", get(hook_script_handler))
}

/// Prefer configured public base; else reverse-proxy headers for edge base.
fn request_base(public_base_url: &str, headers: &HeaderMap) -> String {
    let configured = public_base_url.trim().trim_end_matches('/');
    if !configured.is_empty() {
        return configured.to_owned();
    }
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

async fn agent_setup_docs(headers: HeaderMap, State(state): State<ApiState>) -> Response {
    let base = request_base(&state.config.public_base_url, &headers);
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
    State(state): State<ApiState>,
    Query(query): Query<McpSnippetQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let base = request_base(&state.config.public_base_url, &headers);
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

#[derive(Debug, Deserialize)]
struct HooksSnippetQuery {
    /// droid | factory | claude
    client: String,
}

async fn hooks_snippet(
    headers: HeaderMap,
    State(state): State<ApiState>,
    Query(query): Query<HooksSnippetQuery>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let base = request_base(&state.config.public_base_url, &headers);
    let client = normalize_client(&query.client);
    let snippet = match client.as_str() {
        "droid" | "factory" => droid_hooks_snippet(&base),
        "claude" => claude_hooks_snippet(&base),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "unknown_client",
                    "allowed": ["droid", "factory", "claude"],
                    "note": "Codex uses AGENTS.md only (no native hooks in v1)"
                })),
            ));
        }
    };
    Ok(Json(snippet))
}

async fn hook_script_handler() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")],
        HOOK_SCRIPT.to_owned(),
    )
        .into_response()
}

fn normalize_client(raw: &str) -> String {
    raw.trim().to_ascii_lowercase().replace('_', "-")
}

fn agent_setup_markdown(base: &str) -> String {
    format!(
        r#"# QuerIa agent setup

Default path: **Daily agent** — mint once, set 2–3 env vars on the laptop, install MCP, then `list_projects` → `retrieve_context`.
Skip steps the human already finished.

**Server base (this request):** {base}

Public edge (typical):
- Admin: `{base}/admin`
- MCP: `{base}/mcp`
- API: `{base}/api/v1/...`
- Health: `{base}/healthz`

Do **not** use stale port `67671` or service name `queria-proxy`. Prefer this public base; local compose edge host port is **17674**.

---

## Prerequisites (human / operator)

1. QuerIa stack is up (`GET {base}/healthz` → 200).
2. A **project** exists (Admin `/admin/projects`). Knowledge may still be empty — connect still works.
3. An **agent token** was minted. Prefer Admin **Tokens → Daily agent** (connect panel copies env once).
   - Daily tools: `list_projects`, `retrieve_context`, `search_knowledge`, `propose_memory`, `get_source`, `index_memory`.
   - Daily does **not** include `index_local` or `manage_needs_review` (use **Custom** only when needed).
   - Scope `project_slugs` to the projects this agent may access (multi-slug OK).
   - Set `allow_global_knowledge: true` if global trusted knowledge should be retrievable.
   - API omit-`tools` is propose-only (no `index_memory`); Admin Daily always POSTs the Daily tool list.

The agent **cannot** mint tokens. Ask the operator for `QUERIA_AGENT_TOKEN` if missing.

**Useful answers need ready chunks** (Admin Git ingest/embed, or laptop `index-here` + Promote). Empty retrieve is fine for client connectivity.

---

## 1. Export env (agent machine, once)

```bash
export QUERIA_AGENT_TOKEN='qria_…'   # from Daily connect panel; never commit
export QUERIA_EDGE_URL='{base}'
export QUERIA_MCP_URL='{base}/mcp'
```

User-level session or shell profile is enough. Do **not** require a per-repo env file for Daily retrieve.
Optional: `QUERIA_PROJECT_SLUG` or `QUERIA_PROJECT_ID` **only if** auto-retrieve hooks need an active project.

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

Pick a project from `list_projects` (after MCP works), then:

```http
GET {base}/api/v1/setup/agents-block?project_slug=<SLUG>&project_id=<OPTIONAL_UUID>
```

Merge the returned `markdown` into the repo `AGENTS.md` **idempotently**:
- If markers `<!-- queria:start -->` … `<!-- queria:end -->` exist, replace that block.
- Else if `AGENTS.md` exists, append the block after a `---` separator.
- Else create `AGENTS.md` with the block.

Do not delete the rest of the file.

---

## 4. Install auto-retrieve hooks (optional; Droid / Claude)

Hooks inject condensed QuerIa context on **SessionStart** and throttled **UserPromptSubmit** (hybrid T4+R6+H1). Fail-open: edge down does not block work. Codex: AGENTS.md only (no native hooks in v1). **Skip hooks for a minimal Daily setup.**

When enabling hooks, also set an active project:

```bash
export QUERIA_AGENT_TOKEN='qria_…'
export QUERIA_EDGE_URL='{base}'
export QUERIA_MCP_URL='{base}/mcp'
export QUERIA_PROJECT_SLUG='<slug>'   # or QUERIA_PROJECT_ID=<uuid>
```

```http
GET {base}/api/v1/setup/hooks-snippet?client=<droid|claude>
GET {base}/api/v1/setup/hook-script
```

1. Write hook script to the path_hint under `scripts` from the snippet (chmod +x).
2. Merge the returned hooks JSON into `.factory/hooks.json` (Droid) or `.claude/settings.json` hooks key (Claude).
3. Requires `jq` and `curl` on the agent machine.
4. Soft inject only — still **MUST** call MCP `retrieve_context` for deep/task-specific context.

Agent HTTP (bearer, same authz as MCP retrieve):

| Method | Path |
|---|---|
| POST | `{base}/api/v1/agent/retrieve-context` |
| GET | `{base}/api/v1/agent/projects` |

---

## 5. Smoke tools

After MCP connects:

1. `list_projects` — only projects allowed by the token.
2. Note each project's **UUID** (`project_id` for retrieve/search/index_memory).
3. `retrieve_context` with `project_id` + a real query.
   - Hits: knowledge ready.
   - Empty: embeddings pending or no knowledge yet — **still connected** (not a client failure).
4. Optional: `index_memory` (scratch) if the token includes that tool (Daily does).
5. Optional: `propose_memory` for team truth (needs Admin approval).
6. Optional: start a new Droid/Claude session and confirm `## QuerIa context (auto)` appears when hooks are installed.

Operator Playground: `{base}/admin/playground`.

---

## Workflow contract

```text
Default Daily:
  list_projects → retrieve_context(project_id, query)
  after (fast): index_memory(...)      # scratch, project only
  after (team): propose_memory(...)    # proposed → human approve → trusted

Optional hooks:
  SessionStart / UserPromptSubmit: auto HTTP retrieve inject (fail-open, throttled)
```

Trusted code knowledge still enters via **Admin Git** ingestion or laptop **`index-here`** (Custom + `index_local`) then **Promote** — not via Daily overwrite.

---

## API helpers (this setup pack)

| Method | Path | Auth |
|---|---|---|
| GET | `{base}/api/v1/docs/agent-setup` | none |
| GET | `{base}/api/v1/docs/setup` | none (alias) |
| GET | `{base}/api/v1/setup/mcp-snippet?client=` | none |
| GET | `{base}/api/v1/setup/agents-block?project_slug=` | none |
| GET | `{base}/api/v1/setup/hooks-snippet?client=` | none |
| GET | `{base}/api/v1/setup/hook-script` | none |
| POST | `{base}/api/v1/agent/retrieve-context` | Bearer agent token |
| GET | `{base}/api/v1/agent/projects` | Bearer agent token |

Creating projects, sources, and agent tokens remains **session Admin HTTP** (see onboarding runbook: default 3-step Daily).

---

## Troubleshooting

| Symptom | Check |
|---|---|
| Edge not 200 | Stack / wrong host or port |
| MCP 401 | Token env, `Bearer ` prefix |
| Empty retrieve | No chunks yet or wrong project UUID — connect can still be OK |
| Missing `index_memory` | Not a Daily token; remint Daily or add tool |
| Hook silent | `jq`/`curl` present; QUERIA_EDGE_URL + token + project slug/id; `droid --debug` / Claude debug log |
"#,
        base = base
    )
}

fn agents_block_markdown(project_slug: &str, project_id: Option<&str>) -> String {
    let id_line = match project_id.map(str::trim).filter(|s| !s.is_empty()) {
        Some(id) => format!(
            "- Project slug: `{slug}`\n- Project UUID (for retrieve/search/index_memory): `{id}`\n- Env for hooks: `QUERIA_PROJECT_ID={id}` or `QUERIA_PROJECT_SLUG={slug}`\n",
            slug = project_slug,
            id = id
        ),
        None => format!(
            "- Project slug: `{slug}`\n- Resolve UUID via MCP `list_projects` (or GET `/api/v1/agent/projects`) before retrieve.\n- Env for hooks: `QUERIA_PROJECT_SLUG={slug}`\n",
            slug = project_slug
        ),
    };

    format!(
        r#"{start}
## QuerIa knowledge (project: {slug})

This project uses the central QuerIa MCP gateway for dual-lane knowledge (trusted + scratch).
Optional client hooks may inject a condensed `## QuerIa context (auto)` block on SessionStart / UserPromptSubmit (fail-open, throttled). That inject is **not** a substitute for deep MCP retrieve.

{id_line}
- **MUST before non-trivial coding:** call MCP `retrieve_context` with the project UUID and the user's task/query (even if a hook already injected context). Prefer cited **trusted** facts over scratch.
- **After coding (fast):** call `index_memory` for project-scoped scratch (not team truth) when the token grants it.
- **After coding (team truth):** call `propose_memory` with slug `{slug}`; do not treat it as approved until a human approves.
- Never write global or cross-project scratch. Never overwrite trusted knowledge via MCP.
- Git ingestion and Admin approvals remain the path for official codebase memory.
- Hooks env: `QUERIA_AGENT_TOKEN`, `QUERIA_EDGE_URL`, project id/slug. Install via `/api/v1/setup/hooks-snippet`.
{end}
"#,
        start = AGENTS_MARKER_START,
        end = AGENTS_MARKER_END,
        slug = project_slug,
        id_line = id_line,
    )
}

fn droid_hooks_snippet(base: &str) -> Value {
    let hooks_json = r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"$FACTORY_PROJECT_DIR\"/.factory/hooks/queria-retrieve-hook.sh",
            "timeout": 15
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"$FACTORY_PROJECT_DIR\"/.factory/hooks/queria-retrieve-hook.sh",
            "timeout": 15
          }
        ]
      }
    ]
  }
}
"#;
    json!({
        "client": "droid",
        "path_hint": ".factory/hooks.json",
        "script_path_hint": ".factory/hooks/queria-retrieve-hook.sh",
        "format": "json",
        "env": [
            "QUERIA_AGENT_TOKEN",
            "QUERIA_EDGE_URL",
            "QUERIA_PROJECT_ID",
            "QUERIA_PROJECT_SLUG"
        ],
        "edge_url_example": base,
        "content": hooks_json,
        "script": HOOK_SCRIPT,
        "install_notes": [
            "Write script content to script_path_hint and chmod +x",
            "Merge content into .factory/hooks.json (project scope)",
            "export QUERIA_EDGE_URL to the public edge base (port 17674)",
            "Requires jq and curl; fail-open if edge is down"
        ]
    })
}

fn claude_hooks_snippet(base: &str) -> Value {
    let hooks_json = r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}/.claude/hooks/queria-retrieve-hook.sh\"",
            "timeout": 15
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "\"${CLAUDE_PROJECT_DIR}/.claude/hooks/queria-retrieve-hook.sh\"",
            "timeout": 15
          }
        ]
      }
    ]
  }
}
"#;
    json!({
        "client": "claude",
        "path_hint": ".claude/settings.json (hooks key) or project settings",
        "script_path_hint": ".claude/hooks/queria-retrieve-hook.sh",
        "format": "json",
        "env": [
            "QUERIA_AGENT_TOKEN",
            "QUERIA_EDGE_URL",
            "QUERIA_PROJECT_ID",
            "QUERIA_PROJECT_SLUG"
        ],
        "edge_url_example": base,
        "content": hooks_json,
        "script": HOOK_SCRIPT,
        "install_notes": [
            "Write script content to script_path_hint and chmod +x",
            "Merge hooks object into Claude settings hooks configuration",
            "export QUERIA_EDGE_URL; never commit the raw agent token",
            "Requires jq and curl; fail-open if edge is down"
        ]
    })
}

fn claude_snippet(mcp_url: &str) -> Value {
    // Claude Code HTTP MCP auth is Bearer header — not OAuth.
    // Without --header, Claude tries /.well-known + /register (404 HTML on QuerIa).
    let content = format!(
        r#"# Claude Code — remote HTTP MCP (Bearer agent token; no OAuth)
# export QUERIA_AGENT_TOKEN=...
claude mcp add queria --transport http {url} \
  --header "Authorization: Bearer ${{QUERIA_AGENT_TOKEN}}"
"#,
        url = mcp_url
    );
    json!({
        "client": "claude",
        "path_hint": "run claude mcp add (or project .mcp.json with headers)",
        "format": "shell",
        "env": ["QUERIA_AGENT_TOKEN"],
        "content": content,
        "mcp_url": mcp_url,
        "note": "QuerIa has no OAuth; Claude must use Authorization Bearer header.",
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
        r#"# Factory Droid — remote HTTP MCP
# export QUERIA_AGENT_TOKEN=...
droid mcp add queria {url} \
  --type http \
  --no-oauth \
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

    #[test]
    fn request_base_prefers_configured_public_base_over_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "evil.example:9999".parse().unwrap());
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        let base = request_base("https://queria.fjulian.id/", &headers);
        assert_eq!(base, "https://queria.fjulian.id");
    }

    #[test]
    fn request_base_strips_trailing_slash() {
        let headers = HeaderMap::new();
        assert_eq!(
            request_base("http://127.0.0.1:17674/", &headers),
            "http://127.0.0.1:17674"
        );
    }

    #[test]
    fn request_base_empty_config_uses_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "edge.local:17674".parse().unwrap());
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        let base = request_base("   ", &headers);
        assert_eq!(base, "https://edge.local:17674");
    }

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
        assert!(body.contains("hooks-snippet"));
        assert!(body.contains("agent/retrieve-context"));
    }

    #[tokio::test]
    async fn setup_docs_alias_matches() {
        // Empty public base so Host header fallback is exercised.
        let mut config = AppConfig::default_local();
        config.public_base_url = String::new();
        let app = build_app(config);
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
    async fn setup_docs_prefers_configured_public_base() {
        let mut config = AppConfig::default_local();
        config.public_base_url = "https://queria.fjulian.id/".into();
        let app = build_app(config);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/docs/setup")
                    .header("host", "evil.example:9999")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("https://queria.fjulian.id/mcp"));
        assert!(!body.contains("evil.example"));
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
        assert!(body.contains("MUST before non-trivial coding"));
        assert!(body.contains("QuerIa context (auto)"));
    }

    #[tokio::test]
    async fn hooks_snippet_droid_includes_script_and_events() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/setup/hooks-snippet?client=droid")
                    .header("host", "127.0.0.1:17674")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("SessionStart"));
        assert!(body.contains("UserPromptSubmit"));
        assert!(body.contains("FACTORY_PROJECT_DIR"));
        assert!(body.contains("queria-retrieve-hook"));
        assert!(body.contains("fail-open") || body.contains("QUERIA_EDGE_URL"));
    }

    #[tokio::test]
    async fn hooks_snippet_claude_ok() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/setup/hooks-snippet?client=claude")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("CLAUDE_PROJECT_DIR"));
        assert!(body.contains("SessionStart"));
    }

    #[tokio::test]
    async fn hooks_snippet_unknown_client_400() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/setup/hooks-snippet?client=codex")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn hook_script_is_public_shell() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/setup/hook-script")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_string(response).await;
        assert!(body.contains("#!/usr/bin/env bash"));
        assert!(body.contains("QUERIA_AGENT_TOKEN"));
        assert!(body.contains("agent/retrieve-context"));
    }
}
