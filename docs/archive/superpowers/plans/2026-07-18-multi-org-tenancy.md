# Multi-Organization Tenancy Implementation Plan (v1 isolation MVP)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Org-scoped ACL so Team B is isolated from Team A; super-admin creates orgs; first admin (and further users) join via email invite only.

**Architecture:** Single stack. Add `org_membership` + `org_invite`, session `active_organization_id`, and `is_platform_super_admin`. All tenant APIs and agent tokens filter `organization_id = home`. Qdrant filter home org only. No share grants, no per-org git allowlist, no mailer trait in v1.

**Tech Stack:** Rust (`queria-db`, `queria-api`, `queria-mcp`, `queria-search`, `queria-core`), Postgres migrations, Astro Admin, existing sessions + agent tokens, Voyage + Qdrant.

**Spec:** [`../specs/2026-07-18-multi-org-tenancy-design.md`](../specs/2026-07-18-multi-org-tenancy-design.md) (ponytail-cut v1)

## Global Constraints

- Isolation MVP only: create org + invite + enforce org filter
- Super-admin creates orgs; invite-only join (return token once + log; no InviteMailer)
- One membership per user in v1 (no switcher)
- Humans treated as org_admin powers; no org_member restriction matrix
- No knowledge_share_grant, no org_git_allowlist, no shares UI
- Instance git env allowlist unchanged
- Qdrant: ensure org payload/filter if missing; no new permanent CLI product for backfill
- Leakage smoke: A cannot read B
- Match Sahara Admin patterns; update HANDOFF/PRODUCT/onboarding when shipping

## File map

| Area | Files |
|---|---|
| Schema | `migrations/20260718000100_multi_org_tenancy.sql` |
| Roles/session | `crates/queria-core/src/auth/` (existing mod, not a new file for a tiny enum), `queria-db` auth repo, `queria-api` auth handlers |
| Orgs + invites | **One** `crates/queria-api/src/http/orgs.rs` (orgs + invites + members) |
| Enforce | Existing handlers under `crates/queria-api/src/http/*` + token mint |
| Vectors | `queria-search` upsert/search filter home org |
| Admin | `admin/src/pages/orgs/`, `invites/accept.astro`, optional `members/`; `AdminLayout.astro` |
| Docs | PRODUCT, HANDOFF, onboarding |

Deferred (spec appendices only): share_grants, git_allowlist modules, grant-aware retrieve.

---

### Task 1: Schema — membership, invite, super-admin flag, session active org

**Files:**
- Create: `migrations/20260718000100_multi_org_tenancy.sql`
- Modify: migrate registration if required
- Test: `cargo run -p queria-cli -- database migrate`

**Produces:** `org_membership`, `org_invite`, `user_account.is_platform_super_admin`, `user_session.active_organization_id`

- [ ] **Step 1: Migration SQL (no grant/git tables)**

```sql
alter table user_account
  add column if not exists is_platform_super_admin boolean not null default false;

create table if not exists org_membership (
  user_id uuid not null references user_account(id) on delete cascade,
  organization_id uuid not null references organization(id) on delete cascade,
  role text not null check (role in ('org_admin', 'org_member')),
  created_at timestamptz not null default now(),
  primary key (user_id, organization_id)
);

create unique index if not exists idx_org_membership_one_org_per_user
  on org_membership (user_id);  -- v1: at most one org

create index if not exists idx_org_membership_org on org_membership(organization_id);

alter table user_session
  add column if not exists active_organization_id uuid references organization(id) on delete set null;

create table if not exists org_invite (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization(id) on delete cascade,
  email text not null,
  role text not null check (role in ('org_admin', 'org_member')),
  token_hash text not null unique,
  token_prefix text not null,
  invited_by_user_id uuid references user_account(id) on delete set null,
  expires_at timestamptz not null,
  accepted_at timestamptz,
  revoked_at timestamptz,
  created_at timestamptz not null default now(),
  check (expires_at > created_at),
  check (position('@' in email) > 1)
);

create index if not exists idx_org_invite_org_email on org_invite(organization_id, email);

insert into org_membership (user_id, organization_id, role)
select id, organization_id,
  case when role = 'admin' then 'org_admin' else 'org_member' end
from user_account
on conflict do nothing;
```

- [ ] **Step 2: Migrate locally** — expect success; `\d org_membership`

- [ ] **Step 3: Commit**

```bash
git commit -m "feat(db): org membership invites and active organization session"
```

---

### Task 2: Session binds active organization

**Files:**
- Modify: `crates/queria-db/src/repositories/auth.rs`, `types.rs`
- Modify: `crates/queria-api/src/http/auth.rs`
- Optionally tiny `OrgRole` in existing `permissions.rs` / auth mod (not a new crate file unless needed)

**Produces:** `AuthenticatedSession { user_id, active_organization_id, is_platform_super_admin, … }`

- [ ] **Step 1: Resolve active org on login/session load**

```text
active_organization_id =
  membership.organization_id if exactly one membership
  else user.organization_id if membership/legacy aligned
  else None  -- super-admin with no membership: org routes only
```

Persist on `user_session` when issuing session.

- [ ] **Step 2: Helpers**

```rust
fn require_active_org(session) -> Result<Uuid, 403>
fn require_platform_super_admin(session) -> Result<(), 403>
```

- [ ] **Step 3: Smoke test** — login returns session that scopes list_projects to user’s org (or unit on resolver)

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(auth): bind session to active organization"
```

---

### Task 3: Orgs + invites API and Admin UI

**Files:**
- Create: `crates/queria-api/src/http/orgs.rs` (**includes** invites + members handlers)
- Modify: `app.rs`, `http/mod.rs`
- Create: `admin/src/pages/orgs/index.astro`, `admin/src/pages/invites/accept.astro`
- Modify: `AdminLayout.astro`, `lib/api.ts`

**API:**

| Method | Path | Role |
|---|---|---|
| POST/GET | `/api/v1/orgs` | super_admin |
| POST | `/api/v1/orgs/{slug}/invites` | org_admin or super_admin |
| GET | `/api/v1/orgs/current/members` | org_admin |
| POST | `/api/v1/invites/accept` | public |

Invite token: sha256 hash + prefix (same pattern as agent tokens).  
On create org / invite: `tracing::info!(email, accept_url, "org invite")` and return `token` once in JSON to caller. **No** `InviteMailer` trait.

- [ ] **Step 1: Implement create org + invite + accept**

Accept: create user if needed; insert membership; set `user_account.organization_id`; reject if user already has another org membership.

- [ ] **Step 2: API tests**

```rust
#[tokio::test]
async fn create_org_requires_super_admin() { /* 403 without flag */ }

#[tokio::test]
async fn accept_invite_isolates_orgs() {
  // user in A cannot accept invite to B (v1 single membership)
}
```

- [ ] **Step 3: Admin orgs + accept pages**

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(api): create organization and email invite accept"
```

---

### Task 4: Enforce org on tenant APIs + agent tokens + Qdrant home filter

**Files:**
- Modify handlers under `crates/queria-api/src/http/` (projects, sources, tokens, jobs, knowledge, approvals, dashboard, retrieval, …)
- Modify repositories to take/filter `organization_id`
- Modify token mint to bind org
- Modify `queria-search` / upsert path: payload + filter `organization_id = home` if not already
- No grant merge; no new CLI verb — backfill via one-off if needed during ops

- [ ] **Step 1: Inventory handlers; add active_org filter**

If `active_organization_id` is None → 403 on tenant routes.

- [ ] **Step 2: Agent token mint/list** only projects in active org; store organization_id on token permissions/record

- [ ] **Step 3: Qdrant** ensure write/search use home org id

- [ ] **Step 4: Leakage smoke**

```rust
// Session/token org A cannot list or retrieve org B project
// Super-admin without membership cannot GET tenant project detail
```

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(api): enforce organization isolation on tenant surfaces"
```

---

### Task 5: Docs + prod bootstrap checklist

**Files:**
- `docs/PRODUCT.md`, `docs/HANDOFF.md`, `docs/runbooks/onboarding.md`
- agent-setup markdown if it claims single-global-tenant assumptions

- [ ] **Step 1: Document v1 multi-org**

```text
Super-admin: POST /orgs → invite token
Accept invite → org admin
All data filtered by organization
No cross-org share in v1
```

- [ ] **Step 2: Ops checklist**

```sql
update user_account set is_platform_super_admin = true
where lower(email) = lower('<super-admin-email>');
```

E2E: create `team-b` → accept invite → create project → prove token B ≠ data A.

- [ ] **Step 3: Commit**

```bash
git commit -m "docs: multi-org isolation MVP product and onboarding"
```

---

## Verification matrix

| Check | Action |
|---|---|
| Migrate | `queria-cli database migrate` |
| Unit/API | `cargo test -p queria-api -p queria-db -p queria-core` (+ search if touched) |
| Leakage | Task 4 smoke: A cannot read B |
| Admin | `/admin/orgs`, invite accept, projects stay org-local |
| MCP | token org A only sees A projects |
| Prod | migrate; flag super-admin; create Team B; isolation check |

## Risk notes

- Missing one handler org filter = leak — Task 4 inventory is the critical path.
- Keep `user_account.organization_id` NOT NULL FK; sync on invite accept; drop column later (out of plan).
- Invite token in API response is sensitive; only return to super-admin/org_admin who created it; never log raw token at info if avoidable (prefer log accept_path only + return token in body).

## Out of this plan (spec appendices)

- Share grants API/UI/MCP `shared` markers  
- Per-org git allowlist  
- org_member vs org_admin power split  
- SMTP mailer  
- Multi-org membership + switcher  
- Dedicated Qdrant backfill CLI  

---

## Spec coverage

| Spec v1 | Task |
|---|---|
| Membership + session | 1–2 |
| Create org + invite accept | 3 |
| Enforce isolation + tokens | 4 |
| Qdrant home filter | 4 |
| Docs + bootstrap | 5 |
| Share grants / per-org git | **out** (appendices) |
