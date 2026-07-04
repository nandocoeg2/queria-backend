# Hybrid Retrieval Implementation Plan

> **For Codex:** Execute this plan task-by-task with test-driven development. Do not proceed past a task until its focused tests, `cargo fmt --check`, and relevant `cargo clippy` checks pass.

**Goal:** Replace PostgreSQL substring retrieval with production-ready Voyage 4 + Qdrant + PostgreSQL FTS hybrid retrieval while preserving project/global authorization, citations, deterministic ingestion, and an operational fallback path.

**Architecture:** PostgreSQL remains the source of truth for chunks, access control, embedding state, jobs, and FTS. Voyage creates 1024-dimensional embeddings; Qdrant stores vectors and returns candidate chunk IDs only. Retrieval obtains semantic and lexical candidates independently, fuses ranks with RRF, then hydrates and authorizes final chunks from PostgreSQL. The worker owns asynchronous embedding backfill, upsert, and stale-vector deletion.

**Tech Stack:** Rust 2024, Axum, Tokio, Tower, SQLx/PostgreSQL, reqwest with rustls, Voyage `voyage-4`, Qdrant REST API, mockall, Docker Compose, Infisical.

**Design Source:** `docs/superpowers/specs/2026-07-04-hybrid-retrieval-design.md`

---

## Task 1: Add Embedding Configuration and Database State

**Files:**
- Modify: `crates/queria-core/src/config.rs`
- Modify: `crates/queria-core/src/model.rs`
- Modify: `crates/queria-db/src/migrate.rs`
- Create: `migrations/20260704000500_hybrid_retrieval.sql`
- Modify: `.env.example`
- Modify: `docker-compose.yml`

### Step 1: Write failing configuration tests

Add tests in `crates/queria-core/src/config.rs` that assert:

```rust
assert_eq!(config.embedding_model, "voyage-4");
assert_eq!(config.embedding_dimension, 1024);
assert_eq!(config.embedding_batch_size, 64);
assert_eq!(config.qdrant_collection, "queria_local_chunks_v1");
assert_eq!(config.qdrant_vector_name, "dense_v1");
assert_eq!(config.retrieval_rrf_k, 60);
assert_eq!(config.retrieval_candidate_multiplier, 4);
assert_eq!(config.retrieval_candidate_cap, 100);
```

Add validation cases for blank API keys in non-local environments, unsupported dimensions, zero batch size, and candidate cap smaller than the requested maximum result count.

Run:

```bash
rtk cargo test -p queria-core config
```

Expected: FAIL because the fields do not exist.

### Step 2: Extend `AppConfig`

Add these fields and environment mappings:

```rust
pub voyage_api_key: String,
pub embedding_model: String,
pub embedding_dimension: u32,
pub embedding_profile_version: String,
pub embedding_batch_size: u32,
pub embedding_timeout_seconds: u64,
pub embedding_max_retries: u32,
pub qdrant_api_key: String,
pub qdrant_collection: String,
pub qdrant_vector_name: String,
pub retrieval_rrf_k: u32,
pub retrieval_candidate_multiplier: u32,
pub retrieval_candidate_cap: u32,
```

Use `voyage-4`, dimension `1024`, input types selected by call site, and `dense_v1`. Permit empty provider API keys only when `QUERIA_ENV=local` so unit tests and PostgreSQL-only development remain possible. Never include keys in `Debug`, errors, or tracing fields; implement a redacted custom `Debug` if configuration is logged.

### Step 3: Write the migration test first

Extend the migration tests in `crates/queria-db/src/migrate.rs` to require:

```rust
for required_sql in [
    "create type embedding_status",
    "search_vector tsvector",
    "embedding_content_hash",
    "embedding_profile_version",
    "idx_chunk_search_vector",
    "idx_chunk_embedding_claim",
] {
    assert!(migration.sql.contains(required_sql));
}
```

Run:

```bash
rtk cargo test -p queria-db migrate
```

Expected: FAIL because migration `20260704000500` is not bundled.

### Step 4: Create and bundle the migration

The migration must:

- Create enum `embedding_status` with `pending`, `processing`, `ready`, `failed`, `stale`.
- Add `search_title text not null default ''`.
- Add generated `search_vector tsvector` using PostgreSQL `simple` configuration over weighted title and content.
- Add provider/model/dimension/profile/content-hash/error/attempt/timestamp fields.
- Add `qdrant_point_id uuid`.
- Backfill existing approved active chunks to `pending` without changing rejected/deprecated knowledge.
- Add a GIN index for FTS and a partial claim index for pending/stale/failed rows.
- Add a uniqueness constraint on non-null `qdrant_point_id`.

Bundle it explicitly in `bundled_migrations()`.

### Step 5: Update environment templates

Document every variable in `.env.example`. Configure the local collection as `queria_local_chunks_v1`; Infisical already uses distinct `dev`, `staging`, and `prod` collection names. Add Qdrant API-key support to Compose without requiring a key for local Qdrant.

### Step 6: Verify Task 1

```bash
rtk cargo test -p queria-core config
rtk cargo test -p queria-db migrate
rtk cargo fmt --all --check
rtk cargo clippy -p queria-core -p queria-db --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/queria-core crates/queria-db migrations .env.example docker-compose.yml
git commit -m "feat: add hybrid retrieval state"
```

---

## Task 2: Build Voyage and Qdrant Gateways

**Files:**
- Modify: `crates/queria-search/Cargo.toml`
- Modify: `crates/queria-search/src/lib.rs`
- Create: `crates/queria-search/src/embedding.rs`
- Create: `crates/queria-search/src/voyage.rs`
- Replace: `crates/queria-search/src/qdrant.rs`

### Step 1: Define mockable provider contracts

Write tests against these interfaces before implementing HTTP:

```rust
#[automock]
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed_documents(&self, inputs: &[EmbeddingDocument])
        -> QueriaResult<Vec<EmbeddingVector>>;
    async fn embed_query(&self, query: &str)
        -> QueriaResult<EmbeddingVector>;
}

#[automock]
#[async_trait]
pub trait VectorIndex: Send + Sync {
    async fn ensure_collection(&self) -> QueriaResult<()>;
    async fn upsert(&self, points: &[VectorPoint]) -> QueriaResult<()>;
    async fn search(&self, request: VectorSearchRequest)
        -> QueriaResult<Vec<VectorCandidate>>;
    async fn delete(&self, point_ids: &[Uuid]) -> QueriaResult<()>;
    async fn health(&self) -> QueriaResult<VectorIndexHealth>;
}
```

Tests must verify document/query modes remain distinct, response count must match request count, and vector dimensions are exactly 1024.

### Step 2: Implement `VoyageClient`

Use reqwest against `POST https://api.voyageai.com/v1/embeddings` with:

- `model: "voyage-4"`
- `input_type: "document"` for chunk batches
- `input_type: "query"` for retrieval
- `output_dimension: 1024`
- configured timeout
- bounded exponential retry for 429 and 5xx only
- no retry for authentication, validation, or malformed responses

Map errors to sanitized `QueriaError::Infrastructure` messages containing status and provider request ID when available, never the API key or input body.

### Step 3: Implement `QdrantClient`

Use typed reqwest requests rather than adding the Qdrant SDK. Implement:

- idempotent collection creation with named vector `dense_v1`, cosine distance, size 1024
- payload indexes for `organization_id`, `project_id`, `scope`, `embedding_profile_version`, and `is_active`
- batch upsert with `wait=true`
- named-vector search returning point UUID and score only
- batch deletion with `wait=true`
- API-key header when configured

Qdrant payload is filtering metadata, not authoritative content.

### Step 4: Test HTTP edge cases

Use a local Axum test server to cover 200, 401, 429-then-success, 500 exhaustion, response length mismatch, invalid dimension, empty Qdrant result, and Qdrant error payload sanitization.

### Step 5: Verify Task 2

```bash
rtk cargo test -p queria-search voyage
rtk cargo test -p queria-search qdrant
rtk cargo fmt --all --check
rtk cargo clippy -p queria-search --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/queria-search
git commit -m "feat: add embedding provider gateways"
```

---

## Task 3: Add Embedding Repository and Durable Jobs

**Files:**
- Modify: `crates/queria-db/src/lib.rs`
- Modify: `crates/queria-db/src/ingestion.rs`
- Create: `crates/queria-db/src/embedding.rs`
- Modify: `crates/queria-db/src/repositories.rs`
- Modify: `crates/queria-ingestion/src/service.rs`

### Step 1: Write repository contract tests

Create tests for:

- enqueueing one `embedding_backfill` job per project/profile
- claiming with `FOR UPDATE SKIP LOCKED`
- claiming only approved, active chunks whose stored hash/profile is stale
- moving claimed chunks to `processing`
- marking a batch `ready` only after Qdrant upsert succeeds
- restoring failed chunks to retryable `failed` with bounded error text
- stale ingestion enqueueing `qdrant_delete` before chunk removal
- unchanged chunk hashes remaining `ready`

The claim result must contain the canonical embedding text, chunk UUID, organization/project/scope metadata, content hash, and target profile.

### Step 2: Implement embedding text construction

Use one deterministic function:

```rust
pub fn canonical_embedding_text(chunk: &EmbeddingChunkRecord) -> String {
    format!(
        "title: {}\nsource: {}\nscope: {}\n\n{}",
        chunk.title, chunk.source_path, chunk.scope, chunk.content
    )
}
```

Hash exactly this string plus provider, model, dimension, and profile version. A profile change must make existing rows claimable without rewriting source content.

### Step 3: Implement repository operations

Add `PgEmbeddingRepository` methods to:

- enqueue/list/get/retry/cancel embedding jobs
- claim a job lease
- claim the next chunk batch
- complete or fail a chunk batch
- complete/fail/recover jobs
- load FTS candidates with rank and scope filtering
- hydrate authorized chunk IDs in caller-provided rank order
- report embedding status counts by project and profile

Keep user and agent authorization in SQL. `include_global=false` must exclude global chunks before ranking.

### Step 4: Integrate stale cleanup

Change Git-manifest application so deleted or changed chunks with a non-null Qdrant point ID produce a durable delete payload before their database rows disappear. The transaction must either persist both source changes and cleanup work or neither.

### Step 5: Verify Task 3

```bash
rtk cargo test -p queria-db embedding
rtk cargo test -p queria-db ingestion
rtk cargo test -p queria-ingestion
rtk cargo fmt --all --check
rtk cargo clippy -p queria-db -p queria-ingestion --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/queria-db crates/queria-ingestion
git commit -m "feat: add durable embedding jobs"
```

---

## Task 4: Extend the Worker for Embedding and Cleanup Jobs

**Files:**
- Modify: `crates/queria-worker/src/jobs.rs`
- Modify: `crates/queria-worker/src/main.rs`
- Modify: `crates/queria-worker/Cargo.toml`
- Create: `crates/queria-worker/src/embedding_jobs.rs`

### Step 1: Write dispatcher tests

Replace the Git-only loop with a typed dispatch test matrix:

| `job_type` | Handler |
|---|---|
| `git_ingestion` | existing manifest pipeline |
| `embedding_backfill` | Voyage batch then Qdrant upsert |
| `qdrant_delete` | Qdrant delete then job completion |
| unknown | fail job without crashing worker |

Use mockall for the store, embedding provider, vector index, and Git preparer.

### Step 2: Implement the embedding batch state machine

For each backfill job:

1. Claim up to configured batch size.
2. Build canonical embedding documents.
3. Call Voyage once for the batch.
4. Upsert all returned vectors to Qdrant.
5. Mark rows ready with profile/hash/point ID.
6. Repeat until no claimable chunks remain.
7. Complete job with counts for processed, skipped, and failed chunks.

If Voyage or Qdrant fails, mark the claimed batch failed and fail the job. A retry must remain idempotent.

### Step 3: Initialize providers once

In `main.rs`, construct one reqwest client, `VoyageClient`, and `QdrantClient`. Call `ensure_collection()` during startup and fail fast on an invalid collection schema. Recover expired leases for all supported job types.

### Step 4: Verify Task 4

```bash
rtk cargo test -p queria-worker
rtk cargo fmt --all --check
rtk cargo clippy -p queria-worker --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/queria-worker
git commit -m "feat: process embedding jobs"
```

---

## Task 5: Implement Authorized Hybrid Retrieval

**Files:**
- Modify: `crates/queria-core/src/contracts.rs`
- Modify: `crates/queria-search/src/retrieval.rs`
- Create: `crates/queria-search/src/hybrid.rs`
- Modify: `crates/queria-db/src/repositories.rs`

### Step 1: Write RRF unit tests

Implement and test pure rank fusion:

```rust
pub fn reciprocal_rank_fusion(
    lexical: &[RankedChunk],
    semantic: &[RankedChunk],
    k: u32,
    limit: usize,
) -> Vec<FusedChunk>
```

Cover overlap, disjoint candidates, deterministic UUID tie-breaking, duplicate IDs, empty sources, and hard result limits. Scores returned to callers must be normalized and documented as fusion scores, not semantic similarity.

### Step 2: Define retrieval dependencies

The async service must depend on mockable lexical search, query embedding, vector search, and authorized hydration interfaces. It must request:

```text
candidate_count = min(limit * candidate_multiplier, candidate_cap)
```

Semantic Qdrant filters must include organization, project-or-global scope, active state, and current embedding profile.

### Step 3: Implement fallback behavior

- Both sources healthy: RRF over both.
- Voyage or Qdrant unavailable: PostgreSQL FTS only and `mode=lexical_fallback`.
- FTS unavailable: fail the request; do not silently trust Qdrant content.
- Stale/missing/unauthorized Qdrant candidates: discard during PostgreSQL hydration.
- No candidates: return an empty successful response.

Extend `RetrieveContextResponse` with non-secret diagnostics:

```rust
pub retrieval: RetrievalDiagnostics {
    pub mode: RetrievalMode,
    pub lexical_candidates: u32,
    pub semantic_candidates: u32,
    pub embedding_profile_version: String,
}
```

### Step 4: Verify Task 5

```bash
rtk cargo test -p queria-search hybrid
rtk cargo test -p queria-core contracts
rtk cargo test -p queria-db repositories
rtk cargo fmt --all --check
rtk cargo clippy -p queria-core -p queria-db -p queria-search --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/queria-core crates/queria-db crates/queria-search
git commit -m "feat: add hybrid context retrieval"
```

---

## Task 6: Wire API, MCP, and CLI Adapters

**Files:**
- Modify: `crates/queria-api/src/app.rs`
- Modify: `crates/queria-api/src/http/mod.rs`
- Modify: `crates/queria-api/src/http/retrieval.rs`
- Create: `crates/queria-api/src/http/embedding_jobs.rs`
- Modify: `crates/queria-mcp/src/http.rs`
- Modify: `crates/queria-mcp/src/server.rs`
- Modify: `crates/queria-cli/src/main.rs`
- Create: `crates/queria-cli/src/database.rs`
- Create: `crates/queria-cli/src/embeddings.rs`
- Create: `crates/queria-cli/src/retrieval.rs`

### Step 1: Add API route tests

Cover:

- session required for embedding administration and retrieval status
- project access enforced before enqueue/list/detail/retry/cancel
- duplicate active backfill returns the existing job
- retrieval returns citations plus diagnostics
- provider outage returns 200 with lexical fallback
- invalid project, limit, and query return stable error codes

Routes:

```text
POST /api/v1/projects/:slug/embedding-jobs/backfill
GET  /api/v1/projects/:slug/embedding-jobs
GET  /api/v1/embedding-jobs/:id
POST /api/v1/embedding-jobs/:id/retry
POST /api/v1/embedding-jobs/:id/cancel
GET  /api/v1/projects/:slug/retrieval/status
POST /api/v1/retrieve-context
```

### Step 2: Share retrieval composition

Move provider/repository construction into application state so API and MCP call the same `RetrievalService`. Remove duplicated direct SQL retrieval from both adapters.

### Step 3: Preserve MCP contract compatibility

Keep the existing `retrieve_context` MCP input. Add diagnostics to output without removing citation fields. Agent-token permission and project/global access must be checked before semantic search and again during hydration.

### Step 4: Add operator CLI commands

Provide:

```text
queria-cli database migrate
queria-cli embeddings backfill --project fjulian-me
queria-cli embeddings status --project fjulian-me
queria-cli retrieval probe --project fjulian-me --query "how is the site deployed?"
```

`database migrate` must use the same bundled migrator as API and worker startup. CLI output must show counts/mode/citations and never reveal provider keys.

### Step 5: Verify Task 6

```bash
rtk cargo test -p queria-api
rtk cargo test -p queria-mcp
rtk cargo test -p queria-cli
rtk cargo fmt --all --check
rtk cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Commit:

```bash
git add crates/queria-api crates/queria-mcp crates/queria-cli
git commit -m "feat: expose hybrid retrieval operations"
```

---

## Task 7: Run the Real Local Backfill and Acceptance Suite

**Files:**
- Modify: `README.md`
- Create: `docs/runbooks/local-development.md`
- Create: `docs/runbooks/hybrid-retrieval.md`
- Create: `tests/golden_questions/fjulian-me.jsonl`
- Create: `scripts/smoke-hybrid-retrieval.sh`

### Step 1: Start dependencies and migrate

Use the linked Infisical `dev` environment:

```bash
rtk docker compose up -d postgres qdrant minio
rtk infisical run --env=dev -- cargo run -p queria-cli -- migrate
```

Confirm migration `20260704000500` and Qdrant collection `queria_dev_chunks_v1`.

### Step 2: Run a bounded provider smoke test

Embed one non-sensitive synthetic document and query through the actual Voyage API. Validate dimension 1024 and Qdrant upsert/search/delete before processing source chunks.

### Step 3: Backfill `fjulian-me`

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- embeddings backfill --project fjulian-me
rtk infisical run --env=dev -- cargo run -p queria-worker
rtk infisical run --env=dev -- cargo run -p queria-cli -- embeddings status --project fjulian-me
```

Pass criteria:

- all approved active chunks are `ready`
- zero chunks remain `processing`
- Qdrant point count matches ready chunks
- rerunning backfill produces zero provider work
- no secret values appear in logs

### Step 4: Add golden retrieval questions

Start with at least ten questions covering Astro structure, TypeScript behavior, Markdown content, configuration, deployment, and project/global scope. Every JSONL row must include query, expected source path(s), expected scope, and minimum citation count.

### Step 5: Test fallback and stale cleanup

1. Stop Qdrant and confirm retrieval uses PostgreSQL FTS with `lexical_fallback`.
2. Restart Qdrant and confirm hybrid mode recovers.
3. Reindex one changed file and confirm only affected embeddings change.
4. Remove a temporary indexed file and confirm the PostgreSQL chunk and Qdrant point are both removed through durable cleanup.

### Step 6: Run the complete quality gate

```bash
rtk cargo fmt --all --check
rtk cargo clippy --workspace --all-targets --all-features -- -D warnings
rtk cargo test --workspace
rtk docker compose ps
rtk infisical run --env=dev -- ./scripts/smoke-hybrid-retrieval.sh
```

Document command output, counts, tested commit SHA, and pass/fail status in the runbook.

Commit:

```bash
git add README.md docs tests scripts
git commit -m "test: validate hybrid retrieval"
```

---

## Final Review and Push

Review the complete diff against the design source. Confirm:

- PostgreSQL remains authoritative.
- Qdrant results are always hydrated and authorized in PostgreSQL.
- global knowledge is included only when requested and permitted.
- provider keys are redacted.
- embedding backfill, retry, cancellation, stale cleanup, and model/profile changes are recoverable.
- API and MCP use one retrieval service.
- fallback behavior is observable.

Run:

```bash
rtk git status --short
rtk git diff --check
rtk cargo fmt --all --check
rtk cargo clippy --workspace --all-targets --all-features -- -D warnings
rtk cargo test --workspace
rtk git log --oneline origin/main..HEAD
rtk git push origin main
```
