# Onboarding Friction Pack Design

> Status: REFERENCE — approved design for implementation; not runtime truth
> Last verified: 2026-07-20
> Runtime: HANDOFF when shipped
> Product contract: [`../../PRODUCT.md`](../../PRODUCT.md)
> Runtime truth: [`../../HANDOFF.md`](../../HANDOFF.md)
> Related DX: [`../../runbooks/onboarding.md`](../../runbooks/onboarding.md)
> Related specs:
> - [`2026-07-19-local-git-index-here-design.md`](./2026-07-19-local-git-index-here-design.md)
> - [`2026-07-19-agent-path-edge-e2e-design.md`](./2026-07-19-agent-path-edge-e2e-design.md)
> - [`2026-07-19-agent-auto-retrieve-hooks-design.md`](./2026-07-19-agent-auto-retrieve-hooks-design.md)

## Problem

Operators drop off between Admin login and a first successful agent `retrieve_context`. Friction is product/DX packaging, not trust-model design:

1. **Public base URLs** in setup docs/snippets follow proxy headers and can emit wrong host/scheme in production without an explicit public base.
2. **Token mint UI** has no ready “Daily agent” tool set; operators must know which MCP tools to grant, and default API mint stays propose-only without `index_memory`.
3. **After mint**, the tokens page shows a raw secret once but does not assemble env, MCP one-liners, and paste prompt — operators leave the page incomplete.
4. **Dashboard** does not tell operators what is still missing (project / knowledge / embeddings / token).
5. **Docs-only first-win** via laptop `index-here` exists as Part E but is not called out as the fast path for “I have code on my machine.”

**Need:** a single phased friction pack that gets an operator from empty console to agent retrieve without rewriting the dual-lane trust model.

## Goals

Cut operator → first agent retrieve drop-off **without** rewriting the dual-lane trust model (trusted vs scratch / Needs review, propose vs promote gates).

Success looks like:

- Setup markdown and MCP snippets always point at the real public edge.
- Admin can mint a Daily agent token in one click (explicit tools POST).
- Mint success surfaces a complete connect-agent panel once (token never re-shown).
- Dashboard checklist drives the remaining gaps until green.
- Docs explain a laptop-only first knowledge path using Local index + promote + Daily retrieve.

## Non-goals (out of scope)

| Out | Why |
|---|---|
| Demo corpus seed | Product must not fake “first knowledge”; use real project or index-here |
| Self-serve token mint (agents minting their own tokens) | Admin session remains the only mint surface |
| Dual-lane rule changes | Trusted still gated; Needs review still excluded by default retrieve |
| SMTP / org switcher | Multi-org remains invite-token out of band; v1 one home org |
| API omit-`tools` → Daily default | Backward compatible: omit keeps `default_agent_tools()` propose-only |
| Multi-repo installer product | Snippets + paste prompt only; no remote machine config writer |

## Locked approach

Single phased design. Ship order:

```text
H → T → C → D → F
```

| Phase | Name | Outcome |
|---|---|---|
| **H** | Public base URL | Setup docs/snippets use config-first public edge base |
| **T** | Admin token presets | Daily default; explicit tools always POSTed from Admin |
| **C** | Connect agent after mint | Once-only full panel on token reveal |
| **D** | Dashboard checklist | Steps until project + knowledge + embeddings + token |
| **F** | Docs-only first-win | Onboarding runbook points to index-here path |

Suggested milestones: `ob-base` (H), `ob-tokens` (T+C), `ob-dashboard-docs` (D+F).

---

## H — Public base URL

### Surface

`request_base` in `crates/queria-api/src/http/agent_setup.rs` (used by agent-setup markdown, MCP snippets, hooks-snippet URLs).

### Resolution order (locked)

1. Prefer `QUERIA_PUBLIC_BASE_URL` from config when set (strip trailing slash).
2. Else `X-Forwarded-Proto` + `X-Forwarded-Host` / `Host` (current header path).
3. Default proto `http` only as last resort (local development).

### Config / deploy

| Item | Value |
|---|---|
| Env | `QUERIA_PUBLIC_BASE_URL` |
| Production (this deploy) | `https://queria.fjulian.id` |
| Local | unset → headers / `http://127.0.0.1:17674` family |

Deploy note: set `QUERIA_PUBLIC_BASE_URL=https://queria.fjulian.id` on API (and any service that regenerates setup absolute URLs). Strip trailing `/` once at config load or in `request_base`.

### Behavior

- Markdown from `GET /api/v1/docs/agent-setup` links use the resolved base.
- Snippet endpoints that embed absolute MCP/edge URLs use the same base.
- No other public docs product surface; this pack does not add a separate docs host.

### Tests (H)

- Unit: config value wins over headers; trailing slash stripped.
- Unit: when config empty, proto/host headers form base; missing Host falls back sensibly for local.

---

## T — Admin token presets

### Surface

Admin tokens page: `admin/src/pages/tokens/index.astro`  
API mint: `POST /api/v1/agent-tokens` in `crates/queria-api/src/http/tokens.rs`  
Tool lists: `crates/queria-core/src/auth/agent_token.rs` (+ permissions enum)

### Presets (locked)

| Preset | Tools | UI default |
|---|---|---|
| **Daily agent** | `list_projects`, `retrieve_context`, `search_knowledge`, `propose_memory`, `get_source`, **`index_memory`** | **Yes** (selected by default in Admin form) |
| **Read + propose** | current `default_agent_tools()` (no `index_memory`) | opt-in |
| **Read-only** | `list_projects`, `retrieve_context`, `search_knowledge`, `get_source` | opt-in |
| **Local index** | Daily tools + **`index_local`** | opt-in dedicated preset; warning: uploads land in **Needs review only** |
| **Custom** | per-tool checkboxes; advanced **`manage_needs_review`** default **off** with warning | advanced |

Privileged tools **never** appear in the Daily default:

- `index_local` — only Local index preset (or Custom)
- `manage_needs_review` — only Custom, default unchecked

### Admin vs API contract (explicit, no contradiction)

| Client | Behavior |
|---|---|
| **Admin UI** | Always POSTs an explicit `tools` array matching the selected preset (or Custom checkboxes). Never relies on server default for Daily. |
| **API / curl / scripts omitting `tools`** | Keep **`default_agent_tools()`** = propose-only (no `index_memory`, no privileged tools). Backward compatible. |

### Rust helper (locked)

Add `daily_agent_tools()` next to `default_agent_tools()` in `agent_token.rs`:

```text
daily_agent_tools():
  ListProjects, RetrieveContext, SearchKnowledge,
  ProposeMemory, GetSource, IndexMemory
```

Use in tests and docs alignment. Admin may hardcode the same tool **names** client-side (string parity with API tool enum serialization).

### Expiry form (locked)

Default selected expiry control: **`no_expire`**.

Existing expiry options may remain for opt-in finite lifetimes; the selected default on the generate form is no expiry.

### Mint response (for phase C)

Successful mint already returns raw token once. Ensure response includes the granted `tools` list (or equivalent permissions payload) so the connect panel can render chips without a second round-trip that re-exposes the secret.

### Tests (T)

- Unit: `daily_agent_tools()` contains `index_memory` and does **not** contain `index_local` / `manage_needs_review`.
- Unit: `default_agent_tools()` remains propose-only (existing assertion stays green).
- Integration / API: mint with explicit Daily tools persists `index_memory` on the token permissions.
- Admin always sends `tools` in generate POST (form mapping tests or SSR unit if present).

---

## C — Connect agent on token reveal (full)

### Surface

Admin tokens page, **once** after successful mint (raw token already shown today). Never re-fetch or re-display the secret after navigation away.

### Once-only panel contents (locked order)

1. **Raw token + copy** (existing)
2. **Tools chips** from mint response (Daily / custom labels optional; chips = tool names)
3. **Env export block**
   ```bash
   export QUERIA_AGENT_TOKEN='…'          # one-shot; never commit
   export QUERIA_EDGE_URL='{public base}' # H resolution / known edge
   export QUERIA_MCP_URL='{public base}/mcp'
   # only when a single project slug was selected on mint:
   export QUERIA_PROJECT_SLUG='…'
   ```
4. **Client one-liners** for **droid**, **claude**, **codex** (short; point at MCP URL + env token pattern already documented on agent-setup)
5. **Short agent paste prompt** with EDGE / TOKEN / SLUG filled for the operator to paste into the coding agent
6. **Link** to `GET {EDGE}/api/v1/docs/agent-setup` for the full markdown path

### Security rules (locked)

- Never write the raw token into git-tracked files, downloaded configs committed to repo, or persistent Admin DB plaintext.
- Token appears only in that **one** response HTML after mint (same threat model as today).
- Copy-to-clipboard is fine; no auto-download of files containing the secret by default.

### URL sources

- Edge / MCP base: same public base as phase H (config / headers), not hard-coded IP in UI if base is available.
- If Admin SSR cannot call `request_base` directly, use configured public base env mirrored to Admin, or derive from request Host with the same preference order where possible. Production deploy documents `QUERIA_PUBLIC_BASE_URL`.

### Tests (C)

- Manual / Playwright optional (Admin server available): after generate, panel shows token, tools chips, env block, docs link; after reload of tokens list, raw token is **not** shown again.

---

## D — Dashboard checklist + `agent_token_count`

### Surface

- Admin dashboard: `admin/src/pages/dashboard.astro`
- Summary API: `crates/queria-api/src/http/dashboard.rs`
- Admin client: `getDashboardSummary` in `admin/src/lib/api.ts`
- Store: `get_dashboard_summary` / `DashboardSummaryRecord` in `queria-db` admin queries

### Card: “Get ready for agents”

Show until **all** steps complete; hide or collapse when green.

| # | Complete when | CTA link | Note |
|---|---|---|---|
| 1 | `project_count > 0` | `/admin/projects` | Create at least one project |
| 2 | Knowledge present: `source_count > 0` **OR** `chunk_counts.ready > 0` | `/admin/sources` | Note index-here as laptop alternative |
| 3 | Embeddings: `chunk_counts.ready > 0` | jobs / sources (existing job/status surfaces) | Wait for embed ready |
| 4 | `agent_token_count > 0` | `/admin/tokens` | Mint Daily (or other) agent token |

Step 2 allows either a registered source **or** ready chunks so Local index / needs_review → promote / scratch paths can complete without a Git source form.

### API change (locked)

Add `agent_token_count: i64` to dashboard summary response.

**Count semantics:** active (**non-revoked**) agent tokens visible for the session home org / user, matching **`list_agent_tokens` visibility** (same filter the tokens page list uses). Do not invent a broader super-admin global count for this card.

### Tests (D)

- Unit / repository: summary includes `agent_token_count`.
- Active vs revoked: revoked tokens do not increment count.
- Admin type / client updated so checklist can read the field.

---

## F — Docs-only fast first-win (`index-here`)

### Surface

`docs/runbooks/onboarding.md` — short section near the top or a clearly linked “Fast first knowledge (laptop)” block.

### Path (docs only; no product demo corpus)

```text
1. Create project (Admin)
2. Mint Local index preset (or Custom with index_local grant)
3. queria-cli index-here --token-env QUERIA_AGENT_TOKEN
4. Admin Needs review → Promote (trusted)
5. Mint / use Daily agent token for agents (retrieve + index_memory scratch)
```

Point to **Part E** for the full index-here contract (discover, needs_review, promote, non-goals). This pack does **not** seed a demo corpus and does **not** change dual-lane rules.

### Tests (F)

- Docs review only (link integrity to Part E / agent-setup). No runtime test required for prose.

---

## Files likely touched

| Area | Paths |
|---|---|
| Public base (H) | `crates/queria-api/src/http/agent_setup.rs`, `crates/queria-core/src/config.rs`, `.env.example`, deploy/runbook notes in `docs/runbooks/deployment.md` or `onboarding.md` |
| Daily tools (T) | `crates/queria-core/src/auth/agent_token.rs`, `crates/queria-api/src/http/tokens.rs` (only if validation/helpers need wiring), `admin/src/pages/tokens/index.astro` |
| Connect panel (C) | `admin/src/pages/tokens/index.astro` (and small CSS if needed under admin styles) |
| Dashboard (D) | `crates/queria-api/src/http/dashboard.rs`, `crates/queria-db/src/admin_queries.rs` (and/or repositories), `admin/src/lib/api.ts`, `admin/src/pages/dashboard.astro` |
| Docs (F) | `docs/runbooks/onboarding.md` |
| Tests | unit tests next to `request_base` / `daily_agent_tools`; API mint integration; optional Playwright under `admin/` |

---

## Acceptance criteria (pack)

1. **H:** With `QUERIA_PUBLIC_BASE_URL` set, setup docs and snippet absolute URLs use that base (trailing slash stripped); without it, header-based base still works; local fallback remains `http`.
2. **T:** Admin Daily preset is the default selection and POSTs the Daily tool list including `index_memory` but excluding privileged tools.
3. **T:** API clients that omit `tools` still get `default_agent_tools()` propose-only (no `index_memory`).
4. **T:** Local index preset exists, includes `index_local`, and shows a Needs review warning; Custom can grant `manage_needs_review` only with advanced warning and default off.
5. **T:** Expiry control defaults to **`no_expire`**.
6. **C:** After mint, the once-only panel includes raw token + copy, tools chips, env exports, droid/claude/codex one-liners, short paste prompt, and link to agent-setup docs; token is not written to git-tracked files.
7. **D:** Dashboard exposes `agent_token_count` and shows “Get ready for agents” until project + knowledge + embeddings + token steps are all green, then hides/collapses.
8. **F:** Onboarding runbook documents the laptop fast first-win via index-here → promote → Daily agent retrieve, without a demo corpus seed.

---

## Testing matrix

| Layer | What |
|---|---|
| Unit | `request_base` resolution (config wins; strip slash) |
| Unit | `daily_agent_tools()` contents; privileged tools absent |
| Unit / integration | mint with explicit tools persists `index_memory` |
| Unit / repository | dashboard summary includes `agent_token_count`; revoked excluded |
| Playwright (optional) | mint → connect panel; dashboard checklist visibility if Admin server available |

---

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Wrong public base still ships if env unset in prod | Deploy checklist: set `QUERIA_PUBLIC_BASE_URL=https://queria.fjulian.id`; unit test config path; smoke `GET /api/v1/docs/agent-setup` links |
| Operators assume omit-`tools` API mint is Daily | Keep propose-only omit; Admin only POSTs Daily; document clearly in this design + runbook |
| Local index preset over-grants / confuses trust | Warning copy: Needs review only; promote still human (or explicit `manage_needs_review`, not Daily) |
| Token leak via HTML logs / screenshots | Same once-only secret model; no file download of secret; docs “never commit” |
| Checklist step 2 false-negative for index-here-only orgs | Complete when `source_count > 0` **OR** `chunk_counts.ready > 0` |
| `agent_token_count` mismatches list visibility | Reuse list filter / same org-home scope as `list_agent_tokens` |
| Scope creep into dual-lane or SMTP | Non-goals table; ship H→T→C→D→F only |

---

## Implementation order

```text
ob-base:
  H — config field + request_base preference + deploy note + unit tests

ob-tokens:
  T — daily_agent_tools + Admin presets + no_expire default + mint tools POST
  C — once-only connect panel (depends on mint response tools + public base)

ob-dashboard-docs:
  D — agent_token_count + checklist UI
  F — onboarding.md fast first-win section
```

No parallel dependency between D and F after T/C. H should land first so C’s env block and docs links share one base.

---

## Self-review (pre-implementation)

| Check | Result |
|---|---|
| No TBD / TODO left in this design | Pass |
| No dual-lane / demo corpus / self-serve mint | Pass (non-goals) |
| API omit-`tools` vs Admin Daily | Explicit: omit = propose-only; Admin always posts tools; Daily is Admin UI default only |
| Expiry default | Explicit: **`no_expire`** |
| Local index preset | Explicit: Daily + `index_local` with Needs review warning |
| Privileged tools out of Daily | Explicit: never in Daily default |
| Ship order | H → T → C → D → F |
| Runtime truth not claimed | Status REFERENCE until HANDOFF updates after ship |

---

## End state (operator story)

```text
Admin logs in
  → dashboard checklist (D) shows gaps
  → create project
  → either Git source + embed, or F path: Local index mint (T) → index-here → promote
  → mint Daily agent (T), copy connect panel (C)
  → agent uses public edge (H) + token → retrieve_context first win
```

Dual-lane trust model unchanged end-to-end.
