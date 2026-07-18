# Multi-Organization Tenancy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship multi-tenant QuerIa where Team B has isolated projects/ingest/retrieve by default, super-admin creates orgs, org-admins invite members by email, and org-admins can grant **read-only** cross-org shares.

**Architecture:** Single stack; `organization_id` is the hard ACL. Add `org_membership`, email `org_invite`, `knowledge_share_grant`, `org_git_allowlist`. Sessions gain `active_organization_id`. Agent tokens bind to one org. Qdrant points already (or after backfill) filter on `organization_id`. Retrieval merges home org ∪ grant-readable scopes; writes stay home-org only.

**Tech Stack:** Rust workspace (`queria-db`, `queria-api`, `queria-mcp`, `queria-worker`, `queria-search`, `queria-ingestion`, `queria-core`), Postgres migrations, Astro Admin, existing session cookies + agent tokens, Voyage + Qdrant.

**Spec:** [`../specs/2026-07-18-multi-org-tenancy-design.md`](../specs/2026-07-18-multi-org-tenancy-design.md)

## Global Constraints

- Single stack multi-tenant (no per-tenant deploy/DB)
- Super-admin creates orgs only; first org admin + members via **email invite only** (no temp password create)
- Soft isolation: default deny cross-org; explicit **read** grants only; no foreign write
- Scratch lane **never** shared across orgs
- Same Qdrant collection(s); payload + filter `organization_id`
- Per-org git allowlist (plus optional instance defaults)
- Preserve dual-lane (scratch/trusted) **inside** each org
- Leakage tests required before claiming multi-tenant ready
- Super-admin does **not** default-browse tenant knowledge
- Do not dual-write status outside HANDOFF after ship
- Sahara Admin (pure Astro SSR); match existing patterns in `admin/src/`
- Host production may need rsync if GitHub SSH missing (ops note only)

## File map (locked decomposition)

| Area | Files |
|---|---|
| Schema | `migrations/20260718000100_multi_org_tenancy.sql` |
| Domain types | `crates/queria-core/src/auth/` (roles), `crates/queria-db/src/repositories/` |
| Session + auth | `crates/queria-db/src/repositories/auth.rs`, `crates/queria-api/src/http/auth.rs` |
| Orgs/invites/shares APIs | `crates/queria-api/src/http/orgs.rs`, `invites.rs`, `share_grants.rs`, `git_allowlist.rs`; wire in `app.rs` |
| Enforce org on existing APIs | `projects.rs`, `sources.rs`, `tokens.rs`, `approvals.rs`, `dashboard.rs`, `retrieval.rs`, `knowledge_items.rs`, … |
| Retrieval grants | `crates/queria-search/src/retrieval.rs`, `qdrant.rs` |
| MCP token scope | `crates/queria-mcp/src/`, `queria-core` agent token permissions |
| Git allowlist | `crates/queria-ingestion/src/git.rs`, worker job path |
| Admin UI | `admin/src/pages/orgs/`, `members/`, `shares/`, `invites/accept.astro`; nav in `AdminLayout.astro` |
| Email | `crates/queria-core` or `queria-api` mail sink (`log` + optional SMTP env) |
| Tests | db unit/integration + API oneshot + leakage suite |
| Docs | `PRODUCT.md`, `HANDOFF.md`, `runbooks/onboarding.md` |

---

### Task 1: Schema — membership, invites, grants, git allowlist, session active org

**Files:**
- Create: `migrations/20260718000100_multi_org_tenancy.sql`
- Modify: `crates/queria-db/src/migrate.rs` (if migrations are enumerated)
- Test: `cargo test -p queria-db` migrate smoke / `queria-cli database migrate` local

**Interfaces:**
- Produces: tables `org_membership`, `org_invite`, `knowledge_share_grant`, `org_git_allowlist`; columns `user_account.is_platform_super_admin`, `user_session.active_organization_id`

- [ ] **Step 1: Write migration SQL**

```sql
-- 20260718000100_multi_org_tenancy.sql

alter table user_account
  add column if not exists is_platform_super_admin boolean not null default false;

-- Membership (user may belong to multiple orgs later; backfill from user_account.organization_id)
create table if not exists org_membership (
  user_id uuid not null references user_account(id) on delete cascade,
  organization_id uuid not null references organization(id) on delete cascade,
  role text not null check (role in ('org_admin', 'org_member')),
  created_at timestamptz not null default now(),
  primary key (user_id, organization_id)
);

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

create table if not exists knowledge_share_grant (
  id uuid primary key default gen_random_uuid(),
  source_organization_id uuid not null references organization(id) on delete cascade,
  target_organization_id uuid not null references organization(id) on delete cascade,
  scope_type text not null check (scope_type in ('project', 'organization_global')),
  scope_project_id uuid references project(id) on delete cascade,
  created_by_user_id uuid references user_account(id) on delete set null,
  created_at timestamptz not null default now(),
  expires_at timestamptz,
  revoked_at timestamptz,
  check (
    (scope_type = 'project' and scope_project_id is not null)
    or (scope_type = 'organization_global' and scope_project_id is null)
  ),
  check (source_organization_id <> target_organization_id)
);

create index if not exists idx_share_grant_target on knowledge_share_grant(target_organization_id)
  where revoked_at is null;

create table if not exists org_git_allowlist (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization(id) on delete cascade,
  kind text not null check (kind in ('ssh_host', 'ssh_repository', 'path_root')),
  value text not null,
  created_at timestamptz not null default now(),
  unique (organization_id, kind, value)
);

-- Backfill memberships from legacy single-org user_account.organization_id
insert into org_membership (user_id, organization_id, role)
select id, organization_id,
  case when role = 'admin' then 'org_admin' else 'org_member' end
from user_account
on conflict do nothing;

-- Flag first-org admins as platform super-admin when env migration is applied manually,
-- or: update user_account set is_platform_super_admin = true where email = lower(trim(...));
-- Prefer a one-row ops step in Task 7, not hard-coded email in SQL.
```

- [ ] **Step 2: Run migrate locally**

```bash
cd queria/backend
# with local postgres + env
cargo run -p queria-cli -- database migrate
```

Expected: `{"status":"migrated"}` or equivalent success; `\d org_membership` exists.

- [ ] **Step 3: Commit**

```bash
git add migrations/20260718000100_multi_org_tenancy.sql crates/queria-db/src/migrate.rs
git commit -m "feat(db): multi-org membership invites grants git allowlist schema"
```

---

### Task 2: Auth domain — roles, membership repo, session active org

**Files:**
- Create: `crates/queria-core/src/auth/org_roles.rs`
- Modify: `crates/queria-db/src/repositories/auth.rs`, `types.rs`, `mod.rs`
- Modify: `crates/queria-api/src/http/auth.rs` (login sets active org)
- Test: unit tests in `org_roles.rs` + auth repository tests if present

**Interfaces:**
- Produces:
  - `enum OrgRole { OrgAdmin, OrgMember }`
  - `struct OrgMembership { user_id, organization_id, role }`
  - `AuthenticatedSession { user_id, active_organization_id: Option<Uuid>, is_platform_super_admin: bool, org_role: Option<OrgRole> }`
  - `fn require_org_admin(session) -> Result`
  - `fn require_org_member(session) -> Result` (admin or member)
  - `fn require_platform_super_admin(session) -> Result`

- [ ] **Step 1: Add role types + helpers**

```rust
// crates/queria-core/src/auth/org_roles.rs
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgRole {
    OrgAdmin,
    OrgMember,
}

impl OrgRole {
    pub fn can_admin(self) -> bool { matches!(self, Self::OrgAdmin) }
}
```

- [ ] **Step 2: Extend session load to join membership for active_organization_id**

On login / session lookup:

1. Load user (`is_platform_super_admin`, legacy `organization_id`).
2. Load memberships.
3. Choose `active_organization_id`: request body/cookie override if membership exists; else single membership; else legacy `user.organization_id`; else `None` for super-admin-only.
4. Persist `active_organization_id` on `user_session` when creating/refreshing session.
5. Return role for active org from `org_membership`.

- [ ] **Step 3: Failing test first — member of org A cannot use org B session without membership**

```rust
#[test]
fn org_role_admin_can_admin() {
    assert!(OrgRole::OrgAdmin.can_admin());
    assert!(!OrgRole::OrgMember.can_admin());
}
```

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(auth): org roles membership and active organization session"
```

---

### Task 3: Org + invite API (super-admin create org, email invites)

**Files:**
- Create: `crates/queria-api/src/http/orgs.rs`, `invites.rs`
- Create: `crates/queria-api/src/mail.rs` (trait `InviteMailer` with `LogInviteMailer`)
- Modify: `crates/queria-api/src/app.rs`, `http/mod.rs`
- Create: Admin pages `admin/src/pages/orgs/index.astro`, `admin/src/pages/invites/accept.astro`
- Modify: `admin/src/layouts/AdminLayout.astro` (nav Orgs for super-admin)
- Modify: `admin/src/lib/api.ts`

**Interfaces:**
- `POST /api/v1/orgs` body `{ slug, name, first_admin_email }` → org + invite created; email via mailer
- `GET /api/v1/orgs` super-admin list
- `POST /api/v1/orgs/{slug}/invites` `{ email, role }`
- `POST /api/v1/invites/accept` `{ token, password, display_name? }` public
- `GET /api/v1/orgs/current/members` org_admin

- [ ] **Step 1: Invite token issuance**

Reuse agent-token style hashing (sha256 + prefix), 32 random bytes, store hash only.

- [ ] **Step 2: Wire mailer**

```rust
pub trait InviteMailer: Send + Sync {
    fn send_invite(&self, to: &str, accept_url: &str) -> Result<(), QueriaError>;
}

pub struct LogInviteMailer;
impl InviteMailer for LogInviteMailer {
    fn send_invite(&self, to: &str, accept_url: &str) -> Result<(), QueriaError> {
        tracing::info!(%to, %accept_url, "org invite (log sink)");
        Ok(())
    }
}
```

Env later: `QUERIA_SMTP_*` optional; MVP log sink is enough if documented.

- [ ] **Step 3: API tests (oneshot)**

```rust
#[tokio::test]
async fn create_org_requires_super_admin() {
    // session without is_platform_super_admin -> 403
}

#[tokio::test]
async fn accept_invite_creates_membership() {
    // insert invite, POST accept, membership row exists
}
```

- [ ] **Step 4: Admin UI**

- `/admin/orgs`: form slug/name/first_admin_email → POST
- `/admin/invites/accept`: password form + token query param
- `/admin/members`: list + invite form (org_admin)

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(api): create org and email invite accept flow"
```

---

### Task 4: Enforce org scope on all existing Admin/API surfaces

**Files:**
- Modify every handler under `crates/queria-api/src/http/` that loads projects/sources/jobs/tokens/knowledge/approvals/dashboard/retrieval
- Modify `PgProjectRepository` methods to take `organization_id` explicitly where currently inferred only via `user.organization_id`

**Interfaces:**
- Consumes: `AuthenticatedSession.active_organization_id`
- Rule: if `active_organization_id` is None → 403 on tenant routes (super-admin uses org routes only)

- [ ] **Step 1: Inventory — grep handlers using `session.user_id` for list/create**

Ensure each path filters `organization_id = active_org`.

- [ ] **Step 2: Tokens — bind agent_token.permissions / record to organization_id**

Add `organization_id` column on agent_token if missing (or store in permissions JSON). Mint only projects in active org. Reject foreign slugs.

- [ ] **Step 3: Member vs admin**

- `org_member`: read projects/sources/knowledge/playground; deny POST tokens, deny invite, deny share, deny ingest trigger if design says admin-only (lock: org_admin for ingest/tokens/shares).
- `org_admin`: full current powers within org.

- [ ] **Step 4: Leakage test (API)**

```rust
// Create org A+B fixtures; session A must 404/403 on B project slug
```

- [ ] **Step 5: Commit**

```bash
git commit -m "feat(api): enforce active organization on tenant endpoints"
```

---

### Task 5: Qdrant organization payload + retrieval grant merge

**Files:**
- Modify: `crates/queria-search/src/qdrant.rs`, `retrieval.rs`
- Modify: embedding write path (`queria-worker` / search upsert) to always set `organization_id` in payload
- Create: backfill job or CLI `queria-cli qdrant backfill-org-payload --organization <slug>`
- Test: unit tests for filter builder; integration if available

**Interfaces:**
- Produces: `ReadableScope { home_org, grant_org_ids, foreign_project_ids }`
- `fn resolve_readable_scope(pool, home_org) -> ReadableScope` from non-revoked grants
- Qdrant filter: must_match organization_id in allowed set; project filter intersection

- [ ] **Step 1: Ensure upsert payload includes organization_id (UUID string)**

- [ ] **Step 2: Search filter always includes org constraint**

- [ ] **Step 3: Grant-aware retrieve**

```text
home = token.organization_id | session.active_org
grants = active grants where target = home
foreign projects = grants.scope_type=project
foreign globals = grants.scope_type=organization_global -> include those orgs' global trusted only
never return foreign scratch
```

- [ ] **Step 4: Backfill existing points to org `fjulian`**

- [ ] **Step 5: Leakage tests**

1. Token A cannot retrieve B project without grant  
2. Grant A→B project: B can retrieve A's **trusted** only  
3. B cannot index_memory into A's project  

- [ ] **Step 6: Commit**

```bash
git commit -m "feat(search): org filter on vectors and grant-aware retrieve"
```

---

### Task 6: Per-org git allowlist + worker validation

**Files:**
- Modify: `crates/queria-ingestion/src/git.rs` (`GitSecurityPolicy` takes org allowlist rows)
- Modify: worker `jobs.rs` load allowlist for job.organization_id
- Create: API `git_allowlist.rs` + Admin section under org settings or sources
- Migrate: copy instance env allowlists into `org_git_allowlist` for existing org

**Interfaces:**
- `GitSecurityPolicy::from_org_rows(instance_defaults, org_rows)`
- Validate path roots + ssh host/repo as today, union instance + org

- [ ] **Step 1: Policy loads per-org rows**

- [ ] **Step 2: Worker fails closed if org has empty allowlist and instance empty**

- [ ] **Step 3: Seed prod `fjulian` allowlist** for `github.com`, `nandocoeg2/fjulian.me.git`, path_root `/tmp/seed10001` as needed

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(ingestion): per-organization git allowlists"
```

---

### Task 7: Share grants API + Admin UI + MCP list_projects shared markers

**Files:**
- Create: `crates/queria-api/src/http/share_grants.rs`
- Create: `admin/src/pages/shares/index.astro`
- Modify: MCP `list_projects` / tools descriptions
- Docs: PRODUCT, onboarding, HANDOFF, agent-setup markdown section

**Interfaces:**
- `POST /api/v1/share-grants` `{ target_organization_slug, scope_type, scope_project_slug? }`
- `GET /api/v1/share-grants?direction=outbound|inbound`
- `DELETE /api/v1/share-grants/{id}`
- MCP project list items include `"shared": true` when grant-visible foreign

- [ ] **Step 1: API + tests**

- [ ] **Step 2: Admin shares page**

- [ ] **Step 3: MCP retrieve already grant-aware from Task 5; list_projects marks shared**

- [ ] **Step 4: Update docs**

- PRODUCT: multi-org section CURRENT when shipped  
- onboarding: super-admin create org → invite → project  
- HANDOFF: capability matrix  
- agent-setup: tokens are org-scoped  

- [ ] **Step 5: Ops bootstrap super-admin**

```sql
update user_account set is_platform_super_admin = true
where lower(email) = lower('nando@fjulian.id'); -- use real super-admin email
```

- [ ] **Step 6: End-to-end manual checklist**

1. Super-admin creates `team-b`, invite first admin  
2. Accept invite, login as Team B  
3. Create project, (optional) allowlist git, ingest  
4. Token B cannot see `fjulian-me`  
5. Org A grants project to Team B → B retrieve works read-only  
6. B cannot propose_memory into A  

- [ ] **Step 7: Commit**

```bash
git commit -m "feat: share grants UI MCP and multi-org docs"
```

---

## Verification matrix (before claiming done)

| Check | Command / action |
|---|---|
| Migrate | `queria-cli database migrate` |
| Unit | `cargo test -p queria-core -p queria-db -p queria-api -p queria-search -p queria-ingestion` |
| Leakage | dedicated tests Task 4–5 |
| Admin | login, orgs, members, shares, projects |
| MCP | token org A vs B |
| Prod | migrate; flag super-admin; backfill Qdrant payload; seed git allowlist for `fjulian` |

## Risk notes

- **Large change surface:** enforce Task 4 thoroughly; missing one handler = leak.
- **user_account.organization_id** remains as legacy primary org for NOT NULL FK; membership is source of truth for access. Do not delete column in this plan (follow-up).
- **Email:** log sink means first-admin invite URL must be copied from logs in dev; document `accept_url` in API response for super-admin create (include invite accept path + raw token once in API response to super-admin only).

```json
// POST /api/v1/orgs response (super-admin only)
{
  "organization": { "slug": "team-b", "name": "Team B" },
  "invite": {
    "email": "admin@teamb.example",
    "accept_path": "/admin/invites/accept",
    "token": "<once>",
    "expires_at": "..."
  }
}
```

## Out of this plan

- SSO, billing, break-glass super-admin knowledge browse  
- Per-org Voyage keys  
- Dropping `user_account.organization_id`  

---

## Spec coverage self-check

| Spec section | Task |
|---|---|
| Create org + first admin email invite | 3 |
| Member email invites | 3 |
| Membership + active org session | 2 |
| Hard org filter APIs | 4 |
| Agent token org bind | 4 |
| Soft share grants | 5 + 7 |
| Qdrant organization_id | 5 |
| Per-org git allowlist | 6 |
| Admin UI orgs/members/shares | 3 + 7 |
| Leakage tests | 4, 5 |
| Migration from fjulian | 1 backfill + 5 backfill + 7 ops |
| No foreign write | 5 MCP + 4 API |

Placeholder scan: no TBD. Types: `OrgRole`, `AuthenticatedSession.active_organization_id`, grant `scope_type` consistent across tasks.
