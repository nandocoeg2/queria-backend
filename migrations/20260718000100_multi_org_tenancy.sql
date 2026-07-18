-- Multi-org isolation MVP: membership, invites, super-admin flag, session active org.
-- Membership is the access source of truth; keep user_account.organization_id NOT NULL.
-- Invite tokens are stored as hash + prefix only (never plaintext).

alter table user_account
  add column if not exists is_platform_super_admin boolean not null default false;

create table if not exists org_membership (
  user_id uuid not null references user_account(id) on delete cascade,
  organization_id uuid not null references organization(id) on delete cascade,
  role text not null check (role in ('org_admin', 'org_member')),
  created_at timestamptz not null default now(),
  primary key (user_id, organization_id)
);

-- v1: at most one org per user (enables isolation without a switcher).
create unique index if not exists idx_org_membership_one_org_per_user
  on org_membership (user_id);

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

-- Backfill: one membership per existing user from legacy user_account.organization_id.
-- Map setup role `admin` -> org_admin; everything else -> org_member.
insert into org_membership (user_id, organization_id, role)
select id, organization_id,
  case when role = 'admin' then 'org_admin' else 'org_member' end
from user_account
on conflict do nothing;
