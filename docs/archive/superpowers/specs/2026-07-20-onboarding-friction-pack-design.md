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

Operators drop off between Admin login and first agent `retrieve_context`. Friction is product/DX packaging, not trust-model design:

1. Setup docs/snippets can emit wrong host/scheme without an explicit public base.
2. Token mint has no ready Daily tool set; API mint omit-tools stays propose-only without `index_memory`.
3. After mint, raw secret shows once but no assembled env/edge connect info.
4. Dashboard does not list what is still missing.
5. Docs do not call out laptop `index-here` as the fast first-knowledge path.

## Goals

Cut drop-off to first agent retrieve **without** rewriting dual-lane trust (trusted vs Needs review, propose vs promote).

Success:

- Setup markdown/MCP snippets use the real public edge.
- Admin mints Daily (with `index_memory`) in one form submit; always POSTs tools.
- After mint, operator has enough connect info without leaving the product blind (once-only secret).
- Dashboard checklist until ready for agents.
- Docs: laptop first-win via index-here (Custom + `index_local` → promote → Daily retrieve).

## Non-goals

| Out | Why |
|---|---|
| Demo corpus seed | Real project or index-here only |
| Self-serve mint | Admin session is the only mint surface |
| Dual-lane changes | Trusted gated; Needs review excluded by default retrieve |
| SMTP / multi-org | v1 one home org; invite out of band |
| API omit-`tools` → Daily | Omit keeps `default_agent_tools()` propose-only |
| Client installers in Admin | Operators use agent-setup / mcp-snippet endpoints |

## Locked approach

```text
H → T → C → D → F
```

| Phase | Outcome |
|---|---|
| **H** | Public edge base for docs/snippets |
| **T** | Daily default + Custom; Admin always POSTs tools |
| **C** | Once-only lazy connect panel |
| **D** | Three-step checklist + `agent_token_count` |
| **F** | Docs laptop path via Custom + `index_local` |

Milestones: `ob-base` (H), `ob-tokens` (T+C), `ob-dashboard-docs` (D+F).

---

## H — Public base URL

**Surface:** `request_base` in `crates/queria-api/src/http/agent_setup.rs`.

**Resolution order:**

1. `QUERIA_PUBLIC_BASE_URL` when set (strip trailing slash).
2. Else `X-Forwarded-Proto` + `X-Forwarded-Host` / `Host`.
3. Default proto `http` last resort (local).

| Item | Value |
|---|---|
| Env | `QUERIA_PUBLIC_BASE_URL` |
| Production | `https://queria.fjulian.id` |
| Local | unset → headers / `http://127.0.0.1:17674` family |

Markdown from `GET /api/v1/docs/agent-setup` and snippet absolute URLs share this base. No separate docs host.

**Tests:** config wins over headers; slash stripped; empty config uses headers / local fallback.

---

## T — Admin token modes

**Surface:** `admin/src/pages/tokens/index.astro`, `POST /api/v1/agent-tokens`, tool lists in `crates/queria-core/src/auth/agent_token.rs`.

### UI modes (only two)

| Mode | Tools | Default |
|---|---|---|
| **Daily agent** | `list_projects`, `retrieve_context`, `search_knowledge`, `propose_memory`, `get_source`, `index_memory` | Yes |
| **Custom** | Per-tool checkboxes for all tools, including `index_local` and `manage_needs_review` | Advanced |

Privileged defaults **off** in Custom:

- `index_local` — short warning: uploads land in **Needs review only**
- `manage_needs_review` — short warning: promote/reject

No dedicated presets for Read + propose, Read-only, or Local index. Laptop path uses Custom + check `index_local` (or start Daily then Custom-add `index_local` via a Custom mint).

### Admin vs API

| Client | Behavior |
|---|---|
| **Admin** | Always POSTs explicit `tools` for selected mode. Never relies on server default for Daily. |
| **API omit `tools`** | `default_agent_tools()` propose-only (no `index_memory`, no privileged). |

### Helper

`daily_agent_tools()` next to `default_agent_tools()`:

```text
ListProjects, RetrieveContext, SearchKnowledge,
ProposeMemory, GetSource, IndexMemory
```

### Expiry

Default: **`no_expire`**. Finite options remain opt-in.

### Mint response (for C)

Return raw token once plus granted `tools` so the connect panel can chip without a second secret-bearing fetch.

**Tests:** `daily_agent_tools()` has `index_memory`, not privileged tools; omit-tools stays propose-only; Admin POST maps Daily/Custom; privileged Custom defaults off.

---

## C — Connect panel (once after mint)

**Surface:** Admin tokens page, **once** after successful mint. Never re-show secret after navigation away.

### Contents (required)

1. Raw token + copy
2. Tools chips (from mint response)
3. Env export:
   ```bash
   export QUERIA_AGENT_TOKEN='…'
   export QUERIA_EDGE_URL='{public base}'
   export QUERIA_MCP_URL='{public base}/mcp'
   # only when a single project slug was selected on mint:
   export QUERIA_PROJECT_SLUG='…'
   ```
4. Link to `GET {EDGE}/api/v1/docs/agent-setup`

Client one-liners and paste prompts are **not** embedded in Admin HTML. Operators use agent-setup / mcp-snippet endpoints for client commands.

### Security

- Token only in that one response HTML after mint.
- No git-tracked files, no secret downloads, no Admin DB plaintext secret.
- Copy-to-clipboard OK.

### URL sources

Same public base as H. If Admin SSR cannot call `request_base`, use mirrored public-base env or same preference order from request Host. Deploy documents `QUERIA_PUBLIC_BASE_URL`.

**Tests:** after generate: token, chips, env, docs link; after reload list: raw token gone.

---

## D — Dashboard checklist

**Surface:** `admin/src/pages/dashboard.astro`, summary API `dashboard.rs`, `getDashboardSummary`, DB `DashboardSummaryRecord`.

### Card: “Get ready for agents”

Show until all green; hide/collapse when complete.

| # | Complete when | CTA |
|---|---|---|
| 1 | `project_count > 0` | `/admin/projects` |
| 2 | `chunk_counts.ready > 0` | sources / jobs — note: register source or index-here + promote first |
| 3 | `agent_token_count > 0` | `/admin/tokens` |

Ready chunks are the first-win gate (not a separate source-vs-ready split).

### API

Add `agent_token_count: i64` — active (non-revoked) tokens matching `list_agent_tokens` visibility (home org / session scope). Required for accurate step 3.

**Tests:** field present; revoked excluded; Admin client/types read it.

---

## F — Docs laptop first-win

**Surface:** `docs/runbooks/onboarding.md` — short “Fast first knowledge (laptop)” block.

```text
1. Create project (Admin)
2. Mint Custom with index_local checked (Needs review only warning)
3. queria-cli index-here --token-env QUERIA_AGENT_TOKEN
4. Admin Needs review → Promote (trusted)
5. Mint Daily agent for retrieve + index_memory scratch
```

Point to Part E for full index-here contract. No demo corpus. Dual-lane unchanged.

**Tests:** docs review / link integrity only.

---

## Files likely touched

| Area | Paths |
|---|---|
| H | `agent_setup.rs`, config, `.env.example`, deploy/onboarding notes |
| T | `agent_token.rs`, tokens page Admin form |
| C | tokens page (once-only panel) |
| D | dashboard API/DB, `api.ts`, `dashboard.astro` |
| F | `docs/runbooks/onboarding.md` |
| Tests | `request_base`, `daily_agent_tools`, mint, `agent_token_count` |

---

## Acceptance criteria

1. **H:** Config public base wins (slash stripped); without it headers/local work.
2. **T:** Admin Daily default POSTs Daily tools including `index_memory`, excluding privileged tools.
3. **T:** API omit-`tools` remains propose-only.
4. **T:** Custom can grant `index_local` / `manage_needs_review` with warnings and default off.
5. **T:** Expiry defaults to **`no_expire`**.
6. **C:** Once-only panel: raw token + copy, tools chips, env export, agent-setup link. No required client one-liners or paste prompt in Admin.
7. **D:** `agent_token_count` on summary; three-step checklist hides when green.
8. **F:** Onboarding docs document Custom + `index_local` → promote → Daily retrieve; no demo corpus.

---

## Testing matrix

| Layer | What |
|---|---|
| Unit | `request_base` (config wins; strip slash) |
| Unit | `daily_agent_tools()`; privileged absent |
| Unit / integration | mint with Daily tools persists `index_memory` |
| Unit / repository | `agent_token_count`; revoked excluded |
| Playwright (optional) | mint → connect panel; checklist visibility |

---

## Risks

| Risk | Mitigation |
|---|---|
| Wrong public base if env unset | Deploy: set `QUERIA_PUBLIC_BASE_URL`; unit + smoke agent-setup links |
| Operators assume API omit = Daily | Keep propose-only omit; Admin always posts tools |
| Custom privileged over-grant | Warnings; both default off |
| Token leak via logs/screenshots | Once-only secret; no secret download; “never commit” |
| Checklist blocks index-here path | Step 2 is `chunk_counts.ready > 0` after promote |
| Count vs list mismatch | Same filter as `list_agent_tokens` |

---

## Implementation order

```text
ob-base:     H — config + request_base + deploy note + unit tests
ob-tokens:   T — daily_agent_tools + Daily/Custom UI + no_expire
             C — once-only panel (tools + public base)
ob-dashboard-docs:
             D — agent_token_count + 3-step checklist
             F — onboarding.md laptop section
```

H first so C env and docs links share one base.

---

## Self-review

| Check | Result |
|---|---|
| No TBD / TODO | Pass |
| No dual-lane / demo corpus / self-serve mint | Pass |
| Admin Daily vs API omit | Admin posts tools; omit = propose-only |
| Expiry default | `no_expire` |
| No Local index preset | Pass; F uses Custom + `index_local` |
| No required one-liners / paste | Pass; agent-setup endpoints for clients |
| Privileged out of Daily | Pass |
| D three steps | project / ready / token |
| Ship order | H → T → C → D → F |
| Status | REFERENCE until HANDOFF after ship |

---

## End state

```text
Admin logs in
  → checklist (D) shows gaps
  → create project
  → Git source + embed, or F: Custom+index_local → index-here → promote
  → mint Daily (T), copy connect panel (C)
  → agent uses public edge (H) + token → retrieve_context
```

Dual-lane unchanged.
