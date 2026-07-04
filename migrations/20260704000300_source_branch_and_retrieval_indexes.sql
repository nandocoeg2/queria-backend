alter table source_document
  add column if not exists branch text;

create index if not exists idx_source_document_project_kind_created
  on source_document(project_id, kind, created_at desc);

create index if not exists idx_chunk_source_document
  on chunk(source_document_id);

create index if not exists idx_chunk_created
  on chunk(created_at desc);
