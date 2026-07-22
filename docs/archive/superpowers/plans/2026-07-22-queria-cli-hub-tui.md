# queria-cli hub TUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `queria-cli tui` hub with Doctor, Index-here wizard, and agent-authenticated remote Status (embed + needs_review counts) without laptop `AppConfig` / `QUERIA_SETUP_TOKEN`.

**Architecture:** Clap adds `Tui` only (bare CLI stays help). Hub is ratatui menu reusing `config_tui`. Pure checks + `edge_agent` HTTP (Bearer). Index wizard wraps `index_here` pure discovery/plan and existing upload. New API `GET /api/v1/agent/projects-status` on agent auth path returns permissions + per-project embed/needs_review counts.

**Tech Stack:** Rust workspace, clap, ratatui/crossterm, reqwest, tokio, axum, sqlx/`PgEmbeddingRepository`, existing `credentials` / `index_here` / `doctor_mcp` modules.

**Spec:** [`../specs/2026-07-22-queria-cli-hub-tui-design.md`](../specs/2026-07-22-queria-cli-hub-tui-design.md)

## Global Constraints

- Hub entry is **`queria-cli tui` only**; bare `queria-cli` must remain clap help (no menu).
- Non-TTY `tui` → error exactly: `queria-cli tui needs a TTY`.
- Laptop hub paths **must not** call `AppConfig::from_env()` or Postgres.
- Token never printed; redact via `config::redact_token`.
- Reuse `#[tokio::main]` + `block_in_place` / `Handle::block_on` for async from sync TUI (no nested `Runtime::new` panic).
- Phases: **P0** hub+doctor+config · **P1** index wizard · **P2** API + status; commit after each task; keep `cargo test -p queria-cli` green each phase.
- IndexLocal is **not** inferred from MCP tool names; use agent `permissions` from Status API (P2), else upload 403 copy (P0–P1).
- Modules stay inside `queria-cli` + route in `queria-api` (no new crate).

## File map

| File | Responsibility |
|---|---|
| Create `crates/queria-cli/src/checks.rs` | Pure URL/token/check result types; orchestrate doctor snapshot |
| Create `crates/queria-cli/src/edge_agent.rs` | HTTP: healthz, MCP tools/list, projects-status, thin helpers |
| Create `crates/queria-cli/src/tui_hub.rs` | Hub menu loop |
| Create `crates/queria-cli/src/doctor_tui.rs` | Doctor screen |
| Create `crates/queria-cli/src/index_tui.rs` | Index-here wizard |
| Create `crates/queria-cli/src/status_tui.rs` | Status screen |
| Modify `crates/queria-cli/src/main.rs` | `Command::Tui`, wire hub |
| Modify `crates/queria-cli/src/index_here.rs` | Export upload helper for wizard (`upload_plans` or thin `upload_selected`) |
| Modify `crates/queria-cli/src/doctor_mcp.rs` | Optional: return structured result for reuse (or keep edge_agent separate) |
| Modify `crates/queria-api/src/http/agent_retrieval.rs` | Add `GET /agent/projects-status` (or new `agent_status.rs` mounted same router) |
| Modify `crates/queria-db/src/embedding.rs` | Agent-scoped status counts (include `needs_review` chunks for that project) if current `status_counts` is approved-only |
| Modify `crates/queria-db/src/repositories/projects.rs` | `count_needs_review_for_project` (or equivalent) |
| Modify `docs/runbooks/onboarding.md` + `docs/HANDOFF.md` | Laptop path docs after P2 |

---

### Task 1: Pure checks module (P0)

**Files:**
- Create: `crates/queria-cli/src/checks.rs`
- Modify: `crates/queria-cli/src/main.rs` (add `mod checks;`)
- Test: unit tests inside `checks.rs`

**Interfaces:**
- Consumes: none
- Produces:
  - `pub enum CheckLevel { Pass, Warn, Fail }`
  - `pub struct CheckItem { pub id: &'static str, pub level: CheckLevel, pub detail: String, pub hint: String }`
  - `pub fn is_loopback_host(url: &str) -> bool`
  - `pub fn token_looks_valid(token: Option<&str>) -> bool` (non-empty + starts with `qria_`)
  - `pub fn mcp_tool_names_from_tools_list_body(body: &str) -> Vec<String>` (parse JSON-RPC tools/list; extract `result.tools[].name`)

- [ ] **Step 1: Write failing tests in `checks.rs`**

```rust
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
```

- [ ] **Step 2: Run tests (expect fail)**

```bash
cd queria/backend && cargo test -p queria-cli checks:: -- --nocapture
```

Expected: compile fail / tests not found until module exists.

- [ ] **Step 3: Implement minimal `checks.rs`**

Implement the three pure functions + types above. Parsing: `serde_json::Value`, walk `result.tools` array for `name` strings. Missing/invalid JSON → empty `Vec`.

- [ ] **Step 4: Wire `mod checks;` in `main.rs` and re-run tests**

```bash
cargo test -p queria-cli checks:: -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/queria-cli/src/checks.rs crates/queria-cli/src/main.rs
git commit -m "feat(cli): pure doctor checks helpers for hub TUI"
```

---

### Task 2: edge_agent HTTP helpers (P0)

**Files:**
- Create: `crates/queria-cli/src/edge_agent.rs`
- Modify: `crates/queria-cli/src/main.rs` (`mod edge_agent;`)
- Test: unit tests for path join + JSON tool parse via `checks`; async tests optional with `mockito` only if already in tree (otherwise keep pure URL unit tests)

**Interfaces:**
- Consumes: `credentials::ResolvedCredentials`, `checks::*`
- Produces:
  - `pub async fn edge_health(edge_url: &str) -> Result<(u16, String)>`  
    GET `{edge}/healthz` (trim trailing slash); return status + body text (truncated 200).
  - `pub async fn mcp_tools_list(mcp_url: &str, token: &str) -> Result<(u16, String)>`  
    Same JSON-RPC body as `doctor_mcp::run`, return status + body without requiring success.
  - `pub fn edge_healthz_url(edge_url: &str) -> String`

- [ ] **Step 1: Write failing unit test for URL join**

```rust
#[test]
fn healthz_url_trims_slash() {
    assert_eq!(
        edge_healthz_url("https://queria.fjulian.id/"),
        "https://queria.fjulian.id/healthz"
    );
}
```

- [ ] **Step 2: Implement `edge_agent.rs`** with `reqwest::Client` (30s timeout). Do not use `AppConfig`.

- [ ] **Step 3: Run unit tests**

```bash
cargo test -p queria-cli edge_agent:: -- --nocapture
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/queria-cli/src/edge_agent.rs crates/queria-cli/src/main.rs
git commit -m "feat(cli): edge_agent healthz and MCP tools/list helpers"
```

---

### Task 3: Doctor snapshot orchestration (P0)

**Files:**
- Modify: `crates/queria-cli/src/checks.rs`
- Test: unit tests with injectable outcomes for pure assembly

**Interfaces:**
- Consumes: `ResolvedCredentials`, health `(u16, String)`, mcp `(u16, String)`
- Produces:
  - `pub struct DoctorSnapshot { pub items: Vec<CheckItem>, pub version: String, pub profile: Option<String>, pub edge_url: String, pub mcp_url: String }`
  - `pub fn assemble_doctor_snapshot(…)` pure function assembling ordered check items from inputs (no I/O)

Assembly rules (copy intent from spec):

1. Version: always Pass, detail = `env!("CARGO_PKG_VERSION")`.
2. Token: Fail if `!token_looks_valid`, hint `Open Config and set a qria_… agent token`.
3. MCP/edge URL: Warn if `is_loopback_host`, hint prod URL if user expects public edge; Fail if empty edge/mcp.
4. Health: Pass if status 200; else Fail with host-only detail.
5. MCP: Pass if 200; Fail 401 → `Auth failed — token missing/invalid/revoked`; other status Fail with short body.
6. Permissions (MCP names only pre-P2): Pass if tools list includes `list_projects` or `retrieve_context`; Warn line always: `index-here needs Custom token with index_local (Daily cannot upload)` unless P2 permissions list includes `index_local` (pass that optional list into assembler as `Option<&[String]>`).

- [ ] **Step 1: Failing tests for assemble_doctor_snapshot**

```rust
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
    assert!(snap.items.iter().any(|i| i.id == "token" && matches!(i.level, CheckLevel::Fail)));
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
    assert!(snap.items.iter().any(|i| i.id == "urls" && matches!(i.level, CheckLevel::Warn)));
}
```

- [ ] **Step 2: Implement `assemble_doctor_snapshot`**

- [ ] **Step 3: `cargo test -p queria-cli checks::`** — PASS

- [ ] **Step 4: Commit**

```bash
git add crates/queria-cli/src/checks.rs
git commit -m "feat(cli): assemble_doctor_snapshot for hub doctor"
```

---

### Task 4: Hub TUI + clap `tui` + Doctor/Config screens (P0)

**Files:**
- Create: `crates/queria-cli/src/tui_hub.rs`
- Create: `crates/queria-cli/src/doctor_tui.rs`
- Modify: `crates/queria-cli/src/main.rs` (Command::Tui, mods, match arm)
- Reuse: `config_tui::run_tui`

**Interfaces:**
- Consumes: `checks::assemble_doctor_snapshot`, `edge_agent::*`, `credentials::resolve`, `config::is_tty`
- Produces:
  - `pub fn run_hub(profile: Option<&str>) -> anyhow::Result<()>`
  - `pub async fn collect_doctor_snapshot(profile: Option<&str>) -> anyhow::Result<checks::DoctorSnapshot>`

- [ ] **Step 1: Add clap variant**

In `main.rs` `Command` enum:

```rust
/// Interactive hub TUI: doctor / index / status / config (TTY required).
Tui,
```

Match:

```rust
Command::Tui => {
    tui_hub::run_hub(profile.as_deref())?;
    Ok(())
}
```

Add `mod tui_hub; mod doctor_tui;`.

- [ ] **Step 2: Implement `collect_doctor_snapshot`**

```rust
pub async fn collect_doctor_snapshot(profile: Option<&str>) -> anyhow::Result<checks::DoctorSnapshot> {
    let creds = credentials::resolve(credentials::ResolveOpts {
        profile: profile.map(|s| s.to_owned()),
        require_token: false,
        ..Default::default()
    })?;
    let health = edge_agent::edge_health(&creds.edge_url).await.map_err(|e| e.to_string());
    let mcp = match creds.agent_token.as_deref() {
        Some(t) if !t.is_empty() => edge_agent::mcp_tools_list(&creds.mcp_url, t).await.map_err(|e| e.to_string()),
        _ => Err("no token".into()),
    };
    // map Result<String> into Result<(u16,String)> for assemble — keep types consistent in implementation
    Ok(checks::assemble_doctor_snapshot(
        env!("CARGO_PKG_VERSION"),
        creds.profile.as_deref(), // if field exists; else pass None
        &creds.edge_url,
        &creds.mcp_url,
        creds.agent_token.as_deref(),
        /* health and mcp Results — adapt signatures so assemble accepts Result<(u16,String), String> */,
        None, // permissions from status API: P2
    ))
}
```

Note: align field names with actual `ResolvedCredentials` (`profile` may be present). Adjust so tests still compile.

- [ ] **Step 3: Implement `doctor_tui::run`**

TTY raw mode (copy pattern from `config_tui`): show `DoctorSnapshot` as List of `Pass|Warn|Fail` + detail + hint. Keys: `r` re-run collect (via `block_in_place` + handle.block_on), `Esc`/`q` return to hub.

Block_on helper: same as config_tui MCP install (`block_in_place` + `Handle::try_current()`).

- [ ] **Step 4: Implement `tui_hub::run_hub`**

- If `!config::is_tty()` → `bail!("queria-cli tui needs a TTY");`
- Menu items: Doctor, Index, Status, Config, Quit.
- Keys: `d/i/s/c/q`, arrows, Enter.
- Index/Status before their tasks: stub Message screen `"Index wizard ships in P1"` / `"Status ships in P2"`.
- Config: `config_tui::run_tui(profile)` then redraw hub.

- [ ] **Step 5: Manual smoke (no CI TUI)**

```bash
cargo build -p queria-cli
./target/debug/queria-cli          # help, no hub
./target/debug/queria-cli tui </dev/null   # expect: needs a TTY
cargo test -p queria-cli
```

- [ ] **Step 6: Commit**

```bash
git add crates/queria-cli/src/main.rs crates/queria-cli/src/tui_hub.rs crates/queria-cli/src/doctor_tui.rs
git commit -m "feat(cli): queria-cli tui hub with doctor and config"
```

---

### Task 5: Export index-here upload for wizard (P1)

**Files:**
- Modify: `crates/queria-cli/src/index_here.rs`
- Test: existing unit tests remain green; add thin test for filter-selected plans pure helper

**Interfaces:**
- Produces:
  - `pub fn filter_plans_by_paths(plans: &[RootFilePlan], selected_paths: &[PathBuf]) -> Vec<RootFilePlan>`
  - `pub async fn upload_plans_public(endpoint: &str, token: &str, plans: &[RootFilePlan]) -> Result<()>`  
    either rename `upload_plans` to `pub` or wrap it (keep current private name if preferred: `pub async fn upload_selected_plans` calling private `upload_plans`).

- [ ] **Step 1: Test filter**

```rust
#[test]
fn filter_plans_by_paths_keeps_matching() {
    // build two RootFilePlan with distinct root.path; select one path; len==1
}
```

- [ ] **Step 2: Implement filter + publish upload**

- [ ] **Step 3: `cargo test -p queria-cli index_here::`** PASS

- [ ] **Step 4: Commit**

```bash
git add crates/queria-cli/src/index_here.rs
git commit -m "feat(cli): export index-here plan filter and upload for TUI"
```

---

### Task 6: Index wizard TUI (P1)

**Files:**
- Create: `crates/queria-cli/src/index_tui.rs`
- Modify: `crates/queria-cli/src/tui_hub.rs` (wire `i` → index_tui)
- Modify: `crates/queria-cli/src/main.rs` (`mod index_tui`)

**Interfaces:**
- Consumes: `index_here::discover_git_roots`, `plan_root_files`, `DEFAULT_DEPTH`, `filter_plans_by_paths`, `upload_*`, `credentials::resolve`
- Produces: `pub fn run_index_wizard(profile: Option<&str>) -> anyhow::Result<()>`

Wizard screens / states:

1. Discover (sync, may take time — show “scanning…” then list).
2. Checklist: toggle Space; show `name (branch) +accept −skip`; all selected by default.
3. Preflight text: always show Daily/Custom IndexLocal warning; if later permissions available and `index_local` present, show Pass.
4. Dry-run summary totals for selected only.
5. Confirm upload key (`u` or Enter on confirm screen) → `upload_*` with selected plans as if `--yes` for multi; require `require_token: true`.
6. Result: print job_ids from response (may need `upload_plans` to return `IndexLocalResponse` — if currently only eprintln, change return type to `Result<IndexLocalResponse>` or return job_ids string). **Required:** surface job_ids in TUI message + “Admin → Needs review → Promote”.
7. Esc: cancel.

- [ ] **Step 1: Prefer changing upload to return job_ids**

```rust
pub async fn upload_plans(...) -> Result<Vec<String>> // or IndexLocalResponse
```

Update CLI `run()` to ignore detailed return or print same as now.

- [ ] **Step 2: Implement wizard TUI** following `config_tui` event loop style.

- [ ] **Step 3: Unit test pure selection helper if any additional**

- [ ] **Step 4: `cargo test -p queria-cli` + `cargo clippy -p queria-cli --all-targets -- -D warnings`**

- [ ] **Step 5: Commit**

```bash
git add crates/queria-cli/src/index_tui.rs crates/queria-cli/src/index_here.rs crates/queria-cli/src/tui_hub.rs crates/queria-cli/src/main.rs
git commit -m "feat(cli): index-here wizard TUI in hub"
```

---

### Task 7: DB helpers for agent project status counts (P2)

**Files:**
- Modify: `crates/queria-db/src/embedding.rs` and/or `repositories/projects.rs`
- Test: sqlx unit only if fixtures exist; otherwise repository method with documented SQL + compile

**Interfaces:**
- Produces:
  - Prefer new method that counts embed statuses for **one project** including knowledge with `status in ('approved','needs_review')` and `k.project_id = $1` (not global bleed unless product wants it — **scope project only for agent status**):

```rust
// e.g. on PgEmbeddingRepository
pub async fn status_counts_for_project_items(
    &self,
    project_id: ProjectId,
    profile_version: &str,
) -> QueriaResult<EmbeddingStatusCounts>
```

SQL sketch:

```sql
select
  count(*) filter (where c.embedding_status = 'pending') as pending,
  count(*) filter (where c.embedding_status = 'processing') as processing,
  count(*) filter (where c.embedding_status = 'ready' and c.embedding_profile_version = $2) as ready,
  count(*) filter (where c.embedding_status = 'failed') as failed,
  count(*) filter (where c.embedding_status = 'stale' or c.embedding_profile_version <> $2) as stale
from chunk c
join knowledge_item k on k.id = c.knowledge_item_id
where k.project_id = $1
  and k.status in ('approved', 'needs_review')
```

- Produces needs_review count:

```rust
// PgProjectRepository or embedding repo
pub async fn count_needs_review_items(&self, project_id: ProjectId) -> QueriaResult<i64>
```

```sql
select count(*)::bigint from knowledge_item
where project_id = $1 and status = 'needs_review'
```

- [ ] **Step 1: Add methods + any unit test already used for similar SQL**

- [ ] **Step 2: `cargo test -p queria-db` if feasible; else `cargo check -p queria-db`**

- [ ] **Step 3: Commit**

```bash
git add crates/queria-db/src/embedding.rs crates/queria-db/src/repositories/projects.rs
git commit -m "feat(db): project embed and needs_review counts for agent status"
```

---

### Task 8: API `GET /api/v1/agent/projects-status` (P2)

**Files:**
- Modify: `crates/queria-api/src/http/agent_retrieval.rs` (preferred: keep with agent routes)
- Test: extend `agent_retrieval.rs` tests (bearer required oneshot like existing)

**Interfaces:**
- Route: `.route("/agent/projects-status", get(agent_projects_status))`
- Authz: identical to `agent_list_projects` (ListProjects **or** RetrieveContext).
- Response body:

```json
{
  "embedding_profile_version": "<from state.config.embedding.profile_version>",
  "permissions": ["index_local", "list_projects", "retrieve_context"],
  "projects": [
    {
      "id": "<uuid>",
      "slug": "<slug>",
      "name": "<name>",
      "embed": { "ready": 0, "pending": 0, "failed": 0 },
      "needs_review_count": 0
    }
  ]
}
```

Notes:

- `permissions`: serialize `agent.permissions.tools` via serde snake_case **sorted** alphabetically for stability.
- `embed.ready|pending|failed`: map from `EmbeddingStatusCounts` (ignore processing/stale in JSON or fold processing into pending — **fold `processing` into `pending`** for laptop UX; leave stale out of the three counters or document as pending-ish; use: pending = pending+processing+stale, ready, failed).
- Projects list: same set as `list_projects_for_agent`.

- [ ] **Step 1: Test missing bearer → 401**

```rust
#[tokio::test]
async fn agent_projects_status_requires_bearer() {
    let app = build_app(AppConfig::default_local());
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/api/v1/agent/projects-status")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
```

- [ ] **Step 2: Implement handler**

- [ ] **Step 3: `cargo test -p queria-api agent_projects_status`**

- [ ] **Step 4: Commit**

```bash
git add crates/queria-api/src/http/agent_retrieval.rs crates/queria-db/src/embedding.rs
git commit -m "feat(api): GET /api/v1/agent/projects-status for laptop CLI"
```

---

### Task 9: edge_agent fetch + Status TUI + Doctor permissions hookup (P2)

**Files:**
- Modify: `crates/queria-cli/src/edge_agent.rs`
- Create: `crates/queria-cli/src/status_tui.rs`
- Modify: `crates/queria-cli/src/tui_hub.rs`, `doctor_tui.rs` / `checks.rs`, `main.rs`
- Modify: `crates/queria-cli/src/index_tui.rs` preflight to use `permissions` if status fetch succeeds

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Deserialize)]
pub struct ProjectsStatusResponse {
    pub embedding_profile_version: String,
    pub permissions: Vec<String>,
    pub projects: Vec<ProjectStatusRow>,
}

#[derive(Debug, Deserialize)]
pub struct ProjectStatusRow {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub embed: EmbedCounts,
    pub needs_review_count: i64,
}

#[derive(Debug, Deserialize)]
pub struct EmbedCounts { pub ready: i64, pub pending: i64, pub failed: i64 }

pub async fn fetch_projects_status(edge_url: &str, token: &str) -> Result<(u16, ProjectsStatusResponse)>
```

- 404 → `Err` or status code so Status screen can show redeploy message.
- Status TUI: list rows `slug  ready/pending/failed  NR=n`; key `r` refresh; Esc back.
- Doctor: after status fetch OK, pass `permissions` into `assemble_doctor_snapshot` so IndexLocal becomes Pass when `permissions` contains `index_local`.
- Index preflight: if permissions present and missing `index_local`, **block** upload with Custom token copy; if status 404, soft Daily warn but allow attempt (403 still handled).

- [ ] **Step 1: Unit test deserialize sample JSON**

- [ ] **Step 2: Implement fetch + status_tui + wire hub `s`**

- [ ] **Step 3: Full `cargo test -p queria-cli` and `cargo clippy -p queria-cli --all-targets -- -D warnings`**

- [ ] **Step 4: Commit**

```bash
git add crates/queria-cli/src/edge_agent.rs crates/queria-cli/src/status_tui.rs crates/queria-cli/src/tui_hub.rs crates/queria-cli/src/doctor_tui.rs crates/queria-cli/src/checks.rs crates/queria-cli/src/index_tui.rs crates/queria-cli/src/main.rs
git commit -m "feat(cli): status TUI and permissions via agent projects-status"
```

---

### Task 10: Docs + version bump residual (P2 ship)

**Files:**
- Modify: `docs/runbooks/onboarding.md` (laptop section: `queria-cli tui`)
- Modify: `docs/HANDOFF.md` residual: hub TUI shipped; embeddings status still server-maintainer; laptop status via tui Status
- Modify: `crates/queria-cli/Cargo.toml` version bump when cutting release (e.g. `0.3.0`)

- [ ] **Step 1: Edit onboarding + HANDOFF** with exact commands:

```bash
queria-cli tui          # Doctor / Index / Status / Config
queria-cli doctor mcp   # still valid non-TUI
# embeddings status remains server/DB only for maintainers
```

- [ ] **Step 2: Commit docs**

```bash
git add docs/runbooks/onboarding.md docs/HANDOFF.md
git commit -m "docs: laptop hub TUI path for doctor index status"
```

- [ ] **Step 3 (release, optional in same PR):** bump package, tag `cli-v0.3.0` after merge per existing release workflow.

---

## Spec coverage checklist

| Spec requirement | Task |
|---|---|
| `queria-cli tui` hub, bare help | Task 4 |
| Doctor friction checks | Tasks 1–4 |
| Config from hub | Task 4 |
| Index wizard full flow | Tasks 5–6 |
| No AppConfig on laptop hub path | Tasks 2–4, 6, 9 |
| `GET /agent/projects-status` + permissions | Tasks 7–8 |
| Status TUI + 404 degrade | Task 9 |
| IndexLocal not from MCP tool names | Tasks 3, 6, 9 |
| Docs | Task 10 |
| Phased P0/P1/P2 | Tasks 1–4 / 5–6 / 7–10 |
| Non-TTY error copy | Task 4 |
| Tests unit/API, no CI interactive TUI | All tasks |

## Placeholder / consistency notes

- `ResolvedCredentials.profile` field: use existing struct fields (`profile` is present as `Option<String>` in credentials.rs per design era); if compile fails, pass `None` for profile label.
- Fold embed `processing`+`stale` into `pending` for JSON three-counter UX (explicit in Task 8).
- Upload must return job_ids for TUI (Task 6).
