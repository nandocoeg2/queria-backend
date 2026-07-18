# Agent-Driven Onboarding Implementation Plan

> **For agentic workers:** Execute task-by-task. Checkboxes track progress.

**Goal:** Let an LLM onboard a coding client to QuerIa by fetching live setup docs and applying MCP + AGENTS.md locally (enowx-style), without the server writing to remote agent machines.

**Architecture:** Public read-only API under `queria-api`: markdown agent-setup docs (live base URL), MCP config snippets per client, AGENTS.md block template. The agent installs MCP config and merges AGENTS.md on **its own** host using shell/file tools. Operator still issues the agent token (centralized auth). No server-side `install-mcp` that mutates client home dirs on the QuerIa host.

**Tech Stack:** Rust, axum, serde_json, existing `ApiState`, Caddy `/api/*` edge.

**Global Constraints:**
- Edge path: `/api/v1/...` via host port `17674` (not `67671`)
- Token format prefix `qria_`; env `QUERIA_AGENT_TOKEN`
- MCP tools: retrieve_context, search_knowledge, propose_memory, index_memory, list_projects, get_source
- Do not expose setup tokens or secrets in agent-setup docs
- Do not dual-maintain status outside HANDOFF for shipped claims

## File map

| File | Role |
|---|---|
| `crates/queria-api/src/http/agent_setup.rs` | Handlers + markdown/snippet builders |
| `crates/queria-api/src/http/mod.rs` | Export module |
| `crates/queria-api/src/app.rs` | Nest routes |
| `docs/runbooks/onboarding.md` | Part C agent-driven flow |
| `docs/HANDOFF.md` | One-line capability note when verified |
| `docs/README.md` (backend) | Index if needed |

### Task 1: Public agent-setup docs + snippets API

**Files:** create `agent_setup.rs`; modify `mod.rs`, `app.rs`

- [x] GET `/api/v1/docs/agent-setup` → `text/markdown` instructions (probe, token env, MCP install, AGENTS block, smoke tools)
- [x] GET `/api/v1/docs/setup` → alias same body
- [x] GET `/api/v1/setup/mcp-snippet?client=<id>&mcp_url=<optional>` → JSON `{ client, path_hint, format, content }`
- [x] GET `/api/v1/setup/agents-block?project_slug=<slug>&project_id=<optional uuid>` → JSON `{ markers, markdown }`
- [x] Unit tests with `tower::ServiceExt::oneshot` (no DB required)

Clients for snippets: `claude`, `codex`, `cursor`, `droid`, `factory` (alias droid).

### Task 2: Docs runbook + HANDOFF note

- [x] Update `runbooks/onboarding.md` with Part C paste-prompt
- [x] Pointer in backend docs README if missing
- [x] HANDOFF capability row or residual note for agent-driven onboard

### Task 3: Verify

- [x] `cargo test -p queria-api` (agent_setup: 5 passed)
- [x] Manual: Host header drives base URL in markdown

**Out of scope:** Server writing `~/.cursor/mcp.json`; Admin UI Install button; skill auto-copy; automatic agent token mint without session.
