-- Dual-lane Slice A: project-scoped agent scratch memory.
-- Retrieve lane is derived from status (status = scratch => scratch lane).
-- YAGNI: do not add a separate lane column; status alone is enough.

ALTER TYPE knowledge_status ADD VALUE IF NOT EXISTS 'scratch';
