-- Hybrid local multi-git index-here: items pending human review before trusted.
-- Lane/retrieval semantics derived from status (same YAGNI as scratch).
-- YAGNI: do not add a separate lane or review column; status alone is enough.

ALTER TYPE knowledge_status ADD VALUE IF NOT EXISTS 'needs_review';
