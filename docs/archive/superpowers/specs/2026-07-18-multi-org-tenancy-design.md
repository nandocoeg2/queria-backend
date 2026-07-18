# Multi-Organization Tenancy Design

> Status: REFERENCE (approved design direction; not implementation ledger)  
> Date: 2026-07-18  
> Last revised: 2026-07-18 (ponytail cut: isolation MVP only)  
> Product contract companion: [`../../PRODUCT.md`](../../PRODUCT.md)  
> Runtime truth remains: [`../../HANDOFF.md`](../../HANDOFF.md)  
> Implementation plan: [`../plans/2026-07-18-multi-org-tenancy.md`](../plans/2026-07-18-multi-org-tenancy.md)

## Problem

Teams using QuerIa need **tenant-isolated knowledge hubs**: Team B can create projects, ingest/embed code and notes, and run agents against **their** memory without reading or writing Team A’s data.

Today QuerIa is **single-organization in practice**:

- Schema has `organization` and many tables carry `organization_id`.
- First-run setup creates **one** org and one admin; there is no create-org or invite flow.
- Admin session and agent tokens effectively operate inside that one org.
- Git allowlists, Voyage, and Qdrant are instance-shared (fine for multi-tenant); **product lifecycle for Tenant B is missing**.

## Goals (v1)

1. **Hybrid provisioning:** platform super-admin creates an organization; org admin is onboarded via **email invite only** (API returns invite token once; log sink until SMTP exists).
2. **Hard isolation by default:** every list/create/ingest/retrieve path is scoped to the caller’s organization. Team B cannot see Team A.
3. **Human operators are org admins** for v1 (agent tokens still carry tool permissions). No separate org_member permission matrix until a second human role is needed.
4. **One membership per user for v1** (no org switcher).
5. **Preserve dual-lane knowledge** (scratch vs trusted) **inside** each org.

## Non-goals (v1)

- Cross-org **share grants** (read sharing) — **appendix only**
- Per-org git allowlist tables — keep **instance env** allowlists
- Separate deployment / DB / Qdrant per tenant
- Billing, SSO/SAML, SCIM
- Super-admin default browse of tenant knowledge
- Per-org Voyage API keys
- Multi-membership and org switcher UI
- InviteMailer trait / SMTP pluggability (log + return token)
- `allow_shared_knowledge` token flags

## Decisions (locked for v1)

| Topic | Decision |
|---|---|
| Topology | Single stack multi-tenant |
| Who creates orgs | Platform **super-admin** only |
| Who joins | **Email invite only** (first admin + later members) |
| Human roles | **org_admin** for humans in v1; `org_member` role value may exist for invites later but UI/API powers are admin-equivalent until product needs split |
| Membership | **One org per user** in v1 (membership row + legacy `user_account.organization_id` stay aligned) |
| Isolation | `organization_id = home` on all tenant queries and Qdrant filters |
| Soft share | **Deferred** (see Appendix A) |
| Git | **Instance** env allowlist (unchanged) |
| Mail | `tracing::info!` + one-time token in create/invite API response; no mailer abstraction |

---

## 1. Tenancy model (v1)

### Entities

```text
platform
  super_admin (user_account.is_platform_super_admin)
  organizations[]
    memberships[] (user, role)   -- one membership per user in v1
    invites[]
    projects[] / sources[] / knowledge[] / jobs[] / agent_tokens[]
```

### Roles

| Role | Scope | Powers (v1) |
|---|---|---|
| `platform_super_admin` | Instance | Create org; list orgs; issue first org invite; **not** default knowledge access |
| `org_admin` | User’s org | Full current Admin powers within that org (projects, sources, tokens, jobs, approvals, playground) |

Member-restricted powers are **YAGNI for v1**. If an invite uses role `org_member`, treat as full org operator until a real permission split is designed.

### Membership & session

- Table `org_membership (user_id, organization_id, role, created_at)` PK `(user_id, organization_id)`.
- **v1 rule:** a user has at most one membership (enforce in invite accept: reject second org).
- Session stores `user_id` and `active_organization_id` (= sole membership’s org, or legacy `user_account.organization_id`).
- No switcher UI.

### Resource ownership

Unchanged: project, source, chunk, knowledge, jobs, agent tokens already carry `organization_id`. Access = membership on that org.

---

## 2. Auth, invites, agent tokens

### Super-admin

- Flag `user_account.is_platform_super_admin` (ops SQL / env bootstrap for existing admin email).
- Gates: `POST /api/v1/orgs`, `GET /api/v1/orgs`.

### Create organization

`POST /api/v1/orgs` (super-admin):

```json
{
  "slug": "team-b",
  "name": "Team B",
  "first_admin_email": "admin@teamb.example"
}
```

Effects: insert org; create invite; return org + **one-time invite token** in response; log accept URL. No mailer trait.

### Invites

`POST /api/v1/orgs/{slug}/invites` `{ "email", "role" }` — org_admin of that org (or super-admin).

`POST /api/v1/invites/accept` `{ "token", "password" }` — public.

- Hash invite token at rest; single use; expiry (e.g. 7d).
- Accept: create user if needed; insert membership; set `user_account.organization_id` to invited org (v1 single-org).
- Reject accept if user already has a membership to a **different** org.

Accept UI: `/admin/invites/accept?token=…`.

### Session

Login → load membership → set `active_organization_id` to that org → all tenant routes require it.

### Agent tokens

- Mint only inside active org; `project_slugs` must belong to that org.
- Token carries / is bound to `organization_id`.
- Retrieve/list filter: **home org only** (no grant merge in v1).

MCP tools: unchanged names; all project-scoped tools only resolve projects in the token’s org.

---

## 3. Data plane isolation (v1)

### Postgres

- Add `org_membership`, `org_invite`.
- Add `user_account.is_platform_super_admin`, `user_session.active_organization_id`.
- Repository/API: every tenant query uses `organization_id = active_org` (or token org).

### Qdrant

- Same collection.
- Upsert payload includes `organization_id` when writing (if not already).
- Search filter: `organization_id = home` only.
- Backfill existing points: one-off ops (script/SQL/filter), **not** a new permanent CLI product surface.

### Git

- Keep instance `QUERIA_GIT_ALLOWED_*` env policy.
- No `org_git_allowlist` table in v1.

### Leakage bar (tests)

1. Session/token org A cannot list or retrieve org B projects.  
2. Super-admin without membership cannot load tenant knowledge APIs.  
3. Invite accept cannot attach a user already in another org.

### Audit

- Prefer set `organization_id` on tenant audit rows when easy; not a blocker if existing audit shape lags.

---

## 4. Admin UI & HTTP API (v1)

### Super-admin

| Route | Purpose |
|---|---|
| `/admin/orgs` | List orgs; create org + first admin email |

### Org operator (existing pages)

| Route | Change |
|---|---|
| projects, sources, jobs, tokens, approvals, knowledge, playground | Filter by active org |

### New member UX

| Route | Purpose |
|---|---|
| `/admin/members` | List membership + create invite (optional if invite only from orgs page for v1 minimal; prefer members page) |
| `/admin/invites/accept` | Accept invite |

### API map (v1 only)

| Method | Path | Role |
|---|---|---|
| POST/GET | `/api/v1/orgs` | super_admin |
| POST | `/api/v1/orgs/{slug}/invites` | org_admin or super_admin |
| GET | `/api/v1/orgs/current/members` | org_admin |
| POST | `/api/v1/invites/accept` | public |
| existing | `/api/v1/projects`, sources, tokens, … | membership + active org |

**One module** is fine: `orgs.rs` containing create/list org + invites + members until file size hurts. No separate `share_grants.rs` / `git_allowlist.rs` in v1.

---

## 5. Retrieval (v1)

```text
home = token.organization_id | session.active_organization_id
filter organization_id = home
(+ existing project / lane / status filters)
```

No foreign orgs, no grant loop, no foreign scratch rule (scratch already home-only).

---

## 6. Migration from current prod

1. Keep org `fjulian`.  
2. Flag existing admin `is_platform_super_admin = true`.  
3. Backfill `org_membership` from `user_account.organization_id`.  
4. Ensure Qdrant points for current data filter/payload home org.  
5. Create Team B via API; invite; prove isolation tests green.

Implementation order and commits: **see plan**, not duplicated here.

---

## 7. Security checklist (v1)

- [ ] No tenant query without org filter  
- [ ] Token cannot claim foreign `organization_id`  
- [ ] Invite tokens hashed, single-use  
- [ ] Super-admin cannot retrieve arbitrary projects without membership  
- [ ] User cannot join second org via invite accept (v1)  

---

## Approval history

- 2026-07-18: full multi-org + soft grants design approved in session  
- 2026-07-18: **ponytail cut** applied — v1 = isolation + create org + email invite only; grants/git-allowlist-per-org deferred to appendix  

---

## Appendix A — Deferred: soft share grants (not v1)

When product needs “Team B may read project X from Team A”:

- Table `knowledge_share_grant` (source/target org, scope project|global, revoke/expiry)
- Retrieve merges home ∪ grant-readable **trusted** only (never scratch)
- Admin `/admin/shares` + `POST/GET/DELETE /api/v1/share-grants`
- Optional MCP `list_projects` marker `shared: true`

## Appendix B — Deferred: per-org git allowlist

When instance allowlist is too coarse:

- Table `org_git_allowlist` (ssh_host | ssh_repository | path_root)
- Worker union instance defaults + org rows
- Admin API under org settings

## Appendix C — Deferred: org_member permission split

When humans need read-only operators:

- Restrict token mint, ingest trigger, invites to `org_admin`
- `org_member`: playground + read + propose_memory via policy
