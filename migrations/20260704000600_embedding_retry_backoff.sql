alter table ingestion_job
  add column if not exists retry_after_at timestamptz not null default now();

update ingestion_job
set retry_after_at = coalesce(retry_after_at, now())
where retry_after_at is null;

create index if not exists idx_ingestion_job_embedding_retry_ready
  on ingestion_job (retry_after_at, created_at, id)
  where status = 'queued'
    and job_type in ('embedding_backfill', 'qdrant_delete');
