# Agent Auto-Retrieve Hooks Implementation Plan

> **For agentic workers:** Execute inline or with subagent-driven-development task-by-task.

**Goal:** Ship hybrid auto-retrieve (T4+R6+H1): agent-bearer HTTP retrieve for shell hooks, Droid/Claude hook snippets, stronger AGENTS block, fail-open throttle script.

**Architecture:** Shell hooks call `POST /api/v1/agent/retrieve-context` with `Authorization: Bearer qria_…`, reusing `RetrievalPrincipal::Agent` and the shared hybrid pipeline. Session-cookie retrieve stays unchanged. Setup API returns hook snippets; AGENTS.md stays soft MCP enforcement.

**Tech Stack:** Rust (axum, queria-api/mcp/search/db/core), bash + jq/curl hook script, Markdown setup docs.

## Global Constraints

- Bundle: SessionStart + UserPromptSubmit; 30s cooldown; query hash; ≤3500 chars inject; skip trivial; H1 soft only (no Edit deny).
- Fail-open: network/auth errors never block agent work.
- Token must have `RetrieveContext`; project must be in token slugs; no privileges beyond MCP retrieve.
- Edge port truth: **17674**. No hard deny path in v1.

## Files

| Action | Path |
|--------|------|
| Create | `crates/queria-api/src/http/agent_retrieval.rs` |
| Modify | `crates/queria-api/src/http/mod.rs` |
| Modify | `crates/queria-api/src/app.rs` |
| Modify | `crates/queria-api/src/http/agent_setup.rs` |
| Create | `agent-tools/hooks/queria-retrieve-hook.sh` |
| Modify | `docs/PRODUCT.md`, `docs/runbooks/onboarding.md`, `docs/HANDOFF.md` |
| Create | `docs/archive/superpowers/specs/2026-07-19-agent-auto-retrieve-hooks-design.md` |

### Task 1: Agent bearer retrieve + projects API

- [ ] Implement `agent_retrieval` router (`POST /agent/retrieve-context`, `GET /agent/projects`)
- [ ] Auth: Bearer → hash → `authenticate_agent_token`; require `RetrieveContext` for retrieve
- [ ] Resolve `project_id` or `project_slug`; clamp limit 1–10 for hook path; include_scratch default true
- [ ] Wire into `app.rs` under `/api/v1`
- [ ] Unit/integration tests (401/403/slug/flags)

### Task 2: Setup snippets + stronger AGENTS

- [ ] `GET /setup/hooks-snippet?client=droid|claude`
- [ ] Embed or serve hook script content
- [ ] Strengthen `agents_block_markdown` + agent-setup docs section

### Task 3: Hook script

- [ ] `agent-tools/hooks/queria-retrieve-hook.sh` with R5/R2/R3/R4 and fail-open
- [ ] Manual smoke notes in onboarding

### Task 4: Living docs

- [ ] PRODUCT surface row; onboarding B5; HANDOFF matrix row; design archive REFERENCE

---

Execute in order 1→4. Verify with `cargo test -p queria-api` (and workspace if feasible).
