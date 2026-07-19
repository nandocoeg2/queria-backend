# Queria Product Contract

> Status: CURRENT
> Last verified: 2026-07-19
> Implementation ledger: [`HANDOFF.md`](./HANDOFF.md)
> Post-MVP improvements: [`IMPROVEMENTS.md`](./IMPROVEMENTS.md)

## North star

Centralize organization-wide and project-specific knowledge for humans and AI agents.

Agents always retrieve before work. They may write in two intentional ways:

1. **Scratch (agent memory)** — direct, project-scoped, searchable immediately (enowx-style DX). Not team “official” truth.
2. **Trusted** — permanent shared knowledge only via **approval** or a **trusted Git ingestion** pipeline.

Dual-lane keeps personal/agent velocity without collapsing the team trust model into a single bucket.

## Knowledge lanes (trust model)

| Lane / status | How it enters | Who can write | In default agent retrieve? | Admin / promote |
|---|---|---|---|---|
| **scratch** | `index_memory` (direct) | Agent with `index_memory` | Yes (same project only) | Optional list / TTL / promote / delete |
| **trusted** | Approval of proposed items, or trusted Git pipeline, or **Promote** from Needs review | Not directly by agent | Yes | Yes |
| **needs_review** (“Needs review”) | CLI `index-here` / `POST /api/v1/agent/index-local` (local git roots; self-hosted friendly) | Agent with **`index_local`** permission | **No** (unless `include_needs_review=true`) | Admin `/admin/needs-review` + privileged MCP promote/reject |
| **pipeline only** | `propose_memory` → `proposed` / `draft` until approved | Agent proposes only | No (unless operator tooling) | Approval queue |

Lane is derived from `knowledge_status` (no separate lane column). User-facing term is **Needs review** (not “quarantine”).

### Hard rules

- Scratch is **project-scoped only**. Never `global`. Never cross-project.
- Agents **must not** overwrite, delete, or silently mutate **trusted** items via MCP.
- Promoting scratch toward team truth: `scratch → proposed` (optional `promote_memory`), still requires human (or policy) approval to become trusted.
- **Needs review** is not trusted until an operator **Promotes** (Admin session or privileged MCP). Reject leaves the queue; default retrieve never includes it.
- Golden evaluation and leakage gates apply to **trusted** knowledge only (no needs_review in golden).
- When ranking near-duplicates, **prefer trusted over scratch over needs_review**.
- Audit: every scratch write and index-local / promote / reject records actor, project, timestamp.

### Agent workflow

```text
Before work:
  retrieve_context(project_id, query)
  # default: trusted (project + optional global) ∪ scratch (project)
  # needs_review excluded unless include_needs_review=true

After work (fast, no human):
  index_memory(project_id, body, tags…)   # → scratch, searchable now

After work (want team truth):
  propose_memory(...)                     # → proposed → approve → trusted
  # or promote_memory(scratch_id)         # → proposed → approve → trusted

Bulk local git (self-hosted / laptop clones; no remote allowlist form):
  queria-cli index-here --token-env QUERIA_AGENT_TOKEN --yes
  # → needs_review + async embed jobs → Admin or privileged MCP Promote → trusted
```

Git ingest path is unchanged: allowlisted repo → parse/chunk/scan → trusted (auto-approve only for trusted sources per existing rules).

## Knowledge scopes

| Scope | Meaning |
|---|---|
| `global` | **Trusted only.** Coding, security, deployment, SOP, and operational standards shared across projects. No scratch global. |
| `project` | Project-specific trusted knowledge **plus** that project’s scratch lane. |
| `include_global` | Request flag; still requires token permission. Project-only tokens cannot read global (trusted) knowledge. |
| `include_scratch` | Request flag for retrieval; default **true** for agent `retrieve_context`. May be false for operator “trusted-only” probes. |

## Surfaces

| Surface | Audience | Role |
|---|---|---|
| Admin HTTP + Astro UI | Operators | Setup, projects, sources, approvals, tokens, audit, jobs; **Needs review** list + promote/reject (`/admin/needs-review`); later scratch list / TTL |
| Admin Playground | Operators | Lean SSR `/admin/playground`: live retrieval probe with rerank/compress toggles, scores, lane, diagnostics (not eval product) |
| MCP (`queria-mcp`) | Agents | See tool table below |
| Agent HTTP (hooks + index-local) | Client-side agents / CLI | `POST /api/v1/agent/retrieve-context` + `GET /api/v1/agent/projects` (Bearer `qria_…`). `POST /api/v1/agent/index-local` (Bearer + `index_local`) for bulk local git → **needs_review**. Hooks: `/api/v1/setup/hooks-snippet`, `/setup/hook-script` |
| CLI | Operators | `index-here` (multi-git discover + upload), migrate, embeddings status, retrieval probe (optional `--rerank` / `--compress` / `--include-needs-review`), eval (trusted/golden), backup/restore-drill |

### MCP tools (contract)

| Tool | Status | Lane / role |
|---|---|---|
| `retrieve_context` | Shipped | Read trusted + optional scratch (`include_scratch` default **true**; optional global trusted); optional `include_needs_review` (default **false**); optional `rerank` / `compress` (server defaults on) |
| `search_knowledge` | Shipped | Search with lane-aware filters; same optional flags as retrieve |
| `propose_memory` | Shipped | Write → `proposed` (not immediately trusted) |
| `list_projects` | Shipped | Discovery |
| `get_source` | Shipped | Trusted source metadata |
| `index_memory` | **Shipped (Slice A)** | Direct write → **scratch** only (`IMP-13`) |
| `list_needs_review` | **Shipped** | Privileged list of Needs review items (`manage_needs_review`; **not** default agent mint) |
| `promote_knowledge` | **Shipped** | Privileged needs_review → approved/trusted (`manage_needs_review`) |
| `reject_needs_review` | **Shipped** | Privileged reject Needs review item (`manage_needs_review`) |
| `promote_memory` | **Planned** | scratch → `proposed` (`IMP-16`) |
| `list_sources` / `describe_project` / `get_memory_status` | **Planned** | Read-only discovery (`IMP-07`) |

Maintainer actions (approve/reject, reindex, token admin) stay on **session Admin HTTP** by design, not MCP — except **privileged** Needs review promote tools above (explicit grant only).

Token permissions:

- **`IndexMemory`** (Slice A): project-scoped scratch write. Without it, agent remains propose-only (legacy).
- **`IndexLocal`**: bulk local git via CLI/API → **needs_review** only (not trusted). Required for `queria-cli index-here`.
- **`ManageNeedsReview`**: MCP `list_needs_review` / `promote_knowledge` / `reject_needs_review`. **Not** in default mint.
- Optional `promote_memory` (scratch → proposed) stays planned (`IMP-16`).

## Post-cut product boundaries

After the hard simplification plan in [`SIMPLIFICATION.md`](./SIMPLIFICATION.md), the following are **out of MVP product surface** until product re-opens them:

- 3D knowledge graph on the dashboard (removed P0)
- Multi vector-store backends beyond Qdrant (enowx-rag Qdrant-only P3; Queria uses Voyage + Qdrant)
- Evaluation as a first-class Admin product (page removed P2; use CLI)
- Restore drill as product API (CLI/runbook only P2)
- Pingora-in-process edge (Caddy; P1)

### Dual-lane + retrieval quality + index-here (CURRENT)

- **Shipped (Slice A):** project-scoped scratch via `index_memory`; dual-lane retrieve (`include_scratch`); content_hash idempotency; shared max body with `propose_memory`.
- **Shipped (retrieval quality):** hybrid candidate pool → RRF → hydrate → Voyage rerank (fail-open) → near-dup compress (prefer trusted); optional request flags; Admin Playground SSR (`IMP-01`/`IMP-02`/`IMP-03`).
- **Shipped (local multi-git index-here, local main 2026-07-19):** CLI `index-here` + `POST /api/v1/agent/index-local` → status **`needs_review`**; async embed jobs; default retrieve excludes unless `include_needs_review`; Admin `/admin/needs-review` promote/reject; privileged MCP tools with `manage_needs_review` (`IMP-L1`…`IMP-L5`). Auto-create project slug from origin last path segment. **Prod image may lag.**
- **Still deferred:** Admin scratch UI (`IMP-15`), `promote_memory` (`IMP-16`), durable query metrics (`IMP-04`), auto-promote scores (`IMP-L6`).
- **Out of scope:** agent direct write into **trusted** or **global**; silent promote of bulk local index to trusted; replacing approval for team truth; full enowx multi-store or one-binary product shape; Evaluation Admin product page.

Post-MVP backlog (metrics, Admin scratch, agent DX, etc.) lives in
[`IMPROVEMENTS.md`](./IMPROVEMENTS.md) (`REFERENCE`). Runtime status remains HANDOFF-only.

## Multi-organization tenancy (v1 isolation MVP)

> Status: CURRENT (local `main`; production may lag until post-mission redeploy)  
> Runtime detail: [`HANDOFF.md`](./HANDOFF.md) · Onboarding Team B: [`runbooks/onboarding.md`](./runbooks/onboarding.md)

QuerIa is a **single-stack multi-tenant** system. Isolation is by `organization_id` on every tenant surface (Admin session home, agent token home, Postgres rows, Qdrant payload filter). Dual-lane knowledge (scratch vs trusted) remains **inside** each org.

### Model

| Concept | v1 rule |
|---|---|
| Who creates orgs | Platform **super-admin** only (`POST/GET /api/v1/orgs`) |
| Who joins | **Email invite only** (token in create/invite API response once; no SMTP required) |
| Membership | **One org per user** (unique membership; accept rejects second org) |
| Session home | `user_session.active_organization_id` = sole membership; exposed on `/api/v1/auth/me` |
| Agent home | `agent_token.organization_id` at mint; `project_slugs` must belong to that org |
| Tenant APIs | Require active org (or token org); no home → **403**, not global empty 200 |
| Super-admin without membership | May manage orgs; **cannot** browse tenant projects/knowledge/retrieve as global |

### Bootstrap platform super-admin

Either path works (both evaluated at session load; case-insensitive email):

1. Env: `QUERIA_PLATFORM_SUPER_ADMIN_EMAILS=nando@fjulian.id` (comma-separated list).
2. SQL once:  
   `update user_account set is_platform_super_admin = true where lower(email) = lower('nando@fjulian.id');`

Flag alone elevates the DB column; env list elevates matching emails even if the column is still `false`.

### Provision Team B (happy path)

```text
Super-admin → POST /api/v1/orgs { slug, name, first_admin_email }
  → org + first org_invite; raw invite_token returned once
Invitee → POST /api/v1/invites/accept { token, password, name? }
  → membership + user_account.organization_id aligned
Login → session active org = Team B
Operate → projects / tokens / retrieve only for Team B
```

Admin: `/admin/orgs` (super-admin), public `/admin/invites/accept`, home `/admin/members`.

### Non-goals (v1 — do not expect in product or validators)

- Cross-org **share grants** / soft read sharing
- **Per-org git allowlist** (instance env allowlists only)
- **Multi-membership** or org **switcher** UI
- **SMTP / InviteMailer** (token via API response + optional log of accept path; never require mailer)
- Super-admin default browse of all tenants’ knowledge
- Per-org Voyage keys or separate DB/Qdrant per tenant
- `org_member` permission matrix (invite role may store `org_member`; v1 powers are org-admin-equivalent)

Full design history: [`archive/superpowers/specs/2026-07-18-multi-org-tenancy-design.md`](./archive/superpowers/specs/2026-07-18-multi-org-tenancy-design.md) (REFERENCE).

## Violet Void / Dark Centered Platform

Visual direction: workspace [`DESIGN.md`](../../../DESIGN.md) (**REFERENCE** — approved UI direction, not live implementation status). Runtime/admin status: [`HANDOFF.md`](./HANDOFF.md).

| Token | Value | Role |
|---|---|---|
| `surface.inverse` | `#0A0A0A` | Full composition ground |
| `surface.card` | `#111111` | Cards / elevated surfaces |
| `accent.primary` | `#582CFF` | Primary actions, interactive highlights |
| Typography | Inter / Geist / Funnel Sans | Headings / body / captions |

Admin implementation follows the dark ground and card surfaces; neon streak decorative overlays in DESIGN may be **skipped** in Admin SSR for density and SSR simplicity unless product reopens them.
