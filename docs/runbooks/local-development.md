# Local Development Runbook

> Status: CURRENT for implemented local backend workflows.
> Last verified: 2026-07-18.
> Known gaps and current counts: [`../HANDOFF.md`](../HANDOFF.md).
> Operator UI path (project / Git source / token): [`onboarding.md`](./onboarding.md) **Part A**.

## Services

Queria backend runs these local services:

- Postgres on `127.0.0.1:17675`
- Qdrant local on `127.0.0.1:17676`
- MinIO on `127.0.0.1:17678`

Start infrastructure:

```bash
rtk docker compose up -d postgres qdrant minio
```

Apply migrations:

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- database migrate
```

For local-only development without Infisical, copy `.env.example` to `.env` and set provider keys manually.

After the stack is up (API/worker/Admin as needed), use **[onboarding Part A](./onboarding.md)** for the Admin UI path: create project → Register Git Source → Trigger Ingest → mint agent token (name + project_slugs). CLI steps below remain valid for digs and eval.

## First Project

The seeded first project is:

- project slug: `fjulian-me`
- source path: `/Users/fernandojulian/project/fjulian/fjulian.me`

Prefer Admin `/admin/sources` (Register Git Source + Trigger Ingest) after local login. Or run Git ingestion via worker if the source registry/chunks need a CLI refresh:

```bash
rtk infisical run --env=dev -- cargo run -p queria-worker
```

Stop the worker after the ingestion job succeeds if you only want a one-off run.

## Embedding Backfill

Queue backfill:

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- embeddings backfill --project fjulian-me
```

Check status:

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- embeddings status --project fjulian-me
```

Run worker with a conservative Voyage batch size:

```bash
rtk infisical run --env=dev -- /usr/bin/env QUERIA_EMBEDDING_BATCH_SIZE=8 QUERIA_EMBEDDING_REQUEST_INTERVAL_MS=30000 cargo run -p queria-worker
```

Pacing is durable: after each successful batch the worker requeues and unlocks
the job with `retry_after_at`. It must not sleep while holding a `running` job.
After stopping the worker, verify `processing=0` and no job lock remains.

`failed` chunks are retryable for embedding backfill. A `429 Too Many Requests` response should requeue the job with `retry_after_at`, not leave the newest job terminal failed.

When the provider keeps returning `429`, reduce the batch and cap the retry window while developing locally:

```bash
rtk infisical run --env=dev -- /usr/bin/env QUERIA_EMBEDDING_BATCH_SIZE=4 QUERIA_EMBEDDING_RETRY_BACKOFF_BASE_SECONDS=15 QUERIA_EMBEDDING_RETRY_BACKOFF_MAX_SECONDS=60 cargo run -p queria-worker
```

For the `fjulian-me` backfill readiness pass, use the approved MVP batch size:

```bash
rtk infisical run --env=dev -- /usr/bin/env QUERIA_EMBEDDING_BATCH_SIZE=8 cargo run -p queria-worker
```

## Retrieval Probe

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- retrieval probe --project fjulian-me --query "Astro markdown content flow" --limit 5
```

Expected result:

- `items` contains cited chunks with source path and chunk id.
- `retrieval.mode` is `hybrid` when Voyage and Qdrant are available.
- `retrieval.mode` is `lexical_fallback` only when semantic retrieval is temporarily unavailable.

## Evaluation

Run the retrieval baseline:

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- eval run --project fjulian-me
```

The baseline reads `tests/golden_questions/fjulian-me.jsonl` and reports pass/fail, expected scope hits, expected citation hits, and a regression score.

CLI is the only evaluation operator path (Admin evaluation UI/API removed).
`queria-cli eval run` persists reports when the DB is available; dashboard may
show the latest report afterward.
