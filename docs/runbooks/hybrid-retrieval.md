# Hybrid Retrieval Runbook

## Retrieval Path

Queria uses PostgreSQL as the source of truth and combines:

- PostgreSQL full-text search for lexical candidates.
- Voyage `voyage-4` embeddings for semantic query/document vectors.
- Qdrant for vector candidate ids and scores.
- Reciprocal rank fusion for final ranking.
- PostgreSQL hydration and authorization for final context items.

Qdrant stores only vector payload metadata. Final content, citation, project/global scope, and access control are always checked in PostgreSQL.

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
rtk infisical run --env=dev -- cargo run -p queria-cli -- eval run --project fjulian-me
```

Admin API checks use the same retrieval path and require a valid `queria_session` cookie:

```bash
rtk curl -sS -X POST http://127.0.0.1:17671/api/v1/projects/fjulian-me/retrieval/probe \
  -H 'content-type: application/json' \
  -H 'cookie: queria_session=<session-token>' \
  -d '{"query":"Astro markdown content flow","include_global":true,"limit":5}'

rtk curl -sS -X POST http://127.0.0.1:17671/api/v1/projects/fjulian-me/evaluations/run \
  -H 'cookie: queria_session=<session-token>'

rtk curl -sS http://127.0.0.1:17671/api/v1/projects/fjulian-me/evaluations/latest \
  -H 'cookie: queria_session=<session-token>'
```

Pass criteria:

- migrations are applied
- embedding status command returns JSON
- at least one chunk is `ready`
- retrieval probe returns at least one cited item
- retrieval diagnostics include lexical and semantic candidate counts
- evaluation report has `passed=true` for the project baseline
- evaluation API persists an `evaluation_report` row with the full JSON report

## Evaluation Baseline

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

Admin UI reads persisted reports from:

- `POST /api/v1/projects/:slug/evaluations/run`
- `GET /api/v1/projects/:slug/evaluations`
- `GET /api/v1/projects/:slug/evaluations/latest`
