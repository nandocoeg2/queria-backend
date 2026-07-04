create extension if not exists pgcrypto;

create type knowledge_scope as enum ('global', 'project');
create type knowledge_status as enum ('draft', 'proposed', 'approved', 'rejected', 'deprecated', 'superseded');
create type source_kind as enum ('git_repo', 'markdown_docs', 'manual_note', 'incident_report', 'sop', 'config');
create type approval_status as enum ('pending', 'approved', 'rejected');
create type ingestion_status as enum ('queued', 'running', 'succeeded', 'failed', 'cancelled');

create table organization (
  id uuid primary key default gen_random_uuid(),
  slug text not null unique,
  name text not null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  check (slug ~ '^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$')
);

create table user_account (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization(id) on delete cascade,
  email text not null,
  password_hash text not null,
  role text not null default 'admin',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (organization_id, email),
  check (position('@' in email) > 1)
);

create table project (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization(id) on delete cascade,
  slug text not null,
  name text not null,
  description text,
  default_embedding_model text not null default 'voyage-4',
  include_global_default boolean not null default true,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (organization_id, slug),
  check (slug ~ '^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$')
);

create table source_document (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization(id) on delete cascade,
  project_id uuid references project(id) on delete cascade,
  kind source_kind not null,
  uri text not null,
  title text not null,
  source_path text,
  commit_sha text,
  content_hash text not null,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (organization_id, project_id, uri, content_hash)
);

create table knowledge_item (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization(id) on delete cascade,
  project_id uuid references project(id) on delete cascade,
  source_document_id uuid references source_document(id) on delete set null,
  scope knowledge_scope not null,
  status knowledge_status not null default 'draft',
  title text not null,
  body text not null,
  category text not null,
  tags text[] not null default '{}',
  supersedes_id uuid references knowledge_item(id) on delete set null,
  approved_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  check ((scope = 'global' and project_id is null) or (scope = 'project' and project_id is not null))
);

create table chunk (
  id uuid primary key default gen_random_uuid(),
  knowledge_item_id uuid not null references knowledge_item(id) on delete cascade,
  source_document_id uuid references source_document(id) on delete set null,
  chunk_index integer not null,
  body text not null,
  token_count integer not null default 0,
  embedding_model text not null default 'voyage-4',
  embedding_version text not null default 'v1',
  content_hash text not null,
  qdrant_point_id uuid,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  unique (knowledge_item_id, chunk_index),
  check (chunk_index >= 0),
  check (token_count >= 0)
);

create table approval (
  id uuid primary key default gen_random_uuid(),
  knowledge_item_id uuid not null references knowledge_item(id) on delete cascade,
  requested_by text not null,
  reviewer_user_id uuid references user_account(id) on delete set null,
  status approval_status not null default 'pending',
  reason text,
  policy_snapshot jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  decided_at timestamptz
);

create table agent_token (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization(id) on delete cascade,
  project_id uuid references project(id) on delete cascade,
  name text not null,
  token_prefix text not null,
  token_hash text not null unique,
  allow_global_knowledge boolean not null default false,
  permissions jsonb not null,
  expires_at timestamptz,
  revoked_at timestamptz,
  last_used_at timestamptz,
  created_at timestamptz not null default now(),
  unique (organization_id, token_prefix)
);

create table ingestion_job (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references organization(id) on delete cascade,
  project_id uuid references project(id) on delete cascade,
  source_document_id uuid references source_document(id) on delete set null,
  status ingestion_status not null default 'queued',
  job_type text not null,
  payload jsonb not null default '{}'::jsonb,
  locked_by text,
  locked_at timestamptz,
  attempts integer not null default 0,
  error_message text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  check (attempts >= 0)
);

create table audit_log (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid references organization(id) on delete set null,
  actor_type text not null,
  actor_id text,
  action text not null,
  resource_type text not null,
  resource_id text,
  ip_hash text,
  user_agent_hash text,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

create index idx_project_organization on project(organization_id);
create index idx_source_document_project on source_document(project_id);
create index idx_knowledge_project_status on knowledge_item(project_id, status);
create index idx_knowledge_global_status on knowledge_item(status) where scope = 'global';
create index idx_chunk_knowledge_item on chunk(knowledge_item_id);
create index idx_approval_status_created on approval(status, created_at);
create index idx_agent_token_hash on agent_token(token_hash);
create index idx_ingestion_job_claim on ingestion_job(status, created_at) where status = 'queued';
create index idx_audit_log_resource on audit_log(resource_type, resource_id, created_at);

