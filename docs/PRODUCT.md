# Queria Product Contract

> Status: CURRENT
> Last verified: 2026-07-18
> Implementation ledger: [`HANDOFF.md`](./HANDOFF.md)
> Post-MVP improvements: [`IMPROVEMENTS.md`](./IMPROVEMENTS.md)

## North star

Centralize organization-wide and project-specific knowledge for humans and AI agents.

Agents always retrieve before work. They may write in two intentional ways:

1. **Scratch (agent memory)** — direct, project-scoped, searchable immediately (enowx-style DX). Not team “official” truth.
2. **Trusted** — permanent shared knowledge only via **approval** or a **trusted Git ingestion** pipeline.

Dual-lane keeps personal/agent velocity without collapsing the team trust model into a single bucket.

## Knowledge lanes (trust model)

| Lane | How it enters | Who can write via MCP | In default agent retrieve? | Admin “official” knowledge |
|---|---|---|---|---|
| **scratch** | `index_memory` (direct) | Agent with `index_memory` permission | Yes (same project only) | Optional list / TTL / promote / delete |
| **trusted** | Approval of proposed items, or trusted Git pipeline | Not directly by agent | Yes | Yes |
| **pipeline only** | `propose_memory` → `proposed` / `draft` until approved | Agent proposes only | No (unless operator tooling) | Approval queue |

### Hard rules

- Scratch is **project-scoped only**. Never `global`. Never cross-project.
- Agents **must not** overwrite, delete, or silently mutate **trusted** items via MCP.
- Promoting scratch toward team truth: `scratch → proposed` (optional `promote_memory`), still requires human (or policy) approval to become trusted.
- Golden evaluation and leakage gates apply to **trusted** knowledge only.
- When ranking near-duplicates, **prefer trusted over scratch**.
- Audit: every scratch write records agent token (or actor), project, timestamp.

### Agent workflow

```text
Before work:
  retrieve_context(project_id, query)
  # default: trusted (project + optional global) ∪ scratch (project)

After work (fast, no human):
  index_memory(project_id, body, tags…)   # → scratch, searchable now

After work (want team truth):
  propose_memory(...)                     # → proposed → approve → trusted
  # or promote_memory(scratch_id)         # → proposed → approve → trusted
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
| Admin HTTP + Astro UI | Operators | Setup, projects, sources, approvals, tokens, audit, jobs; later scratch list / promote / TTL |
| Admin Playground | Operators | Lean SSR `/admin/playground`: live retrieval probe with rerank/compress toggles, scores, lane, diagnostics (not eval product) |
| MCP (`queria-mcp`) | Agents | See tool table below |
| CLI | Operators | Migrate, embeddings status, retrieval probe (optional `--rerank` / `--compress`), eval (trusted/golden), backup/restore-drill |

### MCP tools (contract)

| Tool | Status | Lane / role |
|---|---|---|
| `retrieve_context` | Shipped | Read trusted + optional scratch (`include_scratch` default **true**; optional global trusted); optional `rerank` / `compress` (server defaults on) |
| `search_knowledge` | Shipped | Search with lane-aware filters; same optional `rerank` / `compress` flags as retrieve |
| `propose_memory` | Shipped | Write → `proposed` (not immediately trusted) |
| `list_projects` | Shipped | Discovery |
| `get_source` | Shipped | Trusted source metadata |
| `index_memory` | **Shipped (Slice A)** | Direct write → **scratch** only (`IMP-13`) |
| `promote_memory` | **Planned** | scratch → `proposed` (`IMP-16`) |
| `list_sources` / `describe_project` / `get_memory_status` | **Planned** | Read-only discovery (`IMP-07`) |

Maintainer actions (approve/reject, reindex, token admin) stay on **session Admin HTTP** by design, not MCP.

Token permission **`IndexMemory`** (Slice A): project-scoped. Without it, agent remains propose-only (legacy). Optional `promote_memory` permission stays planned (`IMP-16`).

## Post-cut product boundaries

After the hard simplification plan in [`SIMPLIFICATION.md`](./SIMPLIFICATION.md), the following are **out of MVP product surface** until product re-opens them:

- 3D knowledge graph on the dashboard (removed P0)
- Multi vector-store backends beyond Qdrant (enowx-rag Qdrant-only P3; Queria uses Voyage + Qdrant)
- Evaluation as a first-class Admin product (page removed P2; use CLI)
- Restore drill as product API (CLI/runbook only P2)
- Pingora-in-process edge (Caddy; P1)

### Dual-lane + retrieval quality (CURRENT)

- **Shipped (Slice A):** project-scoped scratch via `index_memory`; dual-lane retrieve (`include_scratch`); content_hash idempotency; shared max body with `propose_memory`.
- **Shipped (retrieval quality):** hybrid candidate pool → RRF → hydrate → Voyage rerank (fail-open) → near-dup compress (prefer trusted); optional request flags; Admin Playground SSR (`IMP-01`/`IMP-02`/`IMP-03`).
- **Still deferred:** Admin scratch UI (`IMP-15`), `promote_memory` (`IMP-16`), durable query metrics (`IMP-04`).
- **Out of scope:** agent direct write into **trusted** or **global**; replacing approval for team truth; full enowx multi-store or one-binary product shape; Evaluation Admin product page.

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
