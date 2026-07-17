# queria-backend

> Status: CURRENT - core product live; residual ops acceptance.
> Last verified: 2026-07-17.
> Start with [`docs/HANDOFF.md`](docs/HANDOFF.md). Cuts: [`docs/SIMPLIFICATION.md`](docs/SIMPLIFICATION.md). Backlog: [`docs/IMPROVEMENTS.md`](docs/IMPROVEMENTS.md).

Queria backend workspace for centralized team and agent knowledge.

## Completion Summary

See the full matrix in [`docs/HANDOFF.md`](docs/HANDOFF.md). Short version:

| Area | Status |
|---|---|
| Auth, setup, projects, sources, approvals, tokens, jobs | `COMPLETED` |
| Git ingestion and trusted auto-approval | `COMPLETED` |
| Hybrid retrieval (Voyage + Qdrant + FTS/RRF) | `COMPLETED` |
| MCP agent tools | `COMPLETED` (includes dual-lane `index_memory` + `include_scratch`) |
| Admin API + Astro Admin UI | `COMPLETED` (P0 lean applied: no React/Three/shadcn) |
| Backup/restore, Caddy edge, production compose | `COMPLETED` (Pingora removed P1; restore-drill still P2 defer) |
| Production acceptance pack | `OPEN` |

## Dual-lane knowledge (CURRENT)

Agents retrieve and write in two lanes. Contract: [`docs/PRODUCT.md`](docs/PRODUCT.md). Backlog/follow-ups: [`docs/IMPROVEMENTS.md`](docs/IMPROVEMENTS.md). Runtime residual: [`docs/HANDOFF.md`](docs/HANDOFF.md).

| Lane | Agent write | Enter trusted |
|---|---|---|
| **scratch** | `index_memory` when the token has **IndexMemory** | Not direct — promote/propose + approval (if enabled) |
| **trusted** | Not direct | `propose_memory` → approve, or trusted Git ingest |

- `retrieve_context`: **`include_scratch` defaults to `true`** (project scratch ∪ trusted). Set `false` for trusted-only probes.
- Without **IndexMemory**, agents stay propose-only (legacy).
- Scratch is project-scoped only (never global). Prefer trusted over scratch when ranking near-duplicates.
- Admin UI does **not** manage scratch yet (operator surfaces stay token/approvals/sources as today).

## Docs

| Doc | Role |
|---|---|
| [`docs/HANDOFF.md`](docs/HANDOFF.md) | Canonical current state |
| [`docs/PRODUCT.md`](docs/PRODUCT.md) | Product contract (dual-lane trust model) |
| [`docs/IMPROVEMENTS.md`](docs/IMPROVEMENTS.md) | Post-MVP backlog (REFERENCE) |
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
