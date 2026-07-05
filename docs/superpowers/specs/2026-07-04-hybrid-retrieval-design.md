# Voyage-4, Qdrant, and Postgres FTS Hybrid Retrieval

> Status: PARTIAL. Core design is implemented. Remaining: relaxed lexical candidate generation, shared/persisted CLI evaluation, completion of the real backfill, and final acceptance.
> Superseded execution plan: [`../plans/2026-07-05-queria-end-to-end.md`](../plans/2026-07-05-queria-end-to-end.md).

## Goal

Replace Queria's temporary substring retrieval with production-oriented hybrid
retrieval while preserving Postgres as the authorization and lifecycle source
of truth.

The MVP combines:

- Voyage `voyage-4` dense embeddings;
- Qdrant dense vector candidate search;
- PostgreSQL full-text candidate search;
- rank-based fusion in Rust;
- final Postgres hydration and authorization.

Reranking and Qdrant sparse vectors are outside this phase.

## Fixed Decisions

| Concern | Decision |
|---|---|
| Embedding provider | Voyage AI |
| Model | `voyage-4` |
| Output dimension | 1024 |
| Input type while indexing | `document` |
| Input type while searching | `query` |
| Qdrant collection | `queria_{environment}_chunks_v1` |
| Named vector | `dense_v1` |
| Distance | Cosine |
| Text retrieval | PostgreSQL FTS using `simple` |
| Fusion | Reciprocal Rank Fusion, `k = 60` |
| Source of truth | PostgreSQL |
| Secret delivery in local/dev | Infisical Cloud US CLI injection |
| Production secret delivery | Self-hosted Infisical machine identity |
| Local fallback | `.env` |

All values that affect cost, latency, or index compatibility remain
parameterized.

## Secret Handling

Queria never reads Infisical through an SDK. Binaries only read environment
variables, allowing the same binaries to run through Infisical Cloud,
self-hosted Infisical, Docker secrets, or `.env`.

Required variables:

```text
VOYAGE_API_KEY
QDRANT_API_KEY
```

`QDRANT_API_KEY` is optional when the local Qdrant instance has authentication
disabled. Secret values must never be written to logs, database rows, audit
metadata, job payloads, or command output.

Local commands use:

```text
infisical run --env=dev -- cargo run -p queria-worker
```

The repository may contain Infisical project metadata, but never secret values
or exported secret files.

## Data Model

The existing `chunk` table remains the canonical chunk manifest. Add:

- `search_title text not null default ''`;
- `search_vector tsvector`, generated from weighted title and body using the
  `simple` configuration;
- `embedding_provider text`;
- `embedding_dimension integer`;
- `embedding_profile text`;
- `embedding_content_hash text`;
- `embedding_status`: `pending`, `processing`, `ready`, `failed`, `stale`;
- `embedding_error text`;
- `embedded_at timestamptz`.

Create a GIN index on `search_vector` and a partial claim index for chunks that
need embeddings.

The current `embedding_model`, `embedding_version`, and `qdrant_point_id`
columns remain authoritative compatibility fields.

The existing `ingestion_job` table is reused for:

- `embedding_backfill`;
- `qdrant_delete`.

Deletion jobs are inserted in the same Postgres transaction that deprecates or
supersedes source knowledge.

## Qdrant Schema

Each environment has a separate collection because dev, staging, and
production currently share one Qdrant cluster:

```text
queria_dev_chunks_v1
queria_staging_chunks_v1
queria_prod_chunks_v1
```

Each collection contains named vector `dense_v1`:

```text
size: 1024
distance: cosine
point id: chunk UUID
```

Payload contains identifiers and filter fields only:

- `organization_id`;
- `project_id`;
- `chunk_id`;
- `knowledge_item_id`;
- `source_document_id`;
- `scope`;
- `status`;
- `category`;
- `embedding_provider`;
- `embedding_model`;
- `embedding_dimension`;
- `embedding_version`;
- `content_hash`.

The full chunk body is not duplicated into Qdrant. Payload indexes are created
for organization, project, scope, status, category, and embedding version.

Within an environment, one collection is shared across organizations and
projects. Collections are never shared across environments. Authorization and
scope are enforced by Qdrant filters and then verified again during Postgres
hydration.

## Embedding Flow

After approved chunks are committed:

1. Mark new or changed chunks `pending`.
2. Claim a bounded batch with `FOR UPDATE SKIP LOCKED`.
3. Build embedding text from project, source path, title, category, and chunk
   body.
4. Call Voyage with `input_type=document`, 1024 dimensions, and float output.
5. Upsert deterministic Qdrant points using the chunk UUID.
6. Mark chunks `ready` only after Qdrant confirms the upsert.
7. Store model, dimension, profile, version, content hash, point ID, and
   timestamp.

Default batch size is 128. Timeouts, concurrency, batch size, and retry limits
are configurable.

Voyage `429` and transient `5xx` responses use bounded exponential backoff with
jitter. Authentication and validation failures are not retried. Exhausted
attempts mark chunks `failed` and preserve a sanitized error.

Backfill is resumable. A chunk is skipped only when its current content hash
and complete embedding compatibility tuple already match a `ready` point.

## Update and Delete Consistency

For changed chunks:

1. New Postgres state becomes canonical.
2. The previous Qdrant point is replaced idempotently using the same chunk ID
   when possible.
3. A changed content hash always forces re-embedding.

For removed or deprecated chunks:

1. Postgres status changes first.
2. A `qdrant_delete` job is created transactionally.
3. The worker deletes the Qdrant point idempotently.

Qdrant is never trusted to return final content. Every vector candidate is
hydrated from Postgres and discarded unless the chunk still exists, is
approved, belongs to the authenticated organization, and matches the requested
global/project scope. This prevents stale Qdrant points from leaking data while
asynchronous cleanup is pending.

## PostgreSQL Full-Text Search

The generated search vector is equivalent to:

```sql
setweight(to_tsvector('simple', coalesce(search_title, '')), 'A') ||
setweight(to_tsvector('simple', coalesce(body, '')), 'B')
```

Queries use `websearch_to_tsquery('simple', query)` and `ts_rank_cd`.

The `simple` configuration is intentional: engineering knowledge contains
identifiers, file names, acronyms, commands, and mixed Indonesian/English text
that should not be aggressively stemmed.

## Retrieval Flow

`retrieve_context` and `search_knowledge` use the same retrieval service:

1. Validate organization, project, permissions, limit, and query length.
2. Embed the query with `input_type=query`.
3. Fetch dense candidates from Qdrant with organization and scope filters.
4. Fetch lexical candidates from Postgres FTS with identical scope rules.
5. Fuse ranked lists using Reciprocal Rank Fusion.
6. Hydrate fused chunk IDs from Postgres.
7. Recheck lifecycle, organization, project/global applicability, and source.
8. Return final citations and scores.

Each backend fetches at least `3 * requested_limit`, subject to a configurable
cap. Duplicate chunk IDs are merged before hydration.

Response diagnostics include:

- `retrieval_mode`: `hybrid`, `fts_fallback`, or `vector_only`;
- embedding model and version;
- elapsed time per retrieval stage;
- candidate counts.

Raw vectors and queries are not logged.

## Degraded Operation

Postgres FTS remains available if Voyage or Qdrant is unavailable. In this
case, retrieval returns authorized FTS results with
`retrieval_mode=fts_fallback` and emits a structured warning.

An FTS failure does not silently return vector-only results unless Postgres
hydration remains healthy. Postgres failure fails the request closed because
authorization and lifecycle checks cannot be completed.

## API and Admin Operations

Add protected operations:

```text
POST /api/v1/embedding-jobs/backfill
GET  /api/v1/embedding-jobs
GET  /api/v1/embedding-jobs/{id}
POST /api/v1/embedding-jobs/{id}/retry
GET  /api/v1/retrieval/status
```

Backfill supports optional project, source, model, version, and force filters.
Only administrators can trigger or retry embedding work.

Add CLI equivalents:

```text
queria-cli embeddings backfill
queria-cli embeddings status
queria-cli retrieval probe
```

## Configuration

Add parameterized environment variables:

```text
QUERIA_EMBEDDING_PROVIDER=voyage
QUERIA_EMBEDDING_MODEL=voyage-4
QUERIA_EMBEDDING_DIMENSION=1024
QUERIA_EMBEDDING_PROFILE=managed-balanced
QUERIA_EMBEDDING_VERSION=v1
QUERIA_EMBEDDING_BATCH_SIZE=128
QUERIA_EMBEDDING_TIMEOUT_SECONDS=30
QUERIA_EMBEDDING_MAX_RETRIES=4
QUERIA_QDRANT_COLLECTION=queria_{environment}_chunks_v1
QUERIA_QDRANT_VECTOR_NAME=dense_v1
QUERIA_HYBRID_RRF_K=60
QUERIA_RETRIEVAL_CANDIDATE_MULTIPLIER=3
QUERIA_RETRIEVAL_MAX_CANDIDATES=100
```

Configuration validation fails at startup when dimensions, profile, collection,
or vector name are incompatible. Missing Voyage credentials prevent embedding
work but do not prevent the API from serving FTS fallback.

## Testing

Use `mockall` for Voyage, Qdrant, and repository boundaries.

Required coverage:

- document/query input types;
- deterministic embedding text;
- batching and resumable backfill;
- retry classification and sanitized errors;
- idempotent Qdrant upsert/delete;
- Qdrant payload and scope filters;
- FTS ranking;
- RRF ordering and deduplication;
- global plus project scope;
- stale and unauthorized candidate rejection;
- FTS fallback;
- Postgres fail-closed behavior;
- API authentication and admin authorization.

Integration tests run against the Docker Compose Postgres and Qdrant services.
A real Voyage smoke test embeds a small fixture and must not print credentials.

## Acceptance Criteria

- Existing approved `fjulian-me` chunks can be backfilled without duplicates.
- Every ready Postgres chunk has a matching Qdrant point and compatibility
  metadata.
- Hybrid retrieval returns valid Postgres citations only.
- Global/project and organization boundaries are preserved.
- Re-running backfill skips compatible ready chunks.
- Changed chunks are re-embedded.
- Deprecated and deleted chunks cannot appear in final results.
- Voyage or Qdrant outage produces FTS fallback.
- Full tests, `cargo fmt`, and `cargo clippy -D warnings` pass.
- Golden-question evaluation can compare lexical, vector, and hybrid modes.
