# Multi-Organization Tenancy Design

> Status: REFERENCE (approved design direction; not implementation ledger)  
> Date: 2026-07-18  
> Product contract companion: [`../../PRODUCT.md`](../../PRODUCT.md)  
> Runtime truth remains: [`../../HANDOFF.md`](../../HANDOFF.md)

## Problem

Teams using QuerIa need **tenant-isolated knowledge hubs**: Team B can create projects, ingest/embed code and notes, and run agents against **their** memory without reading or writing Team A’s data by default.

Today QuerIa is **single-organization in practice**:

- Schema has `organization` and many tables carry `organization_id`.
- First-run setup creates **one** org and one admin; there is no create-org, invite, or org switcher.
- Admin session and agent tokens effectively operate inside that one org.
- Git allowlists, Voyage, and Qdrant are instance-shared (fine for multi-tenant) but **product lifecycle for Tenant B is missing**.

## Goals

1. **Hybrid provisioning:** platform super-admin creates an organization; the org’s admin invites members later (email invites only).
2. **Soft isolation:** default deny across orgs; **explicit share grants** allow cross-org **read** of selected scopes.
3. **Full surface:** data plane, Admin UI, session API, agent tokens/MCP, leakage tests, per-org git allowlists, in one cohesive design (one large implementation plan).
4. **Preserve dual-lane knowledge** (scratch vs trusted) **inside** each org.

## Non-goals (this design)

- Separate deployment / DB / Qdrant per tenant
- Billing, SSO/SAML, SCIM
- Super-admin default browse of tenant knowledge (break-glass later)
- Per-org Voyage API keys (instance key remains shared)
- Agent write into a foreign org (even with a read grant)

## Decisions (locked)

| Topic | Decision |
|---|---|
| Topology | Single stack multi-tenant (Approach 1) |
| Who creates orgs | Platform **super-admin** only |
| Who invites members | **Org admin**, email invite only (no temp-password bootstrap for members) |
| First org admin | Also via **email invite** from super-admin when org is created (not plaintext password create) |
| Isolation default | Hard filter on `organization_id` everywhere |
| Soft share | Explicit **read** grants; writes always home-org |
| Vectors | Same Qdrant collection(s); payload + filter `organization_id` |
| Git | **Per-org allowlist** (plus optional instance defaults) |
| Delivery | One design doc + one big implementation plan (user-selected) |

---

## 1. Tenancy model

### Entities

```text
platform
  super_admin (user flag or role)
  organizations[]
    memberships[] (user, role: org_admin | org_member)
    invites[]
    projects[] / sources[] / knowledge[] / jobs[] / agent_tokens[]
    share_grants_outbound[] / share_grants_inbound[]
```

### Roles

| Role | Scope | Powers |
|---|---|---|
| `platform_super_admin` | Instance | Create org; issue first org_admin invite; list orgs; **not** default knowledge access |
| `org_admin` | Active org | Invite/revoke members; manage projects/sources/tokens/approvals/jobs; create/revoke **outbound** share grants; accept nothing foreign write |
| `org_member` | Active org | Use Playground; read knowledge; `propose_memory` / retrieve via MCP if token allows; no invites, no token mint, no share grants |

### Membership & session

- Table `org_membership (user_id, organization_id, role, created_at)` unique `(user_id, organization_id)`.
- Session cookie stores `user_id` and **`active_organization_id`**.
- MVP: after login, if user has one membership, select it; if many, require switcher (or last-used). Super-admin without membership can open `/admin/orgs` only.
- All Admin session routes resolve **active org** then enforce membership.

### Resource ownership

Every of these carries non-null `organization_id` (home org):

- project, source_document, chunk, knowledge_item, ingestion_job, agent_token, invite, share grant (as source_org / target_org)

Scratch and trusted lanes remain as today, always inside home org.

### Soft share

Grant = **read permission** for target principal (org) over a scope in source org.

```text
knowledge_share_grant
  id
  source_organization_id
  target_organization_id
  scope_type: project | organization_global   -- v1 scopes
  scope_project_id: nullable (required if scope_type=project)
  created_by_user_id
  created_at
  expires_at: nullable
  revoked_at: nullable
```

Rules:

- Only `org_admin` of **source** org creates/revokes outbound grants.
- Target org members/tokens may **retrieve/search** granted knowledge (trusted; scratch is **never** shared).
- Target cannot approve, ingest, index_memory, or mint tokens into source org.
- Grants do not imply mutual access.

---

## 2. Auth, invites, agent tokens

### Super-admin bootstrap

- Existing first admin of org `fjulian` can be flagged `platform_super_admin=true` via migration/env one-shot, or new env `QUERIA_PLATFORM_SUPER_ADMIN_EMAILS`.
- Endpoints gated: `POST /api/v1/orgs`, list orgs.

### Create organization

`POST /api/v1/orgs` (super-admin):

```json
{
  "slug": "team-b",
  "name": "Team B",
  "first_admin_email": "admin@teamb.example"
}
```

Effects:

1. Insert organization.
2. Create **invite** for `first_admin_email` with role `org_admin` (email-only; no password set by super-admin).
3. Return org + invite status (email send result).

### Invites (email only)

`POST /api/v1/orgs/{org_slug}/invites` (org_admin or super-admin for first admin):

```json
{ "email": "dev@teamb.example", "role": "org_member" }
```

- Tokenized invite (`invite_token` hash stored, raw sent once).
- Expiry default 7 days.
- `POST /api/v1/invites/accept` with `{ "token", "password", "display_name?" }`:
  - If user email exists: attach membership (if not already).
  - Else: create `user_account` with password hash + membership.
- No API that sets member passwords without accept flow.

Email delivery: pluggable (`log` sink in dev, SMTP/provider in prod). Accept page: `/admin/invites/accept?token=…`.

### Session

- Login unchanged (email/password) then set `active_organization_id` from membership.
- Middleware: `require_session` + `require_org_role(min_role)` + bind repositories to org.

### Agent tokens

- Mint only by `org_admin` in active org.
- Stored permissions: `organization_id`, `project_slugs` (subset of home org projects), tools, `allow_global_knowledge` (home-org global only).
- Retrieve path may read **granted** foreign projects without putting them in `project_slugs` if query names a grant-visible id; alternatively require grant-visible projects to be listed in a separate token flag `allow_shared_knowledge` (default true for org).

**MCP tools (unchanged names):**

| Tool | Home org | Shared foreign |
|---|---|---|
| `list_projects` | Yes | Optional: show grant-visible projects with `shared: true` (design default: **include**) |
| `retrieve_context` / `search_knowledge` | Yes | Read if grant covers project/global |
| `index_memory` / `propose_memory` | Home only | **Denied** |
| `get_source` | Home | Denied for foreign |

---

## 3. Data plane isolation

### Postgres

- Extend/verify `organization_id` on all knowledge-bearing tables.
- Add: `org_membership`, `org_invite`, `knowledge_share_grant`, `org_git_allowlist` (repos/hosts per org).
- Repository pattern: every query joins membership or filters `organization_id = $active_org` (or agent token org).

### Qdrant

- Keep shared collection naming (e.g. `queria_local_chunks_v1`).
- **Required payload:** `organization_id` (UUID string).
- Search filter:  
  `organization_id IN (home_org ∪ orgs_readable_via_grants_for_this_query)`  
  AND existing project/status/lane filters.
- On grant revoke: no reindex required; filters stop returning points.
- Migration: backfill payload for existing points to current org (`fjulian`).

### Git allowlist (per-org)

```text
org_git_allowlist
  organization_id
  kind: ssh_host | ssh_repository | path_root
  value: "github.com" | "owner/repo.git" | "/data/org-b"
```

Worker validation: URI/path must match **union** of instance defaults (if any) and **source’s organization** allowlist.

Production seed path pattern: mount under allowlisted root **and** register path_root for that org.

### Worker

- Job row already has `organization_id`; claim/process only that org’s sources.
- Never follow `source_path` outside allowlist for that org.

### Leakage bar (automated tests)

1. Token A cannot `list_projects` / retrieve B without grant.
2. With project grant A→B, B’s token retrieves A’s **trusted** project knowledge only; not scratch.
3. B cannot `index_memory` / `propose_memory` into A’s project_id.
4. Admin session org A cannot load org B project APIs.
5. Super-admin cannot load project knowledge endpoints without membership (403).

### Audit

- `audit_log.organization_id` set for tenant actions; platform actions may use null org + `actor_type=platform`.

---

## 4. Admin UI & HTTP API

### Super-admin

| Route | Purpose |
|---|---|
| `/admin/orgs` | List orgs, create org + first admin email |
| (no knowledge browse) | — |

### Org admin / member (org-scoped existing pages)

| Route | Change |
|---|---|
| `/admin/projects`, sources, jobs, tokens, approvals, knowledge, playground | Filter active org; hide cross-org |
| `/admin/members` | **New:** invites, roles, revoke |
| `/admin/shares` | **New:** outbound grants create/revoke; inbound list |

### API map (session)

| Method | Path | Role |
|---|---|---|
| POST | `/api/v1/orgs` | super_admin |
| GET | `/api/v1/orgs` | super_admin |
| POST | `/api/v1/orgs/{slug}/invites` | org_admin or super_admin |
| GET | `/api/v1/orgs/{slug}/members` | org_admin |
| DELETE | `/api/v1/orgs/{slug}/members/{user_id}` | org_admin |
| POST | `/api/v1/invites/accept` | public (token) |
| GET/POST/DELETE | `/api/v1/share-grants` | org_admin (outbound); members may GET inbound |
| GET/POST | `/api/v1/orgs/{slug}/git-allowlist` | org_admin + super_admin |
| existing | `/api/v1/projects`, sources, tokens, … | membership + active org |

Agent-setup public docs updated: tokens are org-scoped; multi-tenant isolation rules one paragraph.

---

## 5. Retrieval & grants (algorithm)

```text
principal = session user | agent token
home = principal.active_organization_id | token.organization_id

readable_orgs = { home }
readable_projects_foreign = {}

for grant in active_grants_to(home):
  if grant.scope_type == organization_global:
    readable_orgs += grant.source_organization_id  # only global scope filter
  if grant.scope_type == project:
    readable_projects_foreign += grant.scope_project_id

query_projects = requested project_ids ∩ (
  projects_owned_by(home) ∪ readable_projects_foreign
)

Qdrant/Postgres hybrid:
  filter organization_id IN readable_orgs_relevant_to_projects
  filter project_id IN query_projects (and global lane only if allowed)
  filter status/lane: trusted only when organization_id != home (never foreign scratch)
```

Rerank/compress unchanged after hydrate; authorize again on hydrate rows (defense in depth).

---

## 6. Migration from current prod

1. Keep org `fjulian` as org A.
2. Flag existing admin as platform super-admin (and org_admin of `fjulian`).
3. Backfill memberships for all users.
4. Backfill Qdrant payload `organization_id`.
5. Move instance git allowlist entries into `org_git_allowlist` for `fjulian` (and platform defaults if needed).
6. Create Team B via new API; invite; no data leakage tests green before announcing multi-tenant.

---

## 7. Security checklist

- [ ] No raw SQL without org filter on tenant tables  
- [ ] Token forgery cannot set foreign `organization_id`  
- [ ] Invite tokens single-use, hashed at rest  
- [ ] Share grant revoke immediate  
- [ ] Scratch never in grant hydrate  
- [ ] Path traversal / git path stay within org allowlist roots  
- [ ] Super-admin cannot call retrieve with arbitrary project without membership  

---

## 8. Implementation shape (for writing-plans later)

Single plan, recommended internal phases for commit hygiene (still one plan document):

1. Schema + membership + session active_org + enforce on existing APIs  
2. Super-admin create org + email invites + accept UI  
3. Agent token org binding + leakage tests  
4. Qdrant payload + filter + backfill  
5. Per-org git allowlist + worker  
6. Share grants API + retrieve merge + Admin shares UI  
7. Docs (PRODUCT, onboarding, HANDOFF)  

---

## 9. Open items (resolved at implement time, not product forks)

- Email provider wiring (SMTP env) vs log-only in MVP deploy  
- Exact Astro routes styling (Sahara)  
- Whether `list_projects` shows shared projects (default **yes**, marked `shared`)  

---

## Approval

Design sections confirmed in session 2026-07-18:

- Tenancy model: approved  
- Auth/invites: email-only invites  
- Data plane: per-org git allowlist  
- UI/API: approved  

Next: user reviews this file; then `writing-plans` for the implementation plan (no code until plan approval).
