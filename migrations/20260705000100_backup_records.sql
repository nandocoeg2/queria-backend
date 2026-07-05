do $$ begin
  create type backup_status as enum ('running', 'succeeded', 'failed');
exception
  when duplicate_object then null;
end $$;

create table if not exists backup_record (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid references organization(id) on delete cascade,
  backup_type text not null,
  status backup_status not null default 'running',
  manifest_key text,
  artifact_keys text[] not null default '{}',
  checksums jsonb not null default '{}',
  size_bytes bigint not null default 0,
  error_message text,
  started_at timestamptz not null default now(),
  completed_at timestamptz,
  created_at timestamptz not null default now()
);

create index if not exists idx_backup_record_org_type
  on backup_record(organization_id, backup_type, created_at desc);
