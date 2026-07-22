# queria-cli hub TUI design (doctor · index-here · remote status)

> Status: REFERENCE — approved design for implementation  
> Last verified: 2026-07-22  
> Runtime truth when shipped: [`../../HANDOFF.md`](../../HANDOFF.md)  
> Related: [`2026-07-21-queria-cli-config-design.md`](./2026-07-21-queria-cli-config-design.md),  
> [`2026-07-19-local-git-index-here-design.md`](./2026-07-19-local-git-index-here-design.md),  
> onboarding Daily vs Custom token path

## Problem

Laptop users hit repeated friction after config TUI shipped:

1. **Doctor / credentials** — MCP or edge pointed at `127.0.0.1`, old binary on PATH, empty Bearer, opaque 401/403.
2. **index-here** — multi-root walls (`--yes`), Daily token lacks `index_local`, discovery noise (path/origin/HEAD).
3. **Embed / needs_review status** — `queria-cli embeddings status` loads full **server** `AppConfig` + Postgres and fails with `QUERIA_SETUP_TOKEN must be replaced…` even when agent profile is correct. No agent-token path for counts.

Scripts still need non-interactive CLI. Humans need a single **TTY hub** that does not require DB or setup token.

## Goals

1. **Hub TUI** entry: `queria-cli tui` (TTY required) with menu: Doctor | Index | Status | Config.
2. **Doctor TUI** — friction pack: binary version, active profile (redacted), edge/mcp URL sanity (localhost warn), edge health, MCP `tools/list` auth, permission flags (esp. IndexLocal), clear next-step copy.
3. **Index-here wizard TUI** — discover → checklist roots → permission preflight → dry-run summary → upload → show `job_ids`; plain messaging (no origin/HEAD wall of text).
4. **Remote status via edge** — agent Bearer: per-project embed counts + needs_review counts **without** `AppConfig` / `QUERIA_SETUP_TOKEN` / laptop Postgres.
5. Keep existing non-TUI subcommands for CI (`index-here --dry-run|--yes`, `doctor mcp`, `config` TUI-only as today).

## Non-goals (v1)

- Bare `queria-cli` (no args) opening the hub (must stay clap **help** only).
- Promote / reject from TUI.
- Laptop `embeddings status|backfill` via agent token (server-only stays for maintainers).
- OS keychain, auto binary self-update, OAuth flows.
- Hooks install wizard, Admin UI parity.
- Nested Tokio runtime anti-patterns (reuse existing `#[tokio::main]` + `block_in_place` pattern).

## Locked decisions

| Topic | Choice |
|---|---|
| Hub entry | `queria-cli tui` only; bare CLI = help |
| Config | Reuse existing `config_tui` from hub |
| Credentials | Existing `credentials::resolve` (env > profile) |
| Stack | Modules in `queria-cli` + agent route in `queria-api`; no new crate |
| Status transport | **New** `GET /api/v1/agent/projects-status` (not only extend list body without opt-in; dedicated path) |
| Auth for status | Agent Bearer; allow if token has `ListProjects` and/or `RetrieveContext` (same spirit as `GET /agent/projects`) |
| IndexLocal detection | **Not** from MCP `tools/list` alone (`index_local` is HTTP agent API, not an MCP tool name). Prefer `permissions` list on `GET /api/v1/agent/projects-status` (or thin `GET /api/v1/agent/whoami` if split). Fallback probe: only after P2 — until then preflight message if upload returns 403. |
| Old edge | Status screen degrades: show doctor subset + message to redeploy if `projects-status` 404 |
| Phasing | P0 hub+doctor+config · P1 index wizard · P2 API + status screen |

## CLI surface

```text
queria-cli                  # clap help (no hub)
queria-cli --version
queria-cli tui              # hub TUI (TTY required)
queria-cli config           # existing config TUI
queria-cli doctor mcp       # existing non-TUI doctor
queria-cli index-here …     # existing flags (--yes, --dry-run, …)
```

Non-TTY `tui` → error: `queria-cli tui needs a TTY`.

Global `--profile` / `QUERIA_PROFILE` still apply to hub sessions.

## Architecture

```text
┌─ queria-cli tui (hub) ──────────────────────────────┐
│  [D]octor  [I]ndex  [S]tatus  [C]onfig  [q]uit      │
└──────┬──────────┬──────────┬──────────┬─────────────┘
       │          │          │          │
   doctor_tui  index_tui  status_tui  config_tui (existing)
       │          │          │
       └──── checks + edge_agent (HTTP Bearer) ───┘
                      │
         GET  {edge}/healthz  (or edge health path already used)
         POST {mcp_url} tools/list
         GET  {edge}/api/v1/agent/projects-status   ← new
         POST {edge}/api/v1/agent/index-local       ← existing
```

**Laptop path must not call** `AppConfig::from_env()` for hub doctor/index/status.

```text
crates/queria-cli/src/
  tui_hub.rs          # menu loop
  checks.rs           # pure + thin I/O wrappers
  edge_agent.rs       # HTTP client helpers for agent routes
  doctor_tui.rs
  index_tui.rs
  status_tui.rs
  config_tui.rs       # existing
  index_here.rs       # pure discover/plan/upload (reuse)
  doctor_mcp.rs       # reuse for tools/list body parse helpers if clean
  credentials.rs      # existing

crates/queria-api/src/http/
  agent_retrieval.rs  # or agent_status.rs — mount projects-status
```

## Components

### 1. Hub (`tui_hub`)

- Ratatui + crossterm (same stack as config).
- Home keys: `d` / `i` / `s` / `c` / `q` (and arrows + Enter).
- Status line: active profile name, edge host (no token).
- `c` → existing `config_tui::run_tui` then return to hub.

### 2. Doctor (`doctor_tui` + `checks`)

Checklist (pass / warn / fail) with one-line remediation:

| Check | Pass criteria | Fail / warn copy (intent) |
|---|---|---|
| Binary version | prints `CARGO_PKG_VERSION` | Informational; optional compare to known latest later (v1: show only) |
| Profile / token | `resolve` has non-empty `qria_…` token | No agent token → open Config |
| Edge / MCP URL | absolute https (or http local), not empty | **Warn** if host is `127.0.0.1` / `localhost` when user expects prod |
| Edge health | HTTP 200 on health endpoint | Edge unreachable |
| MCP tools/list | 200 + JSON tools | 401 Auth failed; other → status + short body |
| Permissions | MCP tools present (retrieve/list_projects/…); **IndexLocal** from agent `permissions` field when Status API available (P2+). Pre-P2: warn that Daily cannot upload; hard-fail on upload 403 with existing copy | Index-here needs Custom **index_local** (Daily cannot upload) |

No silent repair of MCP clients in v1 doctor (display only). Optional: key `m` open Config MCP install (reuse existing).

### 3. Index wizard (`index_tui`)

Flow:

1. **Discover** — reuse `index_here` discovery at cwd + depth (default same as CLI).
2. **Checklist** — toggle roots (`space`); show short name + branch + accept/skip counts (same simplified summary style as 0.2.5+ CLI).
3. **Permission preflight** — credentials + MCP/tools or equivalent: if no IndexLocal, **block upload** with Daily vs Custom copy.
4. **Dry-run summary** — totals; confirm no upload yet.
5. **Upload** — explicit confirm key; call existing upload path; print **job_ids** and “Admin → Needs review → Promote”.
6. Escape cancel anytime before upload commit.

Multi-root: must have at least one selected root; no implicit all-upload without confirm step.

### 4. Remote status (`status_tui` + API)

- Resolve credentials → `GET /api/v1/agent/projects-status`.
- Table/list: `slug`, embed `ready` / `pending` / `failed`, `needs_review_count`.
- Refresh key (`r`).
- If 404: degrade message (old edge); still allow Doctor.

#### API contract

```http
GET /api/v1/agent/projects-status
Authorization: Bearer qria_…
```

Authz: same bar as `GET /api/v1/agent/projects` (ListProjects and/or RetrieveContext; authenticated non-revoked token in home org).

Response `200`:

```json
{
  "embedding_profile_version": "voyage-4-1024-v1",
  "permissions": ["list_projects", "retrieve_context", "index_local"],
  "projects": [
    {
      "id": "uuid",
      "slug": "queria-backend",
      "name": "…",
      "embed": { "ready": 80, "pending": 3, "failed": 0 },
      "needs_review_count": 12
    }
  ]
}
```

`permissions` is a sorted list of agent tool grants on the bearer token (stable string ids matching mint/API: e.g. `index_local`, `retrieve_context`, `list_projects`, `manage_needs_review`). Doctor and Index preflight use this after P2; until then Index relies on upload 403 messaging.

Counts:

- **embed**: chunk (or knowledge) rows for project + current embedding profile_version, grouped by embedding status (ready/pending/failed) — reuse repository logic behind CLI `embeddings status` **without** requiring setup-token admin email join on the laptop.
- **needs_review_count**: count of knowledge items in `needs_review` visible to this agent’s project scope.

Errors: `401` missing/invalid bearer; `403` permission_denied; `500` store errors as infrastructure.

No raw token in response. No SETUP_TOKEN.

### 5. Data flow summary

```text
User runs: queria-cli tui
  → resolve credentials (config.toml / env)
  → Doctor: local + healthz + MCP
  → Index: local git + POST index-local
  → Status: GET projects-status
```

## Error handling principles

- Prefer **one** actionable sentence over stack dumps.
- Reuse improved index-here 403 copy (Custom `index_local`).
- Token never printed; redact like config TUI.
- Network failures name URL host only.
- Fail-open where product already does (e.g. show partial doctor if MCP fails but health ok).

## Testing

| Layer | What |
|---|---|
| Unit CLI | checks pure helpers; hub routing not required in CI |
| Unit CLI | index wizard selection / multi-root confirm messaging |
| Unit/API | `projects-status` 401/403/200 shape with test app |
| Manual TTY | full hub flows against staging/prod edge |
| Regression | existing `cargo test -p queria-cli` and API agent tests green |

CI must not require interactive TUI.

## Phased delivery

| Phase | Deliverable | Acceptance |
|---|---|---|
| **P0** | `tui` hub + Doctor + open Config | Non-TTY error; doctor shows profile/edge/MCP/permission; help on bare CLI |
| **P1** | Index wizard TUI | Multi-root select + preflight block without IndexLocal + upload job_ids |
| **P2** | `GET …/agent/projects-status` + Status screen | Laptop status without SETUP_TOKEN; 404 degrade documented |

Ship can cut multiple CLI patch tags (`cli-v0.3.x`) per phase.

## Success criteria (v1 complete)

Laptop user with only `~/.config/queria/config.toml` + public edge can:

1. `queria-cli tui` → Doctor green (or clear fail copy).
2. Index wizard: understand multi-root + permission, upload when allowed, see job_ids.
3. Status: embed + needs_review counts per project **without** DB env or setup token.

## Docs when shipping

- Onboarding: prefer `queria-cli tui` for laptop path; keep `export QUERIA_*` for CI.
- HANDOFF residual: remove “embeddings status requires server env” as the **only** laptop option once P2 ships; keep server embeddings CLI for maintainers.
- Note: binary on PATH must be current release for hub features.

## Changelog

| Date | Note |
|---|---|
| 2026-07-22 | Initial REFERENCE design (hub via `tui`, doctor, index wizard, agent projects-status) |
