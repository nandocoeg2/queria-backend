alter table source_document
  add column if not exists source_root_id uuid references source_document(id) on delete cascade,
  add column if not exists is_active boolean not null default true,
  add column if not exists indexed_at timestamptz;

alter table knowledge_item
  add column if not exists stable_key text,
  add column if not exists generated_by text;

alter table ingestion_job
  add column if not exists started_at timestamptz,
  add column if not exists finished_at timestamptz,
  add column if not exists cancel_requested_at timestamptz,
  add column if not exists result jsonb not null default '{}'::jsonb,
  add column if not exists retry_of_id uuid references ingestion_job(id) on delete set null;

create unique index if not exists idx_ingestion_job_one_active_per_source
  on ingestion_job(source_document_id, job_type)
  where source_document_id is not null and status in ('queued', 'running');

create unique index if not exists idx_source_document_active_child_path
  on source_document(source_root_id, source_path)
  where source_root_id is not null and is_active;

create unique index if not exists idx_knowledge_item_active_stable_key
  on knowledge_item(source_document_id, stable_key)
  where stable_key is not null and status in ('draft', 'proposed', 'approved');

create index if not exists idx_source_document_root_active
  on source_document(source_root_id, is_active);

create index if not exists idx_ingestion_job_source_created
  on ingestion_job(source_document_id, created_at desc);

