# Queria Backend Handoff

> Last verified: 2026-07-16
> Branch: `main`
> Verified commit: `4e7cb37` (docs: update handoff with dashboard 3d galaxy graph and project modal details)
> Docs sync: README status matrix and plan Phase 5–7 notes aligned this date.

This is the canonical continuation document for Queria backend work. It
separates implemented behavior from approved target-state design. When other
product docs disagree with this file, prefer this handoff.

## Product Contract

Queria centralizes organization-wide and project-specific knowledge for humans
and AI agents. Every agent should call `retrieve_context(project_id, query)`
before work and may call `propose_memory` after work. Permanent memory enters
normal retrieval only through approval or a trusted Git ingestion pipeline.

Knowledge scopes:

- `global`: reusable coding, security, deployment, SOP, and operational standards.
- `project`: business flow, technical decisions, integrations, incidents, gotchas, and domain notes for one project.
- `include_global=true` still requires token permission; project-only tokens cannot retrieve global knowledge.

## Repository Boundaries

| Path | Git status | Responsibility |
|---|---|---|
| `queria/backend` | Git repository, `main` tracks `origin/main` | Rust backend, migrations, runtime runbooks, active implementation plan. |
| `queria` | Not a Git repository | Product overview and local workspace grouping. |
| workspace `docs/` | Not a Git repository | Product target-state spec, research, UI flow, MCP client references. |

Do not assume parent-workspace documents are present in a standalone backend
clone. This handoff and the active plan therefore contain all required next-step
context.

## Implemented Architecture

```mermaid
flowchart LR
    Human["Admin or operator"] --> API["queria-api :17671"]
    Agent["Codex or Claude"] --> MCP["queria-mcp :17672"]
    API --> PG[(Postgres :17675)]
    MCP --> PG
    API --> Search["queria-search"]
    MCP --> Search
    Search --> PG
    Search --> Voyage["Voyage voyage-4"]
    Search --> Qdrant[(Qdrant :17676)]
    Worker["queria-worker :17673 health"] --> PG
    Worker --> Voyage
    Worker --> Qdrant
    Worker --> Repo["Allowlisted Git repository"]
    Worker --> TruffleHog["TruffleHog"]
    Proxy["queria-proxy :17674 (Pingora)"] --> API
    Proxy --> MCP
    Proxy --> Admin["queria-admin :4321"]
    Worker --> MinIO[(MinIO / S3-compatible)]
    CLI["queria-cli / queria-backup"] --> MinIO
    CLI --> PG
```

The Rust workspace uses edition 2024 and contains eleven crates:
`queria-core`, `queria-auth`, `queria-db`, `queria-search`,
`queria-observability`, `queria-api`, `queria-mcp`, `queria-worker`,
`queria-ingestion`, `queria-proxy`, and `queria-cli`.

## Completion Matrix

### Backend Capability

| Capability | Status | Evidence or gap |
|---|---|---|
| Rust workspace and binaries | `COMPLETED` | API, MCP, worker, proxy, and CLI binaries compile in one workspace. |
| Runtime config and JSON logging | `COMPLETED` | Environment-driven config and tracing JSON are implemented. |
| Postgres, Qdrant, MinIO local infrastructure | `COMPLETED` | `docker-compose.yml` exposes ports `17675`-`17679`. |
| Baseline schema and migrations | `COMPLETED` | Eight bundled migrations cover baseline, sessions, source indexes, ingestion, hybrid retrieval, retry backoff, evaluation reports, and backup records. |
| First-run setup and local login/session | `COMPLETED` | Setup token, first admin, password hashing, login, cookie session, and `/me` exist. |
| Projects and source registry API | `COMPLETED` | List/create/get project and register/list/get source are DB-backed. |
| Approval flow | `COMPLETED` | List/detail/approve/reject, initial chunk creation, and audit events exist. |
| Git ingestion MVP | `COMPLETED` | Allowlist validation, TruffleHog gate, parser/chunker, stale cleanup, trusted auto-approval, and job lifecycle exist. |
| Voyage-4 and Qdrant clients | `COMPLETED` | Provider clients, collection setup, durable jobs, and backfill are implemented. |
| Hybrid retrieval and RRF | `COMPLETED` | Semantic plus Postgres FTS works with strict-weighted relaxed OR query fallback. |
| Embedding pacing and graceful stop | `COMPLETED` | Paced batches requeue and unlock jobs instead of sleeping while holding a running job. |
| Evaluation baseline | `COMPLETED` | Shared evaluation executor handles runs from both API and CLI and persists reports. |
| MCP HTTP transport | `COMPLETED` | `initialize`, `tools/list`, and `tools/call` work with agent-token authorization. |
| MCP agent tools | `COMPLETED` | Agent surface: `retrieve_context`, `search_knowledge`, `propose_memory`, `list_projects`, `get_source`. Maintainer actions (approve/reject, reindex, token admin) stay on session Admin HTTP API by design, not MCP. |
| Admin-oriented API | `COMPLETED` | Complete set of admin APIs for dashboard, audit logs, evaluations, approvals, and jobs. |
| Pingora reverse proxy | `COMPLETED` | Path router for `/api/`, `/mcp`, admin, and health; live on host port `17674`. |
| Astro Admin UI | `COMPLETED` | Sahara Design System, SSR pages, React islands, Playwright smoke coverage. |
| S3 backup and restore drill | `COMPLETED` | `queria-backup` crate + CLI/runbook; live empty-volume restore remains an acceptance item. |
| Production OCI packaging | `COMPLETED` | Dockerfiles, production Compose, deployment/rollback runbooks. Stack is deployed; Phase 7 acceptance pack still open. |

### Human UI Screens

| Screen / surface | Status | Entry point / honesty note |
|---|---|---|
| Setup Wizard | `COMPLETED` | `/admin/setup` |
| Login / Logout | `COMPLETED` | `/admin/login`, `/admin/logout` |
| Dashboard | `COMPLETED` | `/admin/dashboard` including 3D galaxy knowledge graph |
| Projects | `COMPLETED` | `/admin/projects` with create-project dialog |
| Sources | `COMPLETED` | `/admin/sources`, `/admin/sources/detail` (embedding counts on source detail) |
| Knowledge Items | `COMPLETED` | `/admin/knowledge` |
| Approval Queue | `COMPLETED` | `/admin/approvals` |
| Ingestion Jobs | `COMPLETED` | `/admin/jobs` (primary place for job lifecycle; embedding work shows up as jobs) |
| Embedding Status | `EMBEDDED` | No dedicated `/admin/embedding` route. Visible via dashboard summary, source detail chunk-state counts, jobs list, and CLI `embeddings status`. |
| Retrieval Probe | `EMBEDDED` | No dedicated `/admin/retrieval-probe` route. Operator probe/eval path is Evaluation + CLI `retrieval probe`. |
| Agent Tokens | `COMPLETED` | `/admin/tokens` |
| Audit Logs | `COMPLETED` | `/admin/audit` |
| Evaluation | `COMPLETED` | `/admin/evaluation` |
| Backup/Restore | `API/CLI` | No dedicated Admin UI page. Backup/restore is CLI + `queria-backup` + runbook. |

## Production Host

| Field | Value |
|---|---|
| Public IP | `168.110.214.130` |
| SSH user | `ubuntu` |
| Hostname | `instance-20260518-2039` (Oracle Cloud aarch64) |
| OS | Ubuntu 24.04 (kernel `6.17.0-1016-oracle`) |
| Deploy path | `/home/ubuntu/queria-backend` |
| Compose file | `docker-compose.production.yml` (also legacy copy under `/home/ubuntu/queria`) |
| Local SSH private key | workspace root `ssh-key-2026-04-16.key` (mode `600`; never commit) |
| Local SSH public key | workspace root `ssh-key-2026-04-16.key.pub` |

Connect:

```bash
ssh -i /Users/fernandojulian/project/knowledge-based-rag/ssh-key-2026-04-16.key ubuntu@168.110.214.130
```

Verified live stack on 2026-07-16 (containers up ~7 days):

| Service | Notes |
|---|---|
| `queria-backend-queria-proxy-1` | Public host port `17674` (Pingora path router) |
| `queria-backend-queria-api-1` | Internal only |
| `queria-backend-queria-mcp-1` | Internal only |
| `queria-backend-queria-worker-1` | Internal only |
| `queria-backend-queria-admin-1` | Internal (`4321` in container) |
| `queria-backend-postgres-1` | Healthy |
| `queria-backend-qdrant-1` | Healthy |
| `queria-backend-minio-1` | Running |

Proxy health on the host:

```bash
curl -sS http://127.0.0.1:17674/healthz   # OK / HTTP 200
```

Host resource snapshot (2026-07-16): ~11 GiB RAM, ~188G disk with ~145G free, Docker 29.5.0.

Same host also runs unrelated shared workloads (monitoring, other app DBs, `grok2api`, etc.). Do not treat the box as Queria-only when planning ports, disk, or restarts.

Security:

- Never paste the RSA private key into git, chat history, or docs beyond the local path above.
- Workspace `.gitignore` already ignores `*.key`.
- Prefer Infisical for app secrets; host `.env` files are emergency/runtime only.

## Current Local State

The first project is `fjulian-me`, sourced from:

```text
/Users/fernandojulian/project/fjulian/fjulian.me
```

Embedding snapshot observed on 2026-07-05:

| State | Count |
|---|---:|
| `ready` | 344 |
| `pending` | 717 |
| `failed` | 168 |
| `processing` | 0 |
| `stale` | 0 |

The latest `embedding_backfill` job is `queued`, attempt `12`, with no worker
lock. Historical failed chunks remain retryable.

`README.md` specifically has 10 ready, 12 pending, and 2 failed chunks. The
`README.md: Deployment` chunk is pending, while other build/deployment chunks
are already ready.

## Latest Verified Retrieval Finding

Historical gap (pre-Phase-1): the golden query `deployment and site build notes`
failed under strict-only `websearch_to_tsquery('simple', $query)` because
`simple` kept `and` and AND-combined every term.

**Resolved in code:** hybrid lexical SQL now uses strict-weighted matches plus a
bounded relaxed OR path; RRF still combines lexical and semantic rankings.
Auth, approved status, active source, organization, project, and global-scope
filters remain inside both SQL paths.

Re-verify on current production data after embedding backfill; do not treat the
old 2/3 failure as the live default without a fresh probe.

## Latest Evaluation Result

Historical local observation (2026-07-05, pre-shared executor and pre-relaxed
lexical path):

Command:

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- eval run --project fjulian-me
```

Observed then:

- total: 3
- passed: 2
- failed: 1
- regression score: `0.77777773`
- failed question: `deployment and site build notes`

**Code status since then:** CLI and HTTP share `EvaluationExecutor` and both
persist reports. Fresh production acceptance must re-run eval and record the
new score here; do not close Phase 7 on this historical 2/3 result alone.

## Operational Commands

Start infrastructure:

```bash
rtk docker compose up -d postgres qdrant minio
```

Run migrations:

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- database migrate
```

Run a bounded worker pass:

```bash
rtk infisical run --env=dev -- /usr/bin/env \
  QUERIA_EMBEDDING_BATCH_SIZE=8 \
  QUERIA_EMBEDDING_REQUEST_INTERVAL_MS=30000 \
  cargo run -p queria-worker
```

Check embedding state:

```bash
rtk infisical run --env=dev -- cargo run -p queria-cli -- embeddings status --project fjulian-me
```

Run quality gates:

```bash
rtk cargo fmt --all --check
rtk cargo test --workspace
rtk cargo clippy --workspace --all-targets --all-features -- -D warnings
rtk git diff --check
```

## Security Boundaries

- Never commit provider keys, Cloudflare credentials, setup tokens, sessions, or agent tokens.
- Infisical is the primary runtime secret source; `.env` remains local fallback only.
- Raw agent tokens are shown once; Postgres stores token prefix and hash.
- Project Git paths and SSH repositories must pass explicit allowlists.
- TruffleHog must pass before trusted Git auto-approval.
- Agent proposals never receive trusted Git auto-approval.
- Global retrieval requires both `include_global=true` and token permission.
- Database writes, migrations, dependency additions, pushes, and deployments require explicit approval.

## Residual Gaps (current)

| Gap | Priority | Notes |
|---|---|---|
| Embedding backfill residual | High | Last local snapshot (2026-07-05) had many pending/failed chunks. Re-measure on production and finish bounded backfill. |
| Production acceptance pack | High | Stack is live; Phase 7 DoD (eval 3/3, MCP client accept, backup restore drill, SLO spot-check, handoff close) still open. |
| Admin UI dedicated routes | Low | Embedding / retrieval probe / backup are embedded or CLI-only (see screen matrix). Optional polish only. |
| Maintainer MCP tools | Deferred by design | Approve/reject/reindex/token admin remain Admin HTTP; agent MCP stays five tools. |

## Continue From Here

Use [`superpowers/plans/2026-07-05-queria-end-to-end.md`](./superpowers/plans/2026-07-05-queria-end-to-end.md).
Phases 1–6 implementation work is done. Immediate work is residual ops and
Phase 7 acceptance, not feature scaffolding.

Execution order:

1. Measure embedding status on production; classify and retry failed chunks.
2. Re-run golden evaluation; target 3/3 with a persisted report.
3. Run the production acceptance pack (health, login, probe, MCP, scopes, backup/restore).
4. Record deploy commit/image, endpoints, eval score, and open issues in this handoff.
5. Optionally add dedicated Admin routes only if operators need them daily.
6. Keep maintainer tools off the agent MCP surface unless product requires otherwise.

