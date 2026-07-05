# Queria End-to-End Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** CURRENT

**Goal:** Complete Queria from the current working backend through reliable retrieval, human administration, backup/restore, OCI deployment, and production acceptance.

**Architecture:** Keep the Rust workspace as the backend source of truth. Stabilize retrieval and evaluation before exposing them to the Astro Admin UI, then package API, MCP, worker, proxy, frontend, Postgres, Qdrant, and S3-compatible backup workflows as independently observable services. Each phase has a hard acceptance gate and can be deployed or rolled back without silently changing knowledge visibility.

**Tech Stack:** Rust 2024, Axum, Tower, Tokio, SQLx/Postgres, Voyage-4, Qdrant, TruffleHog, MCP Streamable HTTP, Astro with React islands, MinIO/OCI Object Storage, Cloudflare Pingora, Docker Compose, Infisical, Let's Encrypt.

---

## Status Legend

- `[x]` implemented and verified before this plan was written.
- `[ ]` not completed.
- A phase is complete only when its acceptance gate passes with current evidence.

## Baseline Already Completed

- [x] Rust 2024 workspace with API, MCP, worker, proxy, and CLI binaries.
- [x] Postgres/Qdrant/MinIO local infrastructure.
- [x] First-run setup, password login, cookie sessions, and agent tokens.
- [x] Project and source registry repositories/endpoints.
- [x] Approval repository/endpoints, chunk activation, and audit writes.
- [x] Git ingestion, allowlists, TruffleHog, deterministic parser/chunker, stale cleanup, and trusted Git auto-approval.
- [x] Voyage-4 and Qdrant clients, durable embedding jobs, retry backoff, structured failure logs, and graceful pacing requeue.
- [x] Hybrid retrieval, RRF, API/MCP/CLI adapters, global/project authorization.
- [x] Golden JSONL parser, CLI evaluation runner, and persisted HTTP evaluation endpoints.

## Phase 1: Retrieval and Evaluation Consistency

**Files:**

- Modify: `crates/queria-db/src/hybrid.rs`
- Modify: `crates/queria-search/src/retrieval.rs`
- Create: `crates/queria-search/src/evaluation.rs`
- Modify: `crates/queria-search/src/lib.rs`
- Modify: `crates/queria-cli/src/evaluation.rs`
- Modify: `crates/queria-api/src/http/evaluations.rs`
- Test: `crates/queria-db/src/hybrid.rs`
- Test: `crates/queria-search/src/evaluation.rs`
- Test: `crates/queria-core/tests/evaluation_contract.rs`
- Dataset: `tests/golden_questions/fjulian-me.jsonl`

- [x] **Task 1.1: Add a failing lexical-query regression test**

Add a repository-level test that requires the lexical SQL to contain both a
strict query and a relaxed query while preserving every authorization filter:

```rust
#[test]
fn lexical_search_has_bounded_relaxed_candidates() {
    let sql = LEXICAL_SEARCH_SQL.to_ascii_lowercase();
    assert!(sql.contains("websearch_to_tsquery('simple'"));
    assert!(sql.contains("to_tsvector('simple'"));
    assert!(sql.contains(" | "));
    assert!(sql.contains("k.status = 'approved'"));
    assert!(sql.contains("k.organization_id = access.organization_id"));
    assert!(sql.contains("access.include_global"));
}
```

Run:

```bash
rtk cargo test -p queria-db lexical_search_has_bounded_relaxed_candidates
```

Expected before implementation: FAIL because the SQL has only the strict
`websearch_to_tsquery` path.

- [x] **Task 1.2: Implement strict-weighted relaxed lexical candidates**

Use SQL CTEs to build a strict query and an OR query from normalized lexemes.
Rank strict matches above relaxed matches and retain the configured candidate
limit:

```sql
strict_query as (
  select websearch_to_tsquery('simple', $4) as value
),
relaxed_query as (
  select to_tsquery(
    'simple',
    array_to_string(tsvector_to_array(to_tsvector('simple', $4)), ' | ')
  ) as value
)
```

The candidate predicate must be `search_vector @@ strict_query.value OR
search_vector @@ relaxed_query.value`. Compute score as strict rank multiplied
by `2.0` plus relaxed rank. Do not move tenant/scope/status/source filters out of
SQL.

Run:

```bash
rtk cargo test -p queria-db hybrid
```

Expected: all hybrid repository tests pass.

- [x] **Task 1.3: Verify the real deployment query**

Run the CLI probe:

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- retrieval probe \
  --project fjulian-me \
  --query "deployment and site build notes" \
  --limit 5
```

Expected: at least one project-scoped item with a `README.md` citation. A
semantic provider outage may produce `lexical_fallback`, but it must not produce
zero items.

- [x] **Task 1.4: Extract one shared evaluation executor**

Create a mockable `EvaluationRetriever` boundary and
`queria-search::evaluation::EvaluationExecutor`. The production adapter wraps
`PgRetrievalService`; unit tests use `mockall`. Both API and CLI must call this
executor. The retry predicate remains limited to an empty `lexical_fallback`;
permission and validation errors are not retried.

Required public shape:

```rust
#[mockall::automock]
#[async_trait::async_trait]
pub trait EvaluationRetriever: Send + Sync {
    async fn retrieve(
        &self,
        user_id: Uuid,
        request: RetrieveContextRequest,
    ) -> QueriaResult<RetrieveContextResponse>;
}

pub struct EvaluationExecutor<R> {
    retrieval: R,
    retry_attempts: usize,
    retry_delay: Duration,
}

impl<R: EvaluationRetriever> EvaluationExecutor<R> {
    pub async fn run(
        &self,
        user_id: Uuid,
        project_slug: &str,
        project_id: ProjectId,
        dataset_path: &Path,
    ) -> QueriaResult<EvaluationReport>;
}
```

Write mock-backed tests for immediate success, retry then success, and
non-retryable failure before replacing adapter-local loops.

- [x] **Task 1.5: Persist CLI evaluation reports**

After shared execution, construct `PgEvaluationRepository` from the existing
pool and call `insert_for_project_slug`. Print the persisted record, including
its ID and timestamps, instead of printing an unpersisted report.

The API `POST .../evaluations/run` must use the same executor and repository.
`GET .../evaluations/latest` must return the row created by either adapter.

- [x] **Task 1.6: Run Phase 1 quality gate**

```bash
rtk cargo fmt --all --check
rtk cargo test --workspace
rtk cargo clippy --workspace --all-targets --all-features -- -D warnings
rtk git diff --check
```

Acceptance:

- `queria-cli eval run --project fjulian-me` passes 3/3 questions.
- The CLI-created report is returned by HTTP `evaluations/latest`.
- HTTP retrieval and MCP retrieval return cited project knowledge.
- Project-only tokens cannot retrieve a temporary global sentinel.
- No persistent test session, token, sentinel, or running job remains.

Rollback: revert the Phase 1 commit. No schema migration is required.

## Phase 2: Backfill and Worker Operational Readiness

**Files:**

- Modify: `crates/queria-worker/src/main.rs`
- Modify: `crates/queria-worker/src/embedding_jobs.rs`
- Modify: `crates/queria-db/src/embedding.rs`
- Modify: `docker-compose.yml`
- Create: `Dockerfile`
- Create: `docker/entrypoint.sh`
- Modify: `docs/runbooks/local-development.md`
- Modify: `docs/runbooks/hybrid-retrieval.md`

- [x] **Task 2.1: Add stale-lock recovery tests**

Test that startup recovery requeues only expired `running` jobs, resets chunk
`processing` state to retryable state, clears worker locks, and records a
structured audit/log reason. Active leases must remain untouched.

- [x] **Task 2.2: Add one production-grade multi-stage Dockerfile**

Build all five binaries in a Rust builder stage and copy them into a small
Debian runtime image with CA certificates, Git, and TruffleHog. Run as a
non-root UID. Select the binary through the Compose service `command`; do not
produce five divergent images.

- [x] **Task 2.3: Add application services to Compose**

Add `queria-api`, `queria-mcp`, `queria-worker`, and `queria-proxy`. Use health
dependencies for Postgres and Qdrant, explicit ports `17671`-`17674`, JSON logs,
and restart policy `unless-stopped`. Secrets remain environment references and
must not be hardcoded.

- [x] **Task 2.4: Run supervised backfill without blocking an interactive agent**

Use:

```text
QUERIA_EMBEDDING_BATCH_SIZE=8
QUERIA_EMBEDDING_REQUEST_INTERVAL_MS=30000
```

Observe at least three successful batches. Required trend: ready increases,
pending decreases, failed does not increase, and processing is zero after a
controlled stop. Historical failed chunks may remain until retried.

- [x] **Task 2.5: Classify historical failures**

Group failed chunks by last provider status/error category. Retry only retryable
429/5xx/network failures. Permanent payload/configuration failures must become
an operator-visible terminal category rather than an infinite retry loop.

Acceptance:

- Compose restarts a stopped worker.
- SIGTERM during polling, provider call, and pacing leaves no stale lock.
- Structured failures include `job_id`, `attempts`, `chunk_count`,
  `provider_status`, `retry_after_at`, and sample `source_path`.
- Phase 1 evaluation remains 3/3 during and after backfill.

Rollback: stop application services and use the previous locally built
binaries; infrastructure volumes remain compatible.

## Phase 3: Admin API Completion

**Files:**

- Create: `crates/queria-api/src/http/audit_logs.rs`
- Create: `crates/queria-api/src/http/dashboard.rs`
- Modify: `crates/queria-api/src/http/knowledge_items.rs`
- Modify: `crates/queria-api/src/http/sources.rs`
- Modify: `crates/queria-api/src/http/mod.rs`
- Modify: `crates/queria-db/src/repositories.rs`
- Create: `crates/queria-db/src/admin_queries.rs`
- Test: API router tests in each module

- [x] **Task 3.1: Implement paginated knowledge list**

Add `GET /api/v1/knowledge-items` with cursor pagination and filters for scope,
project, category, status, owner, tag, and validation state. Response fields
must match the UI table and never include deprecated/rejected content unless
explicitly filtered by an authorized human session.

- [x] **Task 3.2: Implement source operational detail**

Extend source detail with latest ingestion, branch/commit, parser/chunker
versions, chunk counts by embedding state, stale cleanup summary, and bounded
content preview. Never return secret-scan context or raw credentials.

- [x] **Task 3.3: Implement audit-log reads**

Add `GET /api/v1/audit-logs` with organization-scoped cursor pagination and
filters for actor, action, resource type, resource ID, project, and date range.
Audit rows remain append-only.

- [x] **Task 3.4: Implement dashboard summaries**

Add project summary counts, latest ingestion/evaluation, source health,
embedding state, pending approvals, and failed jobs in one bounded query surface
for server-rendered UI pages.

- [x] **Task 3.5: Add contract and authorization tests**

Test missing sessions, cross-organization access, invalid cursors, limit caps,
empty states, and stable ordering. Use transaction-isolated Postgres fixtures;
do not mock repository authorization behavior.

Acceptance:

- Every initial Admin UI screen has a stable read/write API.
- All list endpoints are paginated and organization scoped.
- OpenAPI or checked JSON examples document exact response shape.
- Workspace test, clippy, fmt, and diff checks pass.

Rollback: revert Phase 3 routes and queries; no existing contract is removed.

## Phase 4: Astro Admin UI

**Files:**

- Create: `admin/package.json`
- Create: `admin/astro.config.mjs`
- Create: `admin/tsconfig.json`
- Create: `admin/src/layouts/AdminLayout.astro`
- Create: `admin/src/styles/tokens.css`
- Create: `admin/src/lib/api.ts`
- Create: `admin/src/pages/setup.astro`
- Create: `admin/src/pages/projects/index.astro`
- Create: `admin/src/pages/sources/index.astro`
- Create: `admin/src/pages/knowledge/index.astro`
- Create: `admin/src/pages/approvals/index.astro`
- Create: `admin/src/pages/jobs/index.astro`
- Create: `admin/src/pages/tokens/index.astro`
- Create: `admin/src/pages/audit/index.astro`
- Create: `admin/src/pages/evaluation/index.astro`
- Create: focused React islands under `admin/src/components/`

- [ ] **Task 4.1: Scaffold Astro with React islands**

Use SSR pages for navigation and initial data. Hydrate only filters, dialogs,
token copy-once flow, approval action forms, retrieval probe, and evaluation
comparison. Keep session authentication cookie-based through the API.

- [ ] **Task 4.2: Implement Sahara tokens exactly**

Use `#c2652a` primary, `#faf5ee` background, `#8c3c3c` tertiary, EB Garamond
headings, Manrope body, 8px maximum card/button radius, and warm thin borders.
All design reference frames remain 1920x1080, while implementation must also be
usable at mobile and normal desktop widths.

- [ ] **Task 4.3: Implement screens in operational order**

Build Setup, Projects, Sources, Ingestion/Embedding, Retrieval Probe,
Evaluation, Knowledge, Approval, Tokens, and Audit. Every mutation must show
server-confirmed status and preserve filters after navigation.

- [ ] **Task 4.4: Add browser acceptance**

Use Playwright to verify first-run setup, login, project/source navigation,
retrieval probe, approval, token copy-once behavior, and evaluation report. Take
desktop and mobile screenshots and verify there is no overlap or clipped text.

Acceptance:

- `npm run build` succeeds.
- Playwright critical flows pass.
- No page requires client JavaScript for its initial table content.
- No secret/token appears after the copy-once screen is dismissed.
- Page-data p95 target is at most 2 seconds on the local acceptance dataset.

Rollback: deploy the previous admin image; backend API remains compatible.

## Phase 5: Backup, Retention, and Restore Drill

**Files:**

- Create: `crates/queria-backup/` or a focused module under `queria-worker`
- Modify: workspace `Cargo.toml`
- Create: backup job repositories and migrations
- Create: `docs/runbooks/backup-restore.md`
- Modify: `docker-compose.yml`

- [ ] **Task 5.1: Implement S3-compatible object storage abstraction**

Support MinIO locally and OCI Object Storage in production through endpoint,
region, bucket, access key, and secret references. Object keys must partition
organization, project, artifact type, and timestamp.

- [ ] **Task 5.2: Implement backup jobs**

Back up Postgres with a restorable dump and Qdrant with a collection snapshot
or an explicit rebuild manifest. Store checksums, schema version, embedding
profile, source commit, and creation time in a signed manifest.

- [ ] **Task 5.3: Apply retention**

Keep audit logs, rejected proposals, deprecated/superseded knowledge, ingestion
logs, and evaluation reports for 30 days in hot storage. Configure object
lifecycle expiration for logs/reports while preserving current approved
knowledge and the latest accepted evaluation baseline metadata.

- [ ] **Task 5.4: Execute restore drill**

Restore Postgres into an empty instance, restore Qdrant snapshot or rebuild it,
run migrations in verification mode, run MCP doctor, run retrieval probes, and
run golden evaluation. Record pass/fail and duration.

Acceptance:

- Restore succeeds from object storage without using original database volumes.
- Restored evaluation passes the same hard gates as the source environment.
- Missing or mismatched checksums fail closed.
- The runbook contains exact restore and rollback commands.

Rollback: backup jobs are additive; disable scheduling and retain existing
artifacts until their lifecycle expiry.

## Phase 6: Pingora, Packaging, and Production Configuration

**Files:**

- Replace: `crates/queria-proxy/src/main.rs`
- Replace: `crates/queria-proxy/src/routes.rs`
- Modify: `crates/queria-proxy/Cargo.toml`
- Create: `config/production/`
- Create: `docker-compose.production.yml`
- Create: `docs/runbooks/deployment.md`
- Create: `docs/runbooks/rollback.md`

- [ ] **Task 6.1: Replace the Axum proxy skeleton with Pingora**

Route one public domain by path: admin assets/pages, `/api/`, `/mcp`, and
health endpoints. Preserve request IDs, client IP forwarding, timeouts, body
limits, and structured access logs. MCP streaming routes must not be buffered.

- [ ] **Task 6.2: Define production ports**

Use internal service ports in the requested `67671`-`6767x` range. Expose only
443 publicly. Postgres, Qdrant, MinIO/OCI access, worker health, API, and MCP
remain private behind Pingora or the Docker network.

- [ ] **Task 6.3: Wire Infisical self-hosted secret injection**

Use named secret references for Cloudflare, Voyage, Qdrant, database, OCI/S3,
setup token, and per-project SSH keys. `.env` remains a documented emergency
fallback, not the production default. The project SSH reference format is:

```text
infisical://queria/projects/{project_slug}/GIT_SSH_PRIVATE_KEY
```

- [ ] **Task 6.4: Configure TLS and Cloudflare DNS**

Use `fjulian.id`, Cloudflare DNS, origin port 443, and Let's Encrypt on the
server. Do not store certificate private keys in the repository.

- [ ] **Task 6.5: Add production health and rollback controls**

Health must distinguish liveness from readiness. Deployment must fail when
migrations, Postgres, Qdrant, required secrets, or MCP initialization fail.
Rollback must preserve database compatibility and restore the previous image
set without deleting volumes.

Acceptance:

- Only 443 is public.
- API, Admin UI, and MCP are reachable through documented paths.
- Direct internal service ports are unreachable externally.
- Secrets do not appear in Compose render, logs, audit rows, or process output.
- Rollback to the previous image set is rehearsed.

## Phase 7: OCI Deployment and Production Acceptance

**Files:**

- Update: `docs/runbooks/deployment.md`
- Update: `docs/runbooks/rollback.md`
- Update: `docs/runbooks/backup-restore.md`
- Update: `docs/HANDOFF.md`

- [ ] **Task 7.1: Preflight the Oracle Ubuntu host**

Verify 12 GB RAM, 2 CPUs, 190 GB disk, Docker capacity, time synchronization,
firewall 443, persistent volume paths, DNS, and backup-bucket access. Record
free space and memory before deployment.

- [ ] **Task 7.2: Deploy infrastructure and migrate**

Start Postgres and Qdrant, verify health, apply migrations once, then start API,
MCP, worker, Admin UI, and Pingora. Do not run multiple migration writers.

- [ ] **Task 7.3: Run production acceptance pack**

Run health, login/session, project/source reads, retrieval probe, MCP initialize,
MCP `retrieve_context`, project/global scope authorization, evaluation, backup,
and restore-manifest verification.

- [ ] **Task 7.4: Validate SLOs**

Required initial targets:

- API availability: 99.5% monthly.
- retrieval API p95: at most 2 seconds excluding provider rate-limit backoff.
- Admin API-backed page data p95: at most 2 seconds.
- ingestion job start delay p95: at most 60 seconds.
- evaluation runtime: at most 10 minutes for an MVP-size project.
- restore drill: complete and pass within 60 minutes.

- [ ] **Task 7.5: Close the handoff**

Update `docs/HANDOFF.md` with deployed commit/image versions, endpoint paths,
backup location, latest evaluation score, open incidents, and exact rollback
version. Mark this plan complete only when every phase acceptance gate passes.

## Final Definition of Done

Queria is end-to-end complete only when:

- [ ] A human can set up, log in, manage projects/sources, review knowledge,
      approve/reject proposals, inspect jobs, manage tokens, inspect audit logs,
      run retrieval probes, and compare evaluations through the Admin UI.
- [ ] Codex and Claude can initialize MCP and call every authorized tool using
      project-scoped agent tokens.
- [ ] Retrieval respects organization, project, global permission, source
      activity, and approved-status boundaries in lexical and semantic paths.
- [ ] The `fjulian-me` golden dataset passes with persisted, reproducible reports.
- [ ] Git changes ingest deterministically, deleted files deprecate stale
      knowledge, secrets block trusted auto-approval, and jobs are observable.
- [ ] Backups restore into empty infrastructure and pass evaluation.
- [ ] Production traffic uses Pingora, Cloudflare DNS, and Let's Encrypt on 443.
- [ ] Current deployment, rollback, secrets, retention, and incident procedures
      are documented and rehearsed.
