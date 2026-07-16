# queria-backend

> Status: CURRENT - core product live; residual ops acceptance.
> Last verified: 2026-07-16.
> Start with [`docs/HANDOFF.md`](docs/HANDOFF.md). Cuts applied: [`docs/SIMPLIFICATION.md`](docs/SIMPLIFICATION.md).

Queria backend workspace for centralized team and agent knowledge.

## Completion Summary

See the full matrix in [`docs/HANDOFF.md`](docs/HANDOFF.md). Short version:

| Area | Status |
|---|---|
| Auth, setup, projects, sources, approvals, tokens, jobs | `COMPLETED` |
| Git ingestion and trusted auto-approval | `COMPLETED` |
| Hybrid retrieval (Voyage + Qdrant + FTS/RRF) | `COMPLETED` |
| MCP agent tools | `COMPLETED` |
| Admin API + Astro Admin UI | `COMPLETED` (P0 lean applied: no React/Three/shadcn) |
| Backup/restore, Caddy edge, production compose | `COMPLETED` (Pingora removed P1; restore-drill still P2 defer) |
| Production acceptance pack | `OPEN` |

## Docs

| Doc | Role |
|---|---|
| [`docs/HANDOFF.md`](docs/HANDOFF.md) | Canonical current state |
| [`docs/PRODUCT.md`](docs/PRODUCT.md) | Product contract |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | As-is vs post-cut |
| [`docs/SIMPLIFICATION.md`](docs/SIMPLIFICATION.md) | Hard cut plan |
| [`docs/runbooks/`](docs/runbooks/) | Ops |

## Local services

```bash
docker compose up -d postgres qdrant minio
cargo run -p queria-api
cargo run -p queria-worker
```

Copy `.env.example` into the environment used by binaries and replace
`QUERIA_SETUP_TOKEN`. The API applies bundled migrations. The worker requires
`git` and TruffleHog 3.x on `PATH`.

Local endpoints:

| Service | Address |
|---|---|
| API | `http://127.0.0.1:17671` |
| MCP | `http://127.0.0.1:17672` |
| Worker health | `127.0.0.1:17673` |
| Edge (Caddy) | `http://127.0.0.1:17674` |
| Postgres | `127.0.0.1:17675` |
| Qdrant | `127.0.0.1:17676` |
| MinIO | `http://127.0.0.1:17678` |

## Git ingestion API

All ingestion endpoints require the admin session cookie.

```text
POST /api/v1/sources/{source_document_id}/ingest
GET  /api/v1/ingestion-jobs?status=running&limit=50
GET  /api/v1/ingestion-jobs/{job_id}
POST /api/v1/ingestion-jobs/{job_id}/retry
POST /api/v1/ingestion-jobs/{job_id}/cancel
```

## Verification

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
