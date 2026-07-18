# queria-backend

> Status: CURRENT - core product live; residual ops acceptance.
> Last verified: 2026-07-18.
> Start with [`docs/HANDOFF.md`](docs/HANDOFF.md). Cuts: [`docs/SIMPLIFICATION.md`](docs/SIMPLIFICATION.md). Backlog: [`docs/IMPROVEMENTS.md`](docs/IMPROVEMENTS.md).

Queria backend workspace for centralized team and agent knowledge.

## Completion Summary

See the full matrix in [`docs/HANDOFF.md`](docs/HANDOFF.md). Short version:

| Area | Status |
|---|---|
| Auth, setup, projects, sources, approvals, tokens, jobs | `COMPLETED` |
| Multi-org isolation MVP | `COMPLETED` (local `main`; prod image may lag until redeploy) |
| Git ingestion and trusted auto-approval | `COMPLETED` |
| Hybrid retrieval (Voyage + Qdrant + FTS/RRF + rerank/compress) | `COMPLETED` (local `main`; prod image may lag until redeploy) |
| MCP agent tools | `COMPLETED` (includes dual-lane `index_memory` + `include_scratch`) |
| Admin API + Astro Admin UI | `COMPLETED` (P0 lean; Admin Playground at `/admin/playground`) |
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

## Multi-org isolation MVP (local main)

Single-stack multi-tenant isolation by `organization_id` (session home, agent token home, Postgres, Qdrant). Dual-lane knowledge stays **inside** each org. Contract: [`docs/PRODUCT.md`](docs/PRODUCT.md) § Multi-organization tenancy. Runtime/ops: [`docs/HANDOFF.md`](docs/HANDOFF.md).

| Piece | v1 behavior |
|---|---|
| Create org | Platform **super-admin** only — `POST/GET /api/v1/orgs`; Admin `/admin/orgs` |
| Join | **Email invite only** — one-time `invite_token` in API response (no SMTP); accept at `/admin/invites/accept` |
| Membership | **One org per user**; session `active_organization_id` from sole membership |
| Isolation | Tenant routes need an active org (**403** without); agent tokens mint in home org only |
| Super-admin without membership | Can manage orgs; **cannot** browse tenant projects/knowledge |

**Bootstrap super-admin** (either path; case-insensitive email):

1. Env: `QUERIA_PLATFORM_SUPER_ADMIN_EMAILS=you@example.com` (comma-separated).
2. SQL: `update user_account set is_platform_super_admin = true where lower(email) = lower('you@example.com');`

**Happy path:** super-admin creates org with `first_admin_email` → capture one-time invite token → invitee accepts → login binds Team B active org → projects/tokens/retrieve scoped to Team B.

**Not in v1:** cross-org share grants, org switcher / multi-membership, SMTP mailer, per-org git allowlist or Voyage keys, super-admin default browse of all tenants’ knowledge.

## Retrieval quality + Admin Playground (local main)

Shared pipeline (MCP, API, CLI, Admin):

```text
hybrid pool → RRF → hydrate → Voyage rerank (fail-open) → near-dup compress (prefer trusted)
```

- **Admin Playground:** session-auth SSR at `/admin/playground` (form + results; not the evaluation product).
- **Env defaults (on):** `QUERIA_RERANK_ENABLED`, `QUERIA_RERANK_MODEL` (`rerank-2.5`), `QUERIA_COMPRESS_ENABLED`. See `.env.example` and [`docs/runbooks/hybrid-retrieval.md`](docs/runbooks/hybrid-retrieval.md).
- **Probe / per-call flags:** optional `rerank` and `compress` on API/MCP retrieve, CLI `retrieval probe --rerank/--compress`, and Admin probe form. Operator probes default `include_scratch=false`.
- Rerank **fails open** (keeps RRF order). Diagnostics include `rerank_applied`, `compress_dropped`, `latency_ms`.
- Shipped on local `main` only in this note — do not assume production host image has been redeployed.

## Docs

| Doc | Role |
|---|---|
| [`docs/HANDOFF.md`](docs/HANDOFF.md) | Canonical current state |
| [`docs/PRODUCT.md`](docs/PRODUCT.md) | Product contract (dual-lane trust model + multi-org isolation) |
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

## Git ingestion

**Admin UI:** `/admin/sources` — **Register Git Source** form and **Trigger Ingest** per source (session cookie). Token mint at `/admin/tokens` requires **name** + **project_slugs**. Ops path: [`docs/runbooks/onboarding.md`](docs/runbooks/onboarding.md) Part A.

**API** (admin session cookie; optional if UI is enough):

```text
POST /api/v1/sources
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
