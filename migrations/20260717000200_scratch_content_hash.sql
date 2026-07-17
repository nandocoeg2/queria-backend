-- Dual-lane Slice A: idempotent scratch writes (IMP-22).
-- content_hash stores normalized-body SHA-256 for project-scoped scratch only.
-- Uniqueness is partial: same body may exist as approved AND as scratch independently
-- (index_memory must never mutate trusted/approved items).

alter table knowledge_item
  add column if not exists content_hash text;

create unique index if not exists idx_knowledge_item_scratch_content_hash
  on knowledge_item (project_id, content_hash)
  where status = 'scratch'
    and project_id is not null
    and content_hash is not null;
