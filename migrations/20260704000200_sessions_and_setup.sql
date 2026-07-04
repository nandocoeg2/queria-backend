create table setup_state (
  id boolean primary key default true,
  setup_token_hash text not null,
  consumed_at timestamptz,
  consumed_by_user_id uuid references user_account(id) on delete set null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  check (id)
);

create table user_session (
  id uuid primary key default gen_random_uuid(),
  user_id uuid not null references user_account(id) on delete cascade,
  token_prefix text not null,
  token_hash text not null unique,
  expires_at timestamptz not null,
  revoked_at timestamptz,
  last_seen_at timestamptz,
  created_at timestamptz not null default now(),
  check (expires_at > created_at)
);

create index idx_user_session_token_hash on user_session(token_hash);
create index idx_user_session_user_id on user_session(user_id, created_at desc);
