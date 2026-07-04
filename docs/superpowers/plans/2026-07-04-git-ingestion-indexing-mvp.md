# Git Ingestion and Indexing MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a secure, idempotent Git ingestion pipeline that indexes allowlisted project repositories into approved, cited Postgres knowledge and exposes job control through authenticated HTTP endpoints.

**Architecture:** A registered `git_repo` source is the ingestion root. The API creates Postgres jobs; the worker claims one job atomically with `FOR UPDATE SKIP LOCKED`, validates the local path and SSH URI, runs TruffleHog, reads tracked files, parses deterministic sections, and applies the resulting manifest in one database transaction. Child `source_document` rows represent files, generated knowledge is auto-approved only for trusted roots that passed all checks, and stale files are deprecated and removed from retrieval.

**Tech Stack:** Rust 2024, Axum, Tokio, SQLx/Postgres, mockall, SHA-256, Git CLI, TruffleHog CLI, serde_json, serde_yaml, toml.

---

### Task 1: Schema and ingestion-job repository

**Files:**
- Create: `migrations/20260704000400_git_ingestion.sql`
- Modify: `crates/queria-db/src/migrate.rs`
- Modify: `crates/queria-db/src/repositories.rs`

- [ ] Add a migration test that expects job lifecycle fields, child source identity, stable knowledge keys, and active-job uniqueness.
- [ ] Run `rtk cargo test -p queria-db migrate` and confirm the test fails because migration `20260704000400` is absent.
- [ ] Add `started_at`, `finished_at`, `cancel_requested_at`, `result`, and `retry_of_id` to `ingestion_job`; add `source_root_id`, `is_active`, and `indexed_at` to `source_document`; add `stable_key` and `generated_by` to `knowledge_item`.
- [ ] Add partial indexes for one active job per source and one active child document per root/path.
- [ ] Add typed records and repository methods for trigger, list, detail, retry, cancel, claim, succeed, and fail. Claim must use a CTE selecting `FOR UPDATE SKIP LOCKED` and update the selected row in the same statement.
- [ ] Run `rtk cargo test -p queria-db` and confirm all repository/migration unit tests pass.

### Task 2: Authenticated ingestion HTTP API

**Files:**
- Create: `crates/queria-api/src/http/ingestion_jobs.rs`
- Modify: `crates/queria-api/src/http/mod.rs`
- Modify: `crates/queria-api/src/app.rs`

- [ ] Add router tests proving trigger, list, detail, retry, and cancel reject unauthenticated requests.
- [ ] Run `rtk cargo test -p queria-api` and confirm the new route tests fail because the routes are missing.
- [ ] Implement `POST /api/v1/sources/{id}/ingest`, `GET /api/v1/ingestion-jobs`, `GET /api/v1/ingestion-jobs/{id}`, `POST /api/v1/ingestion-jobs/{id}/retry`, and `POST /api/v1/ingestion-jobs/{id}/cancel`.
- [ ] Validate UUIDs, bounded pagination, and state transitions at the HTTP boundary; map missing resources to 404 and invalid transitions to 409.
- [ ] Run `rtk cargo test -p queria-api` and confirm all API tests pass.

### Task 3: Deterministic parsers and chunking

**Files:**
- Create: `crates/queria-ingestion/Cargo.toml`
- Create: `crates/queria-ingestion/src/lib.rs`
- Create: `crates/queria-ingestion/src/parser.rs`
- Create: `crates/queria-ingestion/src/model.rs`
- Modify: `Cargo.toml`

- [ ] Add failing parser tests for Markdown/MDX headings, Astro frontmatter and markup headings, TypeScript exported symbols, and JSON/YAML/TOML/config top-level sections.
- [ ] Add failing chunk tests proving stable order, bounded line windows, overlap, line ranges, citation paths, and repeatable content hashes.
- [ ] Run `rtk cargo test -p queria-ingestion` and confirm failures are caused by missing parser behavior.
- [ ] Implement extension-based parser dispatch, syntax validation for structured config, and deterministic line-aware section extraction.
- [ ] Implement bounded deterministic chunks whose stable key derives from source path, section identity, and chunk index.
- [ ] Run `rtk cargo test -p queria-ingestion` and confirm parser/chunk tests pass.

### Task 4: Git and secret-scan gateway

**Files:**
- Create: `crates/queria-ingestion/src/git.rs`
- Create: `crates/queria-ingestion/src/scanner.rs`
- Modify: `crates/queria-ingestion/src/lib.rs`
- Modify: `crates/queria-core/src/config.rs`
- Modify: `.env.example`

- [ ] Define mockall-backed `GitRepositoryGateway` and `SecretScanner` traits and add failing tests for canonical-path allowlisting, SSH host/repository allowlisting, excluded directories, supported extensions, and maximum file size.
- [ ] Run `rtk cargo test -p queria-ingestion` and confirm the security tests fail before implementation.
- [ ] Add parameterized config defaults for allowed roots, allowed SSH hosts, allowed SSH repositories, excluded directories, maximum file bytes, chunk lines, overlap lines, worker poll interval, worker identity, and TruffleHog executable.
- [ ] Implement Git CLI access without a shell: validate first, then call `git -C <path> rev-parse`, `symbolic-ref`, and `ls-files -z` using argument arrays.
- [ ] Implement TruffleHog as `trufflehog filesystem --json --no-update --fail <path>`; a missing binary, command failure, or finding must fail the job closed.
- [ ] Run `rtk cargo test -p queria-core -p queria-ingestion` and confirm all tests pass.

### Task 5: Transactional manifest application

**Files:**
- Create: `crates/queria-ingestion/src/service.rs`
- Modify: `crates/queria-db/src/repositories.rs`
- Modify: `crates/queria-ingestion/src/lib.rs`

- [ ] Add failing service tests with mockall gateways proving unchanged files are skipped, changed files are parsed, deleted files are stale, and failed scans never reach indexing.
- [ ] Add repository SQL tests proving generated knowledge is approved only under `generated_by = 'trusted_git_pipeline'`, stale knowledge becomes `deprecated`, stale chunks are deleted, and audit actions are inserted.
- [ ] Run targeted tests and confirm expected failures.
- [ ] Implement the service sequence: load root, validate, scan, collect tracked manifest, parse supported files, and call one transactional manifest-application repository method.
- [ ] In that transaction, update root commit/content hash, upsert child documents, supersede changed generated knowledge, insert approved items/chunks, deprecate removed-file knowledge, delete stale chunks, and write per-file plus job audit events with the configured pipeline identity.
- [ ] Store line ranges, citation path, parser, source hash, commit SHA, and pipeline identity in structured metadata.
- [ ] Run `rtk cargo test -p queria-db -p queria-ingestion` and confirm all tests pass.

### Task 6: Worker claim loop

**Files:**
- Modify: `crates/queria-worker/Cargo.toml`
- Modify: `crates/queria-worker/src/jobs.rs`
- Modify: `crates/queria-worker/src/main.rs`

- [ ] Add failing worker tests for one-job claim, success completion, failure recording, and cancellation before indexing.
- [ ] Run `rtk cargo test -p queria-worker` and confirm failures are due to the idle scaffold.
- [ ] Wire pool creation and migrations, create concrete Git/TruffleHog/indexing dependencies, and poll for jobs until Ctrl-C.
- [ ] Record typed failure messages without secrets, use bounded polling, and recover abandoned running jobs after the configured lease duration.
- [ ] Run `rtk cargo test -p queria-worker` and confirm all worker tests pass.

### Task 7: End-to-end verification with fjulian-me

**Files:**
- Modify: `README.md`
- Modify: `.env.example`

- [ ] Run `rtk cargo fmt --all -- --check`.
- [ ] Run `rtk cargo test --workspace`.
- [ ] Run `rtk cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- [ ] Start Postgres with `rtk docker compose up -d postgres`, run migrations, and verify migration `20260704000400` is recorded.
- [ ] Verify TruffleHog is installed and run one real scan against `/Users/fernandojulian/project/fjulian/fjulian.me`.
- [ ] Start API and worker, authenticate, trigger ingestion for the seeded source, and poll the job to a terminal state.
- [ ] Query Postgres/API to prove commit SHA and root hash were updated, supported files became child sources, and chunks have line citations. Use a disposable Git fixture under `/tmp` to prove unchanged reruns skip files and deleted files are deprecated without entering retrieval.
- [ ] Document exact environment variables, API examples, job states, failure semantics, and local run commands.
- [ ] Re-run format, full tests, and clippy after documentation and cleanup.
