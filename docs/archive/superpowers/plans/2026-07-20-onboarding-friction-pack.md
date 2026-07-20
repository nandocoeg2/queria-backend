# Onboarding Friction Pack Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut Admin-login → first agent retrieve_context drop-off with public edge URLs, Daily token mint, connect panel, dashboard checklist, and docs laptop path — without dual-lane changes.

**Architecture:** Phased H→T→C→D→F. Prefer existing `AppConfig.public_base_url` in setup `request_base`. Admin always POSTs explicit Daily/Custom tools. Lazy connect panel once after mint. Dashboard adds `agent_token_count` and 3-step checklist. Docs only for index-here first-win.

**Tech Stack:** Rust (queria-api, queria-core, queria-db), Astro Admin SSR, runbooks Markdown.

## Global Constraints

- Lazy ship only: Daily + Custom modes (no Read-only / Read+propose / Local index presets).
- Connect panel required: raw token + copy, tools chips, env export, agent-setup link. No Admin one-liners/paste.
- Checklist 3 steps: project_count, chunk_counts.ready, agent_token_count.
- API omit-`tools` stays `default_agent_tools()` propose-only.
- Admin Daily POSTs tools including `index_memory`; never privileged tools in Daily.
- Expiry form default: `no_expire`.
- Privileged Custom tools default off with warnings.
- Dual-lane / demo corpus / self-serve mint out of scope.
- Edge truth: public base config production `https://queria.fjulian.id`.

---

**Spec:** [`../specs/2026-07-20-onboarding-friction-pack-design.md`](../specs/2026-07-20-onboarding-friction-pack-design.md)

**Milestones:** `ob-base` (H), `ob-tokens` (T+C), `ob-dashboard-docs` (D+F)

---

## File map

| Area | Paths | Change |
|---|---|---|
| H | `crates/queria-api/src/http/agent_setup.rs` | `request_base` prefers `public_base_url`; wire 3 handlers |
| H notes | `docs/runbooks/onboarding.md`, `docs/HANDOFF.md` (brief) | Document `QUERIA_PUBLIC_BASE_URL` |
| T core | `crates/queria-core/src/auth/agent_token.rs` | Add `daily_agent_tools()` + unit tests |
| T Admin | `admin/src/pages/tokens/index.astro` | Daily\|Custom mode, always POST `tools`, expiry default `no_expire` |
| C | `admin/src/pages/tokens/index.astro` | Once-only connect panel after mint |
| D db | `crates/queria-db/src/admin_queries.rs` | `agent_token_count` on summary + SQL |
| D api | `crates/queria-api/src/http/dashboard.rs` | Map + serialize field |
| D ui | `admin/src/pages/dashboard.astro` | “Get ready for agents” checklist |
| F | `docs/runbooks/onboarding.md` | Laptop first-win Custom + `index_local` block |

No new crates, no migration, no API route additions (mint already accepts `tools`; summary extends JSON).

---

### Task 1: H — `request_base` prefers public base

**Files:**
- Modify: `crates/queria-api/src/http/agent_setup.rs`
- Touch (deploy note only, if not already set): `.env.example` already has `QUERIA_PUBLIC_BASE_URL` — leave as-is unless blank

**Interfaces:**
- Consumes: `AppConfig.public_base_url: String` (already loaded from `QUERIA_PUBLIC_BASE_URL`, default `http://127.0.0.1:17674`)
- Produces: `fn request_base(public_base_url: &str, headers: &HeaderMap) -> String`
- Call sites (today `request_base(&headers)`): `agent_setup_docs`, `mcp_snippet`, `hooks_snippet` — all need `State(state): State<ApiState>` already or add it

**Resolution order (normative):**
1. Non-empty `public_base_url` after trim + strip trailing `/`
2. Else existing header fallback (`X-Forwarded-Proto` + `X-Forwarded-Host` / `Host`, default proto `http`, host `127.0.0.1:17674`)

- [ ] **Step 1: Write unit tests in `#[cfg(test)] mod tests`**

Add pure tests (no HTTP app) near top of tests module:

```rust
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
```

- [ ] **Step 2: Run tests — expect FAIL**

Run: `cargo test -p queria-api request_base -- --nocapture`  
Expected: compile fail or FAIL (function signature not updated / tests not linked)

- [ ] **Step 3: Implement + wire handlers**

Replace:

```rust
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
```

With:

```rust
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
```

Wire each call site. Pattern for `agent_setup_docs` (and same for `mcp_snippet`, `hooks_snippet`):

```rust
async fn agent_setup_docs(
    headers: HeaderMap,
    State(state): State<ApiState>,
) -> Response {
    let base = request_base(&state.config.public_base_url, &headers);
    // unchanged body...
}
```

If a handler already has `State(_state)`, change to `State(state)` and use `state.config.public_base_url`.

**Note:** `AppConfig::default_local()` sets a non-empty public base (`http://127.0.0.1:17674`). Existing integration tests that assert host header wins will **flip**: base becomes config value. Update assertions accordingly:
- Tests that only need local host → keep (config default matches).
- `setup_docs_alias_matches` currently expects `http://example.test/mcp` from Host header — with default config it will prefer config. Fix by either:
  - building app with empty public base for that test, or
  - asserting the configured base (prefer: construct `AppConfig` with `public_base_url: String::new()` for header-fallback cases; keep one test that default_local config wins).

Example config override for header-fallback integration test:

```rust
let mut config = AppConfig::default_local();
config.public_base_url = String::new();
let app = build_app(config);
// Host header wins
```

Example config-wins integration test:

```rust
let mut config = AppConfig::default_local();
config.public_base_url = "https://queria.fjulian.id/".into();
let app = build_app(config);
// response body contains https://queria.fjulian.id/mcp not evil Host
```

- [ ] **Step 4: Run tests — PASS**

Run: `cargo test -p queria-api request_base -- --nocapture`  
Also: `cargo test -p queria-api --lib agent_setup -- --nocapture`  
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add crates/queria-api/src/http/agent_setup.rs
git commit -m "$(cat <<'EOF'
feat(api): prefer QUERIA_PUBLIC_BASE_URL for agent setup base

Co-authored-by: factory-droid[bot] <138933559+factory-droid[bot]@users.noreply.github.com>
EOF
)"
```

---

### Task 2: T core — `daily_agent_tools()`

**Files:**
- Modify: `crates/queria-core/src/auth/agent_token.rs`

**Interfaces:**
- Consumes: `AgentToolPermission` enum in `crates/queria-core/src/auth/permissions.rs`
- Produces: `pub fn daily_agent_tools() -> Vec<AgentToolPermission>`
- Leaves untouched: `default_agent_tools()` (propose-only; no IndexMemory)

- [ ] **Step 1: Write failing tests**

In existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn daily_agent_tools_includes_index_memory_not_privileged() {
    let tools = daily_agent_tools();
    assert!(tools.contains(&AgentToolPermission::RetrieveContext));
    assert!(tools.contains(&AgentToolPermission::SearchKnowledge));
    assert!(tools.contains(&AgentToolPermission::ProposeMemory));
    assert!(tools.contains(&AgentToolPermission::ListProjects));
    assert!(tools.contains(&AgentToolPermission::GetSource));
    assert!(
        tools.contains(&AgentToolPermission::IndexMemory),
        "Daily must grant index_memory"
    );
    assert!(
        !tools.contains(&AgentToolPermission::IndexLocal),
        "Daily must not grant index_local"
    );
    assert!(
        !tools.contains(&AgentToolPermission::ManageNeedsReview),
        "Daily must not grant manage_needs_review"
    );
}

#[test]
fn default_and_daily_tools_differ_only_by_index_memory() {
    let default = default_agent_tools();
    let daily = daily_agent_tools();
    assert!(!default.contains(&AgentToolPermission::IndexMemory));
    assert!(daily.contains(&AgentToolPermission::IndexMemory));
    assert_eq!(default.len() + 1, daily.len());
}
```

Keep existing `default_agent_tools_remains_propose_only_without_index_memory` green.

- [ ] **Step 2: Run — FAIL**

Run: `cargo test -p queria-core daily_agent_tools -- --nocapture`  
Expected: FAIL (undefined `daily_agent_tools`)

- [ ] **Step 3: Implement next to `default_agent_tools`**

```rust
/// Daily agent tool set: propose path + project-scoped scratch `index_memory`.
/// No privileged tools (`index_local`, `manage_needs_review`).
pub fn daily_agent_tools() -> Vec<AgentToolPermission> {
    vec![
        AgentToolPermission::RetrieveContext,
        AgentToolPermission::SearchKnowledge,
        AgentToolPermission::ProposeMemory,
        AgentToolPermission::ListProjects,
        AgentToolPermission::GetSource,
        AgentToolPermission::IndexMemory,
    ]
}
```

Do **not** change `tokens.rs` omit-tools path — it must keep `unwrap_or_else(default_agent_tools)`.

- [ ] **Step 4: Run — PASS**

Run: `cargo test -p queria-core --lib agent_token -- --nocapture`  
Expected: PASS (default + daily tests)

- [ ] **Step 5: Commit**

```bash
git add crates/queria-core/src/auth/agent_token.rs
git commit -m "$(cat <<'EOF'
feat(core): add daily_agent_tools with index_memory

Co-authored-by: factory-droid[bot] <138933559+factory-droid[bot]@users.noreply.github.com>
EOF
)"
```

---

### Task 3: T Admin — Daily | Custom mint UI

**Files:**
- Modify: `admin/src/pages/tokens/index.astro`

**Interfaces:**
- Consumes: `POST /api/v1/agent-tokens` with `tools?: AgentToolPermission[]` (already supported in `CreateAgentTokenRequest`)
- Produces: Admin always POSTs explicit `tools`; never relies on server omit-default for Daily
- Tool string values (snake_case):  
  `retrieve_context`, `search_knowledge`, `propose_memory`, `list_projects`, `get_source`, `index_memory`, `index_local`, `manage_needs_review`

**Current gap:** form POSTs name/slugs/global/expires only; API falls back to `default_agent_tools()` (no `index_memory`). Default expiry UI is `7_days`.

- [ ] **Step 1: Capture granted tools + set defaults in POST handler**

Extend frontmatter after form parse:

```ts
const DAILY_TOOLS = [
  'retrieve_context',
  'search_knowledge',
  'propose_memory',
  'list_projects',
  'get_source',
  'index_memory',
] as const;

const ALL_TOOLS = [
  ...DAILY_TOOLS,
  'index_local',
  'manage_needs_review',
] as const;

let generatedTools: string[] = [];
let generatedProjectSlugs: string[] = [];

// inside action === 'generate':
const mode = data.get('mode')?.toString() || 'daily';
const expiresIn = data.get('expires_in')?.toString() || 'no_expire';

let tools: string[];
if (mode === 'custom') {
  tools = data
    .getAll('tools')
    .map((v) => v.toString())
    .filter((t) => (ALL_TOOLS as readonly string[]).includes(t));
  if (tools.length === 0) {
    message = 'Select at least one tool for Custom mode.';
    isError = true;
  }
} else {
  // daily — always explicit full set
  tools = [...DAILY_TOOLS];
}

// when not isError:
body: JSON.stringify({
  name,
  project_slugs: selectedSlugs,
  allow_global_knowledge: allowGlobal,
  expires_in: expiresIn,
  tools,
}),
// on success:
generatedToken = result.token;
generatedTools = Array.isArray(result.agent_token?.tools)
  ? result.agent_token.tools
  : tools;
generatedProjectSlugs = selectedSlugs;
```

- [ ] **Step 2: Form UI — mode + Custom checkboxes + expiry default**

Inside generate form (after projects / before expires or after):

```astro
<div class="form-group">
  <label>Mode</label>
  <div class="radio-list">
    <label class="checkbox-row single">
      <input type="radio" name="mode" value="daily" checked />
      <span class="checkbox-label">Daily agent (recommended)</span>
    </label>
    <label class="checkbox-row single">
      <input type="radio" name="mode" value="custom" id="mode-custom" />
      <span class="checkbox-label">Custom tools</span>
    </label>
  </div>
  <p class="field-hint">Daily includes retrieve, search, propose, list_projects, get_source, and index_memory. Privileged tools only via Custom.</p>
</div>

<div class="form-group custom-tools" id="custom-tools" hidden>
  <label>Tools</label>
  {['retrieve_context','search_knowledge','propose_memory','list_projects','get_source','index_memory'].map((t) => (
    <label class="checkbox-row">
      <input type="checkbox" name="tools" value={t} checked />
      <span class="checkbox-label"><code>{t}</code></span>
    </label>
  ))}
  <label class="checkbox-row">
    <input type="checkbox" name="tools" value="index_local" />
    <span class="checkbox-label">
      <code>index_local</code>
      <span class="field-hint warn">Uploads land in Needs review only.</span>
    </span>
  </label>
  <label class="checkbox-row">
    <input type="checkbox" name="tools" value="manage_needs_review" />
    <span class="checkbox-label">
      <code>manage_needs_review</code>
      <span class="field-hint warn">Can promote/reject Needs review items.</span>
    </span>
  </label>
</div>

<script is:inline>
  (function () {
    const custom = document.getElementById('mode-custom');
    const box = document.getElementById('custom-tools');
    if (!custom || !box) return;
    const sync = () => {
      const isCustom = document.querySelector('input[name="mode"]:checked')?.value === 'custom';
      box.hidden = !isCustom;
    };
    document.querySelectorAll('input[name="mode"]').forEach((el) => el.addEventListener('change', sync));
    sync();
  })();
</script>
```

Expiry select — default `no_expire`:

```astro
<select name="expires_in" id="expires_in" class="form-control">
  <option value="1_day">1 day</option>
  <option value="7_days">7 days</option>
  <option value="30_days">30 days</option>
  <option value="1_year">1 year</option>
  <option value="no_expire" selected>No expiry</option>
</select>
```

Minimal CSS for warn hint if missing:

```css
.field-hint.warn { color: var(--color-warning, #f0a000); display: block; margin-top: 0.25rem; }
```

- [ ] **Step 3: Manual / smoke check (no cargo)**

1. Open `/admin/tokens`, leave Daily, submit → inspect network POST body: `"tools":[... includes index_memory, excludes index_local ...]`
2. Custom: uncheck all tools → error; check only retrieve → mint succeeds with that list
3. Privileged checkboxes default off
4. Expiry default No expiry

Optional: if Admin has Playwright smoke, skip for this plan (manual OK).

- [ ] **Step 4: Commit**

```bash
git add admin/src/pages/tokens/index.astro
git commit -m "$(cat <<'EOF'
feat(admin): Daily/Custom token modes with explicit tools

Co-authored-by: factory-droid[bot] <138933559+factory-droid[bot]@users.noreply.github.com>
EOF
)"
```

---

### Task 4: C — Once-only connect panel

**Files:**
- Modify: `admin/src/pages/tokens/index.astro` (same page as Task 3; ship after T UI exists)

**Interfaces:**
- Consumes: `generatedToken`, `generatedTools`, `generatedProjectSlugs` from mint response path
- EDGE base: Admin SSR cannot call Rust `request_base`. Resolve:

```ts
const publicBase = (
  import.meta.env.QUERIA_PUBLIC_BASE_URL ||
  process.env.QUERIA_PUBLIC_BASE_URL ||
  // request host fallback for local SSR
  (() => {
    try {
      const u = new URL(Astro.request.url);
      // Prefer configured edge; fall back to localhost edge port family
      return `${u.protocol}//${u.hostname}:17674`;
    } catch {
      return 'http://127.0.0.1:17674';
    }
  })()
).replace(/\/$/, '');
```

Prefer env when set (production/deploy: `QUERIA_PUBLIC_BASE_URL=https://queria.fjulian.id`). Document in Task 6.

**Panel contents (required only when `generatedToken` is non-empty this response):**
1. Raw token + copy
2. Tools chips from `generatedTools`
3. Env export block
4. Link to `{publicBase}/api/v1/docs/agent-setup`

No client one-liners, no paste-prompt blocks, no secret download.

- [ ] **Step 1: Replace bare token-reveal box with connect panel**

```astro
{generatedToken && (
  <div class="card token-reveal-box connect-panel">
    <div class="reveal-header">
      <h3>Connect this agent</h3>
      <p class="warning-text">
        This token is shown once. It cannot be recovered after you navigate away or refresh.
        Never commit it.
      </p>
    </div>
    <div class="reveal-body">
      <label class="field-hint">Raw token</label>
      <code class="raw-token" id="raw-token">{generatedToken}</code>
      <button
        type="button"
        class="btn btn-primary btn-sm"
        onclick={`navigator.clipboard.writeText(document.getElementById('raw-token').textContent || '');`}
      >
        Copy token
      </button>
    </div>

    <div class="connect-section">
      <label class="field-hint">Granted tools</label>
      <div class="tools-chips">
        {generatedTools.map((t) => (
          <code class="tool-chip">{t}</code>
        ))}
      </div>
    </div>

    <div class="connect-section">
      <label class="field-hint">Env export</label>
      <pre class="env-export" id="env-export">{`export QUERIA_AGENT_TOKEN='${generatedToken}'
export QUERIA_EDGE_URL='${publicBase}'
export QUERIA_MCP_URL='${publicBase}/mcp'${
  generatedProjectSlugs.length === 1
    ? `\nexport QUERIA_PROJECT_SLUG='${generatedProjectSlugs[0]}'`
    : ''
}`}</pre>
      <button
        type="button"
        class="btn btn-secondary btn-sm"
        onclick={`navigator.clipboard.writeText(document.getElementById('env-export').textContent || '');`}
      >
        Copy env
      </button>
    </div>

    <div class="connect-section">
      <a class="stat-link" href={`${publicBase}/api/v1/docs/agent-setup`}>
        Agent setup docs →
      </a>
      <p class="field-hint">Use setup / mcp-snippet endpoints for client commands. No paste required here.</p>
    </div>
  </div>
)}
```

Add compact styles:

```css
.connect-panel .connect-section { margin-top: var(--spacing-md); }
.tools-chips { display: flex; flex-wrap: wrap; gap: 0.35rem; }
.tool-chip {
  background: var(--surface-card, #111);
  border: 1px solid var(--border-subtle, #222233);
  padding: 0.15rem 0.45rem;
  border-radius: 4px;
  font-size: 0.8rem;
}
.env-export {
  background: #0a0a0a;
  border: 1px solid #222233;
  padding: 0.75rem;
  overflow-x: auto;
  font-size: 0.8rem;
}
```

- [ ] **Step 2: Verify once-only behavior**

1. Generate Daily → panel shows token, chips include `index_memory`, env has EDGE/MCP, docs link.
2. Single project selected → `QUERIA_PROJECT_SLUG` line present; multi/none → absent.
3. Reload `/admin/tokens` → panel gone; list shows prefix only.
4. No privileged chip on Daily.

- [ ] **Step 3: Commit**

```bash
git add admin/src/pages/tokens/index.astro
git commit -m "$(cat <<'EOF'
feat(admin): once-only connect panel after token mint

Co-authored-by: factory-droid[bot] <138933559+factory-droid[bot]@users.noreply.github.com>
EOF
)"
```

---

### Task 5: D — `agent_token_count` + dashboard checklist

**Files:**
- Modify: `crates/queria-db/src/admin_queries.rs`
- Modify: `crates/queria-api/src/http/dashboard.rs`
- Modify: `admin/src/pages/dashboard.astro`
- `admin/src/lib/api.ts` — `getDashboardSummary` already returns full JSON; no change if typings are untyped

**Interfaces:**
- Produces: `DashboardSummaryRecord.agent_token_count: i64`
- API JSON: `"agent_token_count": <i64>`
- Count filter: same membership join as `list_agent_tokens`, plus **active only** (`revoked_at is null`). Spec: match list visibility (home org via membership) for accurate checklist step 3.

Note: `list_agent_tokens` currently returns revoked rows too; checklist counts **active** only. Document that intentionally so step 3 goes green only when a usable token exists.

- [ ] **Step 1: Extend record + SQL**

In `DashboardSummaryRecord` add:

```rust
pub agent_token_count: i64,
```

In `get_dashboard_summary` stats query, add a subquery:

```sql
(select count(*)
 from agent_token at
 join org_membership m on m.organization_id = at.organization_id
 where m.user_id = $1
   and at.revoked_at is null) as agent_token_count,
```

Map field:

```rust
agent_token_count: stats
    .try_get("agent_token_count")
    .map_err(to_infrastructure_error)?,
```

If any unit/fixture constructs `DashboardSummaryRecord` manually, add the field.

- [ ] **Step 2: Map through API**

`DashboardSummaryResponse`:

```rust
struct DashboardSummaryResponse {
    project_count: i64,
    source_count: i64,
    pending_approvals_count: i64,
    agent_token_count: i64,
    chunk_counts: ChunkCountsByEmbeddingState,
    failed_jobs_count: i64,
    latest_ingestion: Option<IngestionJobSummary>,
    latest_evaluation: Option<EvaluationSummary>,
}
```

`From` impl:

```rust
agent_token_count: value.agent_token_count,
// ...rest unchanged
```

- [ ] **Step 3: Compile check**

Run: `cargo test -p queria-db --lib -- --nocapture` (or at least `cargo check -p queria-db -p queria-api`)  
Expected: PASS compile. Prefer any existing dashboard repository tests updated if present; otherwise manual:

```bash
# With running stack + admin session cookie:
curl -sS -H "Cookie: …" http://127.0.0.1:17674/api/v1/dashboard/summary | jq .agent_token_count
```

- [ ] **Step 4: Checklist UI on dashboard**

In `dashboard.astro` frontmatter after destructure:

```ts
const {
  project_count,
  source_count,
  pending_approvals_count,
  agent_token_count = 0,
  chunk_counts,
  failed_jobs_count,
  latest_ingestion,
  latest_evaluation,
} = summary;

const readyChunks = chunk_counts?.ready ?? 0;
const checklist = [
  {
    id: 'project',
    label: 'Create a project',
    done: project_count > 0,
    href: '/admin/projects',
    cta: 'Projects',
  },
  {
    id: 'ready',
    label: 'Have ready knowledge chunks',
    done: readyChunks > 0,
    href: '/admin/sources',
    cta: 'Sources / jobs',
    note: 'Register a Git source and embed, or index-here + promote Needs review first.',
  },
  {
    id: 'token',
    label: 'Mint an agent token',
    done: (agent_token_count ?? 0) > 0,
    href: '/admin/tokens',
    cta: 'Tokens',
  },
];
const checklistComplete = checklist.every((s) => s.done);
```

Markup (top of layout content, before stat grid):

```astro
{!checklistComplete && (
  <div class="card checklist-card">
    <div class="panel-header">
      <h2>Get ready for agents</h2>
      <p class="muted-text">Finish these steps before your first retrieve_context.</p>
    </div>
    <ol class="ready-checklist">
      {checklist.map((step, i) => (
        <li class={step.done ? 'done' : 'todo'}>
          <span class="step-mark">{step.done ? '✓' : String(i + 1)}</span>
          <span class="step-body">
            <strong>{step.label}</strong>
            {step.note && !step.done && <span class="field-hint">{step.note}</span>}
          </span>
          {!step.done && (
            <a class="btn btn-secondary btn-sm" href={step.href}>{step.cta} →</a>
          )}
        </li>
      ))}
    </ol>
  </div>
)}
```

Minimal styles:

```css
.checklist-card { margin-bottom: var(--spacing-lg); padding: var(--spacing-xl); }
.ready-checklist { list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.75rem; }
.ready-checklist li { display: flex; align-items: center; gap: 0.75rem; }
.ready-checklist li.done { opacity: 0.65; }
.step-mark {
  width: 1.5rem; height: 1.5rem; border-radius: 4px;
  display: inline-flex; align-items: center; justify-content: center;
  background: #222233; font-size: 0.85rem;
}
.ready-checklist li.done .step-mark { background: #582CFF; color: #fff; }
.step-body { flex: 1; display: flex; flex-direction: column; }
```

When all green, card hidden (no collapse chrome required).

- [ ] **Step 5: Commit**

```bash
git add \
  crates/queria-db/src/admin_queries.rs \
  crates/queria-api/src/http/dashboard.rs \
  admin/src/pages/dashboard.astro
git commit -m "$(cat <<'EOF'
feat(admin): agent_token_count and get-ready checklist

Co-authored-by: factory-droid[bot] <138933559+factory-droid[bot]@users.noreply.github.com>
EOF
)"
```

---

### Task 6: F — Docs laptop path + deploy base note

**Files:**
- Modify: `docs/runbooks/onboarding.md`
- Modify: `docs/HANDOFF.md` (brief bullet only)

**Interfaces:** None (docs-only). Links must remain valid relative paths.

- [ ] **Step 1: Add Fast first knowledge block**

Insert a short section early in operator path (after edge table / before or after Part A create-project), titled:

```markdown
## Fast first knowledge (laptop)

For a laptop clone without Admin Git registration:

1. Create project (Admin → Projects).
2. Mint **Custom** token with `index_local` checked (warning: uploads land in **Needs review only**).
3. From the repo (or monorepo root):

   ```bash
   export QUERIA_AGENT_TOKEN='…'   # from connect panel
   export QUERIA_EDGE_URL='https://queria.fjulian.id'   # or local edge
   queria-cli index-here --token-env QUERIA_AGENT_TOKEN
   ```

4. Admin → Needs review → **Promote** (trusted path).
5. Mint **Daily** agent for normal retrieve + `index_memory` scratch.

Full contract: index-here design / Part E if present in this runbook. No demo corpus seed. Dual-lane (trusted vs Needs review) unchanged.
```

Adjust headings so they do not collide with existing “Part E” if already present — re-use link text to existing index-here section rather than duplicating the full contract.

- [ ] **Step 2: Document public base for deploy**

In Edge URLs section or a one-liner under env:

```markdown
Production **must** set `QUERIA_PUBLIC_BASE_URL=https://queria.fjulian.id` so agent-setup markdown and MCP snippet absolute URLs use the public edge (not the internal Host). Local: leave default `http://127.0.0.1:17674` or unset to use headers.
```

- [ ] **Step 3: HANDOFF note (2–4 lines)**

Under runtime/config section:

```markdown
- Onboarding friction pack: Admin Daily mint + connect panel; dashboard checklist; `request_base` prefers `QUERIA_PUBLIC_BASE_URL` (prod `https://queria.fjulian.id`). Spec: `docs/archive/superpowers/specs/2026-07-20-onboarding-friction-pack-design.md`.
```

Only after code ships should status move off REFERENCE; this plan’s commit may leave HANDOFF as “pending implement” or add the bullet when Tasks 1–5 land — implementers add HANDOFF in the same docs commit as F.

- [ ] **Step 4: Link integrity skim**

Open relative links in the new section; ensure `../HANDOFF.md` and design archive paths resolve from `docs/runbooks/`.

- [ ] **Step 5: Commit**

```bash
git add docs/runbooks/onboarding.md docs/HANDOFF.md
git commit -m "$(cat <<'EOF'
docs: laptop index-here first-win and public base deploy note

Co-authored-by: factory-droid[bot] <138933559+factory-droid[bot]@users.noreply.github.com>
EOF
)"
```

---

## Acceptance mapping

| Spec AC | Task |
|---|---|
| H config wins + slash strip; header fallback | 1 |
| Admin Daily POSTs tools including `index_memory`, no privileged | 2 + 3 |
| API omit-`tools` stays propose-only | 2 (no change to `tokens.rs` default) |
| Custom privileged default off + warnings | 3 |
| Expiry default `no_expire` | 3 |
| Once-only panel: token, chips, env, docs link | 4 |
| `agent_token_count` + 3-step checklist | 5 |
| Docs Custom + `index_local` → promote → Daily | 6 |

## Suggested ship order

```text
ob-base:           Task 1
ob-tokens:         Tasks 2 → 3 → 4
ob-dashboard-docs: Tasks 5 → 6
```

H first so C env and docs links share one production base.

## Out of scope (do not implement)

- Demo corpus seed
- Self-serve mint
- Dual-lane / retrieve filter changes
- Read-only / Read+propose / Local index Admin presets
- Admin-embedded client one-liners or paste prompts
- Changing API omit-`tools` default to Daily

---

## Self-review

| Check | Result |
|---|---|
| Spec H/T/C/D/F each has a task | Yes — Tasks 1–6 |
| No TBD / TODO / “similar to Task N” placeholders | Pass |
| Exact paths + code for each step | Pass |
| `daily_agent_tools` vs `default_agent_tools` distinction preserved | Pass |
| Admin always POSTs tools; API omit unchanged | Pass |
| Privileged tools not in Daily | Pass |
| Checklist uses project_count / ready / agent_token_count | Pass |
| Connect panel once-only, no one-liners | Pass |
| Type consistency (`agent_token_count: i64`, snake tools) | Pass |
| Ponytail: no new crates, no migration, reuse existing mint API | Pass |

---

## End state (operator path)

```text
Admin logs in
  → checklist (D) shows gaps
  → create project
  → Git source + embed, or F: Custom+index_local → index-here → promote
  → mint Daily (T), copy connect panel (C)
  → agent uses public edge (H) + token → retrieve_context
```

Dual-lane unchanged.
