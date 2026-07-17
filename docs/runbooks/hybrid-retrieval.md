# Hybrid Retrieval Runbook

> Status: CURRENT for hybrid retrieval ops; evaluation is CLI-only (Admin evaluation HTTP removed P2).
> Last verified: 2026-07-18.
> Current evidence: [`../HANDOFF.md`](../HANDOFF.md).

## Retrieval Path

Queria uses PostgreSQL as the source of truth. Shared pipeline for MCP, API, CLI, and Admin Playground:

1. Authorize principal (session user or agent token).
2. Candidate **pool** size = `min(limit * candidate_multiplier, candidate_cap)` (oversample; not final `limit`).
3. Parallel: PostgreSQL FTS (lexical) + Voyage `voyage-4` embed query.
4. Qdrant dense vector search (or empty path → lexical fallback mode).
5. **RRF over the pool** (fuse then keep pool size, not final `limit`).
6. PostgreSQL **hydrate** pool (bodies, lane/status, citations) + authorize filters.
7. **Rerank** (Voyage `rerank-2.5`) when enabled → top_k = request `limit`.
8. **Compress** near-duplicates when enabled (prefer **trusted** over **scratch**).
9. Response items + diagnostics: `mode`, candidate counts, `rerank_applied`, `compress_dropped`, `latency_ms`.

Qdrant stores only vector payload metadata. Final content, citation, project/global scope, and access control are always checked in PostgreSQL.

```text
pool → RRF → hydrate → rerank → compress → response
```

### Rerank and compress env

| Env | Default | Meaning |
|---|---|---|
| `QUERIA_RERANK_ENABLED` | `true` | Default when request omits `rerank`. Skipped if no `VOYAGE_API_KEY`. |
| `QUERIA_RERANK_MODEL` | `rerank-2.5` | Voyage rerank model |
| `QUERIA_RERANK_TIMEOUT_SECONDS` | `30` | Rerank HTTP timeout |
| `QUERIA_COMPRESS_ENABLED` | `true` | Default when request omits `compress` |

Per-call overrides (optional bools; `null`/omit → use env default):

- API retrieve body and project probe: `rerank`, `compress`
- MCP `retrieve_context` / `search_knowledge` tool args: same
- CLI: `--rerank` / `--compress` (probe)
- Admin Playground form toggles → `POST /api/v1/projects/{slug}/retrieval/probe`

Operator probe and CLI probe default `include_scratch=false`. Agent MCP retrieve defaults `include_scratch=true`.

### Fail-open (rerank)

Rerank **never** fails the retrieve. On error, timeout, empty response, missing API key, or explicit `rerank=false`:

- Keep hydrated RRF order (clamped to `limit`)
- Set `rerank_applied=false`
- Continue to compress if enabled
- Log sanitized warnings only (no secrets / raw provider bodies)

Compress off or no near-dups → `compress_dropped=0`.

## Embedding State

Each chunk has embedding state:

- `pending`: needs embedding.
- `processing`: claimed by a worker.
- `ready`: vector exists in Qdrant for the current profile.
- `failed`: retryable provider/index failure.
- `stale`: profile or source changed.

Backfill claims `pending`, `failed`, and `stale` chunks whose profile does not match the configured `QUERIA_EMBEDDING_PROFILE_VERSION`.

## Rate Limit Behavior

Transient provider/index errors are retryable:

- Voyage `429 Too Many Requests`
- provider `5xx`
- timeout or connection-level infrastructure errors

The worker marks the batch `failed`, releases the job back to `queued`, and sets `ingestion_job.retry_after_at`. This prevents a single rate-limit response from making the whole project backfill terminal failed.

Non-retryable errors still fail the job:

- invalid config
- authentication failure
- permission failure
- validation errors
- cancellation

## Qdrant Collection

Default local collection:

```text
queria_local_chunks_v1
```

Infisical dev currently uses a separate collection:

```text
queria_dev_chunks_v1
```

Collection identity is configured by:

- `QUERIA_QDRANT_URL`
- `QUERIA_QDRANT_API_KEY`
- `QUERIA_QDRANT_COLLECTION`
- `QUERIA_QDRANT_VECTOR_NAME`

## Smoke Commands

Run all local smoke checks:

```bash
rtk bash scripts/smoke-hybrid-retrieval.sh fjulian-me "Astro markdown content flow"
```

Manual checks:

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- embeddings status --project fjulian-me
rtk infisical run --env=dev -- cargo run -p queria-cli -- retrieval probe --project fjulian-me --query "Astro markdown content flow" --limit 5
# Optional flag overrides (defaults follow env):
rtk infisical run --env=dev -- cargo run -p queria-cli -- retrieval probe --project fjulian-me --query "Astro markdown content flow" --limit 5 --rerank=false --compress=true
rtk infisical run --env=dev -- cargo run -p queria-cli -- eval run --project fjulian-me
```

Admin API checks use the same retrieval path and require a valid `queria_session` cookie. Admin UI path: `/admin/playground`.

```bash
rtk curl -sS -X POST http://127.0.0.1:17671/api/v1/projects/fjulian-me/retrieval/probe \
  -H 'content-type: application/json' \
  -H 'cookie: queria_session=<session-token>' \
  -d '{"query":"Astro markdown content flow","include_global":true,"limit":5,"rerank":true,"compress":true}'
```

Expect diagnostics fields on success: `retrieval.mode`, `lexical_candidates`, `semantic_candidates`, `rerank_applied`, `compress_dropped`, `latency_ms`.

Evaluation (CLI only):

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- eval run --project fjulian-me
```

Pass criteria:

- migrations are applied
- embedding status command returns JSON
- at least one chunk is `ready`
- retrieval probe returns at least one cited item
- retrieval diagnostics include lexical and semantic candidate counts
- with Voyage key and defaults on: live probe can show `rerank_applied=true` (still pass if fail-open `false` after provider error)
- CLI evaluation report has `passed=true` for the project baseline

## Evaluation Baseline

Current verified baseline is 2/3. `deployment and site build notes` returns zero
lexical candidates because `websearch_to_tsquery('simple', ...)` requires every
lexeme, including `and`. Do not treat the runbook's passing target below as the
current observed state until Phase 1 of the active roadmap is complete.

Golden questions live in:

```text
tests/golden_questions/<project-slug>.jsonl
```

Each line describes one query:

```json
{"id":"fjulian-me-astro-content","project_slug":"fjulian-me","query":"Astro markdown content flow","include_global":true,"expected_scope":["project"],"expected_citations":[],"minimum_items":1}
```

Run:

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- eval run --project fjulian-me
```

The report includes:

- pass/fail per question
- returned item count
- expected scope hits
- expected citation hits
- retrieval mode and candidate counts
- regression score from 0.0 to 1.0

Operator path is CLI only (`queria-cli eval run`). Evaluation HTTP routes and the Admin evaluations page were removed in SIMPLIFICATION P2. The dashboard may still show the latest report row if CLI persisted one.
