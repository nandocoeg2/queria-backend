do $$
begin
  create type embedding_status as enum ('pending', 'processing', 'ready', 'failed', 'stale');
exception
  when duplicate_object then null;
end
$$;

alter table chunk
  add column if not exists search_title text not null default '',
  add column if not exists search_vector tsvector
    generated always as (
      setweight(to_tsvector('simple', coalesce(search_title, '')), 'A') ||
      setweight(to_tsvector('simple', coalesce(body, '')), 'B')
    ) stored,
  add column if not exists embedding_provider text not null default 'voyage',
  add column if not exists embedding_dimension integer not null default 1024,
  add column if not exists embedding_profile_version text not null default 'voyage-4-1024-v1',
  add column if not exists embedding_content_hash text,
  add column if not exists embedding_status embedding_status not null default 'pending',
  add column if not exists embedding_error text,
  add column if not exists embedding_attempts integer not null default 0,
  add column if not exists embedded_at timestamptz,
  add column if not exists embedding_updated_at timestamptz not null default now();

alter table chunk
  add constraint chunk_embedding_dimension_positive check (embedding_dimension > 0),
  add constraint chunk_embedding_attempts_non_negative check (embedding_attempts >= 0);

update chunk c
set search_title = k.title,
    embedding_status = case
      when k.status = 'approved' then 'pending'::embedding_status
      else 'stale'::embedding_status
    end,
    embedding_updated_at = now()
from knowledge_item k
where k.id = c.knowledge_item_id;

create index if not exists idx_chunk_search_vector
  on chunk using gin(search_vector);

create index if not exists idx_chunk_embedding_claim
  on chunk(embedding_status, embedding_updated_at, id)
  where embedding_status in ('pending', 'failed', 'stale');

create unique index if not exists idx_chunk_qdrant_point_id
  on chunk(qdrant_point_id)
  where qdrant_point_id is not null;

create unique index if not exists idx_ingestion_job_one_active_embedding_project
  on ingestion_job(project_id, job_type)
  where project_id is not null
    and source_document_id is null
    and job_type = 'embedding_backfill'
    and status in ('queued', 'running');
