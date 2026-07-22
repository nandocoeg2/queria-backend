# Queria Backend Handoff

> Last verified: 2026-07-23 (CLI release automation chain on branch; host stack unchanged)
> Branch: `feat/cli-release-automation` (merge target `main`)
> **Deploy path (intended):** GitHub Actions → GHCR (`backend` + `admin`, `linux/arm64`) → SSH compose pull/up. Runbook: [`runbooks/deployment.md`](./runbooks/deployment.md).
> **Verified this session:** rsync + host `compose build` tagged as `ghcr.io/nandocoeg2/queria-backend/{backend,admin}:latest`; stack recreated; migrate `{"status":"migrated"}`.
> **Public access live:** `http://168.110.214.130:17674/healthz` **200**; **`https://queria.fjulian.id/healthz` 200** (Nginx + Certbot LE); `/admin/login` **200**.
> **Residual:** GHCR registry pull denied until packages exist + host `GHCR_TOKEN` / green Actions deploy; set repo secrets so next push uses Path A.

### CLI release automation (2026-07-23)

- Chain: **detect-and-tag** (Cargo.toml version) → **Release queria-cli** (unchanged matrix) → **Homebrew formula** direct-push.
- Design: `docs/superpowers/specs/2026-07-23-cli-release-automation-design.md`
- Residual: per-laptop `brew reinstall` + `HOMEBREW_GITHUB_API_TOKEN` while backend private; accidental version bumps still release — review Cargo.toml carefully.


> Docs pack: post–ponytail-audit living docs (PRODUCT, ARCHITECTURE, SIMPLIFICATION, DOCS_POLICY); historical plans archived.
> SIMPLIFICATION P0 applied: Admin dashboard is stat cards only (Three.js + unused shadcn/React islands removed).
> SIMPLIFICATION P1 applied: Caddy edge (no Pingora/`queria-proxy`); observability folded into core; dead db traits removed.
> SIMPLIFICATION P2–P3 applied: Admin eval UI deferred (CLI kept); `proxy_addr` removed; enowx-rag Qdrant-only.
> **Production now runs Caddy `queria-edge` + dual-lane image** (redeploy 2026-07-17; see stack identity below).
> **Production project `fjulian-me` seeded**; embedding backfill **substantially complete** (ready 1226 / failed 3 Voyage 429 residual).
> **Local main multi-org isolation MVP** (schema → session → orgs/invites → Admin orgs/accept → tenant enforce). Production not redeployed for this yet.

This is the canonical continuation document for Queria backend work. It
separates implemented behavior from approved target-state design. When other
product docs disagree with this file, prefer this handoff.

Living companion docs: [`PRODUCT.md`](./PRODUCT.md), [`ARCHITECTURE.md`](./ARCHITECTURE.md),
[`SIMPLIFICATION.md`](./SIMPLIFICATION.md), [`IMPROVEMENTS.md`](./IMPROVEMENTS.md),
[`DOCS_POLICY.md`](./DOCS_POLICY.md).

## Product Contract

Queria centralizes organization-wide and project-specific knowledge for humans
and AI agents. Full contract: [`PRODUCT.md`](./PRODUCT.md).

**Implemented today:** agents call `retrieve_context` before work and may call
`propose_memory` after work. Permanent **trusted** memory enters normal retrieval
only through approval or a trusted Git ingestion pipeline.

**Dual-lane Slice A (code + prod image 2026-07-17):** project-scoped **scratch**
via MCP `index_memory` (permission `IndexMemory`), sync Voyage+Qdrant embed,
content_hash idempotency, shared max body with `propose_memory`, and
`include_scratch` default true on agent retrieve. Promote / Admin scratch UI
still deferred (`IMP-15`/`IMP-16`). See PRODUCT lanes and
[`IMPROVEMENTS.md`](./IMPROVEMENTS.md).

**Local multi-git index-here (on `main` 2026-07-19/20):** CLI `queria-cli index-here`
discovers nested git roots (parent `ls-files` **skips paths under nested roots**
in the same run), gates files, uploads via `POST /api/v1/agent/index-local`
(permission `IndexLocal`) → status **`needs_review`** (“Needs review”) + async
embed jobs; default retrieve excludes unless `include_needs_review=true`; Admin
`/admin/needs-review` promote/reject; privileged MCP `list_needs_review` /
`promote_knowledge` / `reject_needs_review` (`ManageNeedsReview`, not default mint).
Auto-create project from origin last path segment. **Does not** auto-promote to
trusted (`IMP-L6` deferred). Smoke script: `scripts/e2e_index_here_edge.py`
(needs token with `index_local`; optional `QUERIA_PROMOTE_TOKEN`). **Prod host may
lag** until redeploy + `queria-cli database migrate` (enum `needs_review`).
Backlog: IMP-L1…L5 `done`; IMP-L6 stays proposed.

**Retrieval quality + Playground (local main 2026-07-18):** shared pipeline
pool → RRF → hydrate → Voyage rerank (`rerank-2.5`, **fail-open**) → near-dup
compress (prefer trusted); optional `rerank`/`compress` on API/MCP/CLI; state-held
`PgRetrievalService` on API/MCP; Admin SSR `/admin/playground`. Env knobs
`QUERIA_RERANK_*`, `QUERIA_COMPRESS_ENABLED` (defaults on). Diagnostics:
`rerank_applied`, `compress_dropped`, `latency_ms`. Backlog IDs IMP-01/02/03
(and pool sizing IMP-17 folded in). **Not** production-redeployed with this handoff.

Knowledge scopes:

- `global`: reusable coding, security, deployment, SOP standards (**trusted only**; no scratch global).
- `project`: project trusted knowledge plus that project’s scratch when `include_scratch` is true.
- `include_global=true` still requires token permission; project-only tokens cannot retrieve global knowledge.
- `include_scratch`: default true for agent retrieve; false/trusted-only for golden eval path.

## Repository Boundaries

| Path | Git status | Responsibility |
|---|---|---|
| `queria/backend` | Git repository, `main` tracks `origin/main` | Rust backend, migrations, runtime runbooks, HANDOFF + SIMPLIFICATION. |
| `queria` | Not a Git repository | Product overview and local workspace grouping. |
| workspace `docs/` | Not a Git repository | Product REFERENCE research, UI flow, MCP client notes, thin mirrors. |

Do not assume parent-workspace documents are present in a standalone backend
clone. This handoff and [`SIMPLIFICATION.md`](./SIMPLIFICATION.md) contain the
required next-step context for ops acceptance and complexity cuts.

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
    Edge["queria-edge :17674 (Caddy)"] --> API
    Edge --> MCP
    Edge --> Admin["queria-admin :4321"]
    Worker --> MinIO[(MinIO / S3-compatible)]
    CLI["queria-cli / queria-backup"] --> MinIO
    CLI --> PG
```

The Rust workspace uses edition 2024 and contains nine crates:
`queria-core` (auth + observability), `queria-db`, `queria-search`,
`queria-api`, `queria-mcp`, `queria-worker`, `queria-ingestion`,
`queria-cli`, and `queria-backup`. Public edge is Caddy (`docker/Caddyfile`),
not a Rust proxy crate.

## Completion Matrix

### Backend Capability

| Capability | Status | Evidence or gap |
|---|---|---|
| Rust workspace and binaries | `COMPLETED` | API, MCP, worker, and CLI binaries compile in one workspace (edge is Caddy). |
| Runtime config and JSON logging | `COMPLETED` | Environment-driven config and tracing JSON are implemented. |
| Postgres, Qdrant, MinIO local infrastructure | `COMPLETED` | `docker-compose.yml` exposes ports `17675`-`17679`. |
| Baseline schema and migrations | `COMPLETED` | Eight bundled migrations cover baseline, sessions, source indexes, ingestion, hybrid retrieval, retry backoff, evaluation reports, and backup records. |
| First-run setup and local login/session | `COMPLETED` | Setup token, first admin, password hashing, login, cookie session, and `/me` exist. |
| Projects and source registry API | `COMPLETED` | List/create/get project and register/list/get source are DB-backed. |
| Approval flow | `COMPLETED` | List/detail/approve/reject, initial chunk creation, and audit events exist. |
| Git ingestion MVP | `COMPLETED` | Allowlist validation, TruffleHog gate, parser/chunker, stale cleanup, trusted auto-approval, and job lifecycle exist. |
| Voyage-4 and Qdrant clients | `COMPLETED` | Provider clients, collection setup, durable jobs, and backfill are implemented. |
| Hybrid retrieval and RRF | `COMPLETED` | Semantic plus Postgres FTS works with strict-weighted relaxed OR query fallback. |
| Rerank + compress pipeline | `COMPLETED` (local main 2026-07-18) | Pool RRF → hydrate → Voyage rerank fail-open → near-dup compress (prefer trusted). Flags + diagnostics on all surfaces. Runbook: [`runbooks/hybrid-retrieval.md`](./runbooks/hybrid-retrieval.md). Prod image may lag until redeploy. |
| Admin retrieval Playground | `COMPLETED` (local main 2026-07-18) | `/admin/playground` SSR form + results; not Evaluation Admin product. |
| Embedding pacing and graceful stop | `COMPLETED` | Paced batches requeue and unlock jobs instead of sleeping while holding a running job. |
| Evaluation baseline | `COMPLETED` (CLI) | Shared executor via `queria-cli eval run`; Admin evaluation HTTP routes removed. |
| MCP HTTP transport | `COMPLETED` | `initialize`, `tools/list`, and `tools/call` work with agent-token authorization. |
| MCP agent tools | `COMPLETED` | Agent surface: `retrieve_context`, `search_knowledge`, `propose_memory`, `list_projects`, `get_source`, `index_memory` (scratch). Optional `rerank`/`compress` on retrieve/search. Maintainer actions stay on session Admin HTTP, not MCP. |
| Agent-driven onboarding docs | `COMPLETED` (prod 2026-07-18) | Public `GET /api/v1/docs/agent-setup` (alias `/docs/setup`), `GET /api/v1/setup/mcp-snippet`, `GET /api/v1/setup/agents-block`. Live on edge `:17674`. LLM applies MCP + AGENTS.md on the agent machine. Runbook Part C: [`runbooks/onboarding.md`](./runbooks/onboarding.md). |
| Agent auto-retrieve hooks (hybrid) | `COMPLETED` (local main 2026-07-19) | T4+R6+H1: `POST /api/v1/agent/retrieve-context` + `GET /api/v1/agent/projects` (Bearer agent token, same authz as MCP). Setup: `/api/v1/setup/hooks-snippet?client=droid\|claude`, `/setup/hook-script`, script `agent-tools/hooks/queria-retrieve-hook.sh`. Stronger AGENTS block. SessionStart + throttled UserPromptSubmit inject, fail-open. **Prod image may lag.** Design: [`archive/superpowers/specs/2026-07-19-agent-auto-retrieve-hooks-design.md`](./archive/superpowers/specs/2026-07-19-agent-auto-retrieve-hooks-design.md). |
| Agent path edge E2E script | `COMPLETED` (local main; run on demand) | `scripts/e2e_agent_path_edge.py` E0–E12 against edge `:17674` with pre-minted smoke token. Spec: [`archive/superpowers/specs/2026-07-19-agent-path-edge-e2e-design.md`](./archive/superpowers/specs/2026-07-19-agent-path-edge-e2e-design.md). Operator green run pending (script shipped; mark green only after one successful operator run). |
| Multi-org isolation MVP | `COMPLETED` (local main 2026-07-18) | Migration `20260718000100_multi_org_tenancy` (`org_membership`, `org_invite`, `is_platform_super_admin`, `user_session.active_organization_id`, one-org unique, backfill). Session binds active org + super-admin (DB flag and/or `QUERIA_PLATFORM_SUPER_ADMIN_EMAILS`). API: `POST/GET /api/v1/orgs`, invites, accept, current members. Admin: `/admin/orgs`, `/admin/invites/accept`, `/admin/members`. Tenant handlers + MCP/agent tokens filter home org only; SA without membership 403s tenant routes. **Prod image not redeployed** for multi-org yet. Product note: [`PRODUCT.md`](./PRODUCT.md). Ops: section **Multi-org isolation MVP** below. |
| Local multi-git `index-here` (IMP-L1…L5) | `COMPLETED` (main 2026-07-19/20) | Schema `needs_review`; gates/slug; agent `POST /api/v1/agent/index-local` + `IndexLocal`; CLI `index-here` (nested path filter); retrieve `include_needs_review` default false; Admin `/admin/needs-review`; MCP `ManageNeedsReview`. Smoke: [`scripts/e2e_index_here_edge.py`](../scripts/e2e_index_here_edge.py). Design: [`archive/superpowers/specs/2026-07-19-local-git-index-here-design.md`](./archive/superpowers/specs/2026-07-19-local-git-index-here-design.md). **Residual:** operator green run of smoke script + host migrate/redeploy for `needs_review`. IMP-L6 **not** shipped. |
| Laptop hub TUI (`queria-cli tui`) | `COMPLETED` (branch `feat/queria-cli-hub-tui` 2026-07-22) | Hub: Doctor friction checks, Index wizard (`index-here`), Status via agent `projects-status`, Config TUI. No AppConfig/SETUP_TOKEN on laptop path. `queria-cli doctor mcp` remains non-TUI. **Embeddings status** stays maintainer/server (`embeddings status` + DB). Docs: [`runbooks/onboarding.md`](./runbooks/onboarding.md). Residual: release tag after merge (package bump optional). |
| Admin-oriented API | `COMPLETED` | Dashboard, audit logs, approvals, jobs, sources, tokens, needs-review promote (no evaluations HTTP). |
| Edge reverse proxy | `COMPLETED` | Caddy path router (`docker/Caddyfile`) for `/api/`, `/mcp`, admin, and health on host port `17674`. Pingora/`queria-proxy` removed in P1. |
| Astro Admin UI | `COMPLETED` | Violet Void dark SSR pages; pure Astro (no React islands). SIMPLIFICATION P0 applied 2026-07-16. |
| S3 backup and restore drill | `COMPLETED` | Backup in `queria-backup`; restore-drill lives only in `queria-cli` (removed from lib). Live empty-volume restore remains acceptance. |
| Production OCI packaging | `COMPLETED` | Dockerfiles, production Compose, deployment/rollback runbooks. Stack is deployed; Phase 7 acceptance pack still open. |

### Human UI Screens

| Screen / surface | Status | Entry point / honesty note |
|---|---|---|
| Setup Wizard | `COMPLETED` | `/admin/setup` |
| Login / Logout | `COMPLETED` | `/admin/login`, `/admin/logout` |
| Dashboard | `COMPLETED` | `/admin/dashboard` stat cards + embedding bar + latest job/eval panels |
| Projects | `COMPLETED` | `/admin/projects` with create-project dialog |
| Sources | `COMPLETED` | `/admin/sources`, `/admin/sources/detail` — **Register Git Source** form (uri, title, branch, optional path) + per-source **Trigger Ingest**; embedding counts on source detail |
| Knowledge Items | `COMPLETED` | `/admin/knowledge` |
| Approval Queue | `COMPLETED` | `/admin/approvals` — native HTML <dialog> confirm UI for approve/reject (SSR POST) |
| Ingestion Jobs | `COMPLETED` | `/admin/jobs` (primary place for job lifecycle; embedding work shows up as jobs) |
| Embedding Status | `EMBEDDED` | No dedicated `/admin/embedding` route. Visible via dashboard summary, source detail chunk-state counts, jobs list, and CLI `embeddings status`. |
| Retrieval Probe / Playground | `COMPLETED` | Dedicated lean SSR `/admin/playground` (nav: Playground). Session probe reuses `POST /api/v1/projects/{slug}/retrieval/probe` with rerank/compress toggles, scores, lane, diagnostics. Eval remains CLI only. CLI `retrieval probe` flags still available. |
| Agent Tokens | `COMPLETED` | `/admin/tokens` — mint requires **name** + at least one **project_slugs** (checkbox multi-select); token shown once |
| Organizations (platform) | `COMPLETED` (local main) | `/admin/orgs` — super-admin list/create; one-time invite token after create |
| Invite accept | `COMPLETED` (local main) | Public `/admin/invites/accept` (token + password); no SMTP |
| Org members | `COMPLETED` (local main) | `/admin/members` — home org members + further invite |
| Needs review (local index) | `COMPLETED` (branch 2026-07-19) | `/admin/needs-review` — list by origin/commit; Promote / Reject (single + bulk); copy-paste `index-here` panel. User term **Needs review** (not quarantine). |
| Audit Logs | `COMPLETED` | `/admin/audit` |
| Evaluation | `CLI` | Admin page + evaluation HTTP removed. Run `queria-cli eval run --project <slug>`; dashboard may show last report if present |
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

### Stack identity (GHCR CI path added 2026-07-19; edge `:17674` + subdomain)

| Field | Value |
|---|---|
| Host deploy path | `/home/ubuntu/queria-backend` |
| Primary image path | **GHCR** `ghcr.io/nandocoeg2/queria-backend/backend` + `.../admin` (`linux/arm64`); workflow [`.github/workflows/deploy.yml`](../.github/workflows/deploy.yml) |
| Fallback source sync | **rsync from workstation** + host `compose build` (host GitHub SSH cannot `git fetch`) |
| Image / commit | Host build 2026-07-19 as `ghcr.io/.../backend:latest` + `admin:latest` (`QUERIA_SOURCE_COMMIT=e4f24a6` tree); **not** yet pulled from registry |
| Edge service (live) | `queria-backend-queria-edge-1` image `caddy:2.10-alpine`, host port **`17674`** |
| Public hostname | **`https://queria.fjulian.id`** live (Nginx → `127.0.0.1:17674`, Certbot LE expires ~2026-10-17); Nginx still owns host 80/443 |

| Legacy proxy | **removed** |
| API / MCP / worker | Multi-binary image package **`backend`**; Admin package **`admin`** |
| Postgres / Qdrant | **healthy**; volumes **not wiped** |
| MinIO | `Up` (volume preserved) |
| Schema | migrate idempotent `{"status":"migrated"}` on redeploy (multi-org migration only after image that includes it) |
| Org | `fjulian` (1 user/admin; setup already consumed) |
| Projects | **1** — slug `fjulian-me` |
| Public smoke | `http://168.110.214.130:17674/healthz` 200; `https://queria.fjulian.id/healthz` after Nginx+LE |


Verified live stack after redeploy (2026-07-17):

| Service | Notes |
|---|---|
| `queria-backend-queria-edge-1` | Public host port `17674` (Caddy; Server header `Caddy`) |
| `queria-backend-queria-api-1` | Image dual-lane; `/usr/local/bin/queria-cli` present |
| `queria-backend-queria-mcp-1` | Dual-lane image |
| `queria-backend-queria-worker-1` | Dual-lane image |
| `queria-backend-queria-admin-1` | Internal (`4321`) |
| `queria-backend-postgres-1` | Healthy; volume preserved |
| `queria-backend-qdrant-1` | Healthy; volume preserved |
| `queria-backend-minio-1` | Running |

Edge health after redeploy + explicit migrate:

```bash
curl -sS -o /tmp/healthz.out -w "%{http_code}" http://127.0.0.1:17674/healthz
# http_code=200 body=OK  (Server: Caddy)
docker compose -f docker-compose.production.yml run --rm --no-deps queria-api queria-cli database migrate
# {"status":"migrated"}  (idempotent; migrations already included dual-lane)
```

**Note:** runbook `deployment.md` historically used `queria-api database migrate` without the `queria-cli` binary name; production entrypoint requires `queria-cli database migrate`.

Host resource snapshot: ~11 GiB RAM, ~188G disk with ~144G free, Docker on OCI aarch64.

Same host also runs unrelated shared workloads (monitoring, other app DBs, `grok2api`, etc.). Do not treat the box as Queria-only when planning ports, disk, or restarts.

### Mission ops acceptance pack (2026-07-17, measure-only — historical)

Earlier same day, before seed: project missing; status/probe/eval all exited 1 with `admin or project not found`. See git history of this section if needed. **Superseded by seed pack below.**

### Mission ops seed pack (2026-07-17, `ops-prod-seed-fjulian-me`)

**Allowed for this feature:** create project `fjulian-me`, register Git source `git@github.com:nandocoeg2/fjulian.me.git`, ingestion + embedding backfill, then status/probe/**one** eval + HANDOFF.  
**Not done:** full image rebuild/redeploy, volume wipe, second eval run, dual-lane `index_memory` on prod.

| Check | Result |
|---|---|
| Edge healthz | **HTTP 200**, body `OK` (Caddy) |
| Project | slug **`fjulian-me`** exists (org `fjulian`, project id `9e5d90ee-c782-457e-98b1-86ff85cffb6a`) |
| Git source | `git@github.com:nandocoeg2/fjulian.me.git` active; local checkout `/tmp/seed10001/fjulian.me` (allowlist host `github.com`, repo `nandocoeg2/fjulian.me.git`) |
| Git ingest | job `a8e589f9-…` **succeeded**: 231 files, **1213** knowledge items, **1229** chunks (all initially pending) |
| TruffleHog fix | image lacked `config/trufflehog-*.txt`; worker bind-mount `./config:/config:ro` + absolute env paths; first fail was missing include paths (not real secrets) |
| Embedding backfill | job `6528e606-…` enqueued; Voyage **429** rate limits with batch 8 / 2s interval; ready increased 0 → **72** during seed session |
| Embeddings status | **exit 0** JSON (see residual below) |
| Retrieval probe | **exit 0**; structured `items` (5) + `retrieval.mode` hybrid for both golden-ish queries |
| Golden eval (once) | **3/3 passed**, regression **1.0**, report id `6c8b5df9-89e4-45c1-8fda-e76b5a4ec567` persisted |

**Production embeddings residual for `fjulian-me` (2026-07-17 seed session, post-probe/eval):**

```json
{
  "project": "fjulian-me",
  "project_exists": true,
  "embedding_profile_version": "voyage-4-1024-v1",
  "counts": {
    "ready": 72,
    "pending": 1005,
    "failed": 152,
    "processing": 0,
    "stale": 0
  },
  "knowledge_items_approved": 1213,
  "chunks_total": 1229,
  "ingest_job": "succeeded",
  "backfill_job_status": "queued (retrying; Voyage 429 residual)",
  "cli_exit": 0
}
```

**Probe notes (seed session):**

- Query `deployment and site build notes` → 5 items, `retrieval.mode=hybrid`, first path `src/entry-server.tsx`, status/lane `approved`/`trusted`.
- Query `Astro markdown content flow` → 5 items, hybrid, citation paths present.
- Read-ish after seed writes; no second backfill enqueue for probes.

**Eval (exactly one run this seed session):**

| Field | Value |
|---|---|
| Project | `fjulian-me` |
| Report id | `6c8b5df9-89e4-45c1-8fda-e76b5a4ec567` |
| Status | `passed` |
| Total / passed / failed | **3 / 3 / 0** |
| Regression score | **1.0** |
| Failing questions | **none** |
| Modes observed | hybrid + lexical_fallback mix (semantic partial while embeddings residual) |
| Mission note | Single prod eval only; Phase 7 golden **3/3 met** on content criteria |

### Mission ops backfill restatus (2026-07-17, `ops-prod-embedding-backfill-restatus`)

**Allowed:** poll embeddings status, optional probe, HANDOFF residual; worker continues pacing.  
**Not done:** volume wipe, backfill re-enqueue, second eval, deploy/restart/migrate.

| Check | Result |
|---|---|
| Edge healthz | **HTTP 200**, body `OK` (Caddy) |
| Containers | api/mcp/worker/edge/admin/postgres/qdrant/minio **Up** (same dual-lane image) |
| Embeddings status | **exit 0** (JSON below) |
| Progress vs seed | ready **72 → 1226**; pending **1005 → 0**; failed **152 → 3** |
| Backfill job `6528e606-…` | **succeeded** (attempts ~55; Voyage 429 retries with batch 8 / ~2s interval) |
| Retrieval probe (optional) | **exit 0**; hybrid 5 hits for `deployment and site build notes` (README scripts/Docker deploy paths; semantic candidates 20) |
| Second eval | **skipped** (default; counts improved substantially but prior golden 3/3 and no user-facing regression suspected) |
| Volume wipe | **none** |

**Production embeddings residual for `fjulian-me` (2026-07-17 restatus, final poll ~13:08 UTC):**

```json
{
  "project": "fjulian-me",
  "project_exists": true,
  "embedding_profile_version": "voyage-4-1024-v1",
  "counts": {
    "ready": 1226,
    "pending": 0,
    "failed": 3,
    "processing": 0,
    "stale": 0
  },
  "chunks_total": 1229,
  "knowledge_items_approved": 1213,
  "backfill_job_status": "succeeded",
  "failed_error_class": "Voyage 429 Too Many Requests (all 3 residual)",
  "failed_sample_titles": [
    "src/utils/siteOutput.ts: StaticFileOutput",
    ".agents/skills/vercel-react-best-practices/rules/js-index-maps.md: Build Index Maps for Repeated Lookups",
    ".agents/skills/code-review/references/language/rust.md: `unsafe`"
  ],
  "worker_pacing": {
    "QUERIA_EMBEDDING_BATCH_SIZE": 8,
    "QUERIA_EMBEDDING_REQUEST_INTERVAL_MS": 2000,
    "QUERIA_EMBEDDING_MAX_RETRIES": 3
  },
  "poll_timeline_ready": [1104, 1152, 1200, 1224, 1226],
  "cli_exit": 0
}
```

**Probe notes (restatus):** query `deployment and site build notes` → 5 items, `retrieval.mode=hybrid`, `semantic_candidates=20`, top paths `README.md` (Scripts / Docker / Portainer deploy). Read-ish only.

**Ops open issues (after restatus):**

1. ~~Prod has no projects/knowledge~~ **Resolved** (`fjulian-me`, 1213 items / 1229 chunks).
2. ~~Embedding residual still large~~ **Mostly resolved** — ready **1226** / pending **0** / failed **3** (Voyage 429). No wipe; optional later bounded retry of the 3 failed if quota allows (not required for golden DoD).
3. ~~**TruffleHog config not in runtime image**~~ **Committed on `main`** — Dockerfile `COPY config/trufflehog-*.txt` + absolute `QUERIA_TRUFFLEHOG_*` env. **Prod image may lag** until next redeploy (seed used host `/config` bind-mount when needed).
4. **Inactive Mac path source** remains deactivated; only GitHub SSH source is active.
5. ~~Runtime edge `queria-proxy`~~ **Resolved** (Caddy on `:17674`).
6. Optional: bake worker pacing (`QUERIA_EMBEDDING_BATCH_SIZE=8`, interval ≥2s) into permanent prod `.env` to reduce 429 churn on future large ingests.

Security:

- Never paste the RSA private key into git, chat history, or docs beyond the local path above.
- Workspace `.gitignore` already ignores `*.key`.
- Prefer Infisical for app secrets; host `.env` files are emergency/runtime only.

## Current Local State

The first project is `fjulian-me`, sourced from:

```text
/Users/fernandojulian/project/fjulian/fjulian.me
```

**Historical local** embedding snapshot observed on 2026-07-05 (not production):

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

**Production (2026-07-17 restatus):** project `fjulian-me` present; embeddings ready **1226** / pending **0** / failed **3** (backfill job succeeded; see Mission ops backfill restatus).

## Latest Verified Retrieval Finding

Historical gap (pre-Phase-1): the golden query `deployment and site build notes`
failed under strict-only `websearch_to_tsquery('simple', $query)` because
`simple` kept `and` and AND-combined every term.

**Resolved in code:** hybrid lexical SQL now uses strict-weighted matches plus a
bounded relaxed OR path; RRF still combines lexical and semantic rankings.
Auth, approved status, active source, organization, project, and global-scope
filters remain inside both SQL paths.

**Production re-verify (2026-07-17 seed + restatus):** probe for `deployment and site build notes`
returns structured hybrid hits (5 items) with semantic candidates populated after
backfill completion (ready 1226). Golden eval remains 3/3 from the single seed run.

## Latest Evaluation Result

### Production seed acceptance (2026-07-17) — one allowed run

Command (on prod host; host `.env` + golden file from deploy tree):

```bash
docker run --rm --network container:queria-backend-queria-api-1 \
  --env-file /home/ubuntu/queria-backend/.env \
  -v /home/ubuntu/queria-backend/tests:/workdir/tests:ro \
  -w /workdir --entrypoint /usr/local/bin/queria-cli \
  queria-backend:latest eval run --project fjulian-me
```

Observed:

- CLI exit: **0**
- Report id: `6c8b5df9-89e4-45c1-8fda-e76b5a4ec567`
- total: **3** / passed: **3** / failed: **0**
- regression score: **1.0**
- status: **passed**
- `evaluation_report` insert: **yes** (1 row)
- **Not** a second run for score shopping. Phase 7 golden content DoD **met**.

### Historical measure-only (2026-07-17 earlier, pre-seed)

- Exit 1 `admin or project not found`; no report. Superseded by seed run above.

### Historical local only (2026-07-05)

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
persist reports. Do **not** close Phase 7 on the historical local 2/3 result alone.
Production re-measure (above) did not produce a content score either.

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

## Multi-org isolation MVP (local main 2026-07-18)

Product contract: [`PRODUCT.md`](./PRODUCT.md) § Multi-organization tenancy.  
Team B path for operators: [`runbooks/onboarding.md`](./runbooks/onboarding.md) Part D.

### What shipped (code on `main`)

| Layer | Behavior |
|---|---|
| Schema | Bundled migration `20260718000100` / `multi_org_tenancy`: `org_membership` (unique on `user_id`), `org_invite` (hash+prefix only), `user_account.is_platform_super_admin` default false, `user_session.active_organization_id`, membership backfill from existing users |
| Session | Login binds `active_organization_id` from sole membership; `/api/v1/auth/me` exposes `active_organization_id`, `active_organization_slug`, `is_platform_super_admin` |
| Platform API | `POST/GET /api/v1/orgs` super-admin only; body create `{ slug, name, first_admin_email }` → response includes `invite_token` **once** |
| Invites | `POST /api/v1/orgs/{slug}/invites`, public `POST /api/v1/invites/accept` `{ token, password, name? }`; `GET /api/v1/orgs/current/members` |
| Isolation | Tenant Admin/API require active org (403 if missing). Agent token mint binds home org; Qdrant/search filters `organization_id = home`. Super-admin without membership cannot list/retrieve tenant knowledge |
| Admin UI | Orgs nav only when `is_platform_super_admin`; `/admin/orgs`, `/admin/invites/accept`, `/admin/members` |

**v1 non-goals (do not expect; not bugs):** share grants, per-org git allowlist, multi-membership org switcher, SMTP mailer trait, production redeploy as part of this feature.

### Super-admin bootstrap (ops)

Both paths make `require_platform_super_admin` succeed at session load. Prefer documenting **both** so local/prod match.

**1. Env (comma-separated, case-insensitive match):**

```bash
# e.g. in local .env or prod runtime env
export QUERIA_PLATFORM_SUPER_ADMIN_EMAILS='nando@fjulian.id'
# multiple: email1@example.com,email2@example.com
```

Restart `queria-api` after changing env. Existing login sessions re-evaluate flag on load.

**2. One-time SQL (DB flag):**

```sql
update user_account
set is_platform_super_admin = true
where lower(email) = lower('nando@fjulian.id');
```

Verify:

```sql
select email, is_platform_super_admin from user_account
where lower(email) = lower('nando@fjulian.id');
```

Then login → `GET /api/v1/auth/me` shows `"is_platform_super_admin": true`. Super-admin **without** `org_membership` has null active org: `POST/GET /api/v1/orgs` work; `GET /api/v1/projects` (and other tenant routes) return **403** `active_organization_required`.

### Migrate note

```bash
# local (with QUERIA_DATABASE_URL)
cargo run -p queria-cli -- database migrate
# expect {"status":"migrated"} — multi_org_tenancy is in the bundled list
```

After migrate, legacy users with `user_account.organization_id` have a backfilled `org_membership` and keep single-org login.

### Isolation smoke checklist (ops / validators)

1. Flag super-admin (env and/or SQL above).
2. As SA: create org Team B with `first_admin_email` → capture **one-time** `invite_token` (Admin `/admin/orgs` or API). Token must **not** reappear on list/refresh.
3. Public accept → login as Team B admin → `/admin/projects` empty of Team A data; create a project under B only.
4. Session A vs B: `GET /api/v1/projects` sets are disjoint (no cross-org rows).
5. SA without membership: `GET /api/v1/projects` → 403; `GET /api/v1/orgs` → 200.
6. No SMTP required for any step. Do not block on share grants / git-per-org / switcher.

### Prod note

Live host image listed under **Stack identity** is still pre–multi-org. Redeploy + migrate on production is **post-mission** ops only (out of feature work). Do not wipe volumes.

## Security Boundaries

- Never commit provider keys, Cloudflare credentials, setup tokens, sessions, agent tokens, or **raw invite tokens**.
- Infisical is the primary runtime secret source; `.env` remains local fallback only.
- Raw agent tokens and raw invite tokens are shown once; Postgres stores token prefix and hash.
- Project Git paths and SSH repositories must pass explicit allowlists (instance-level; not per-org in v1).
- TruffleHog must pass before trusted Git auto-approval.
- Agent proposals never receive trusted Git auto-approval.
- Global retrieval requires both `include_global=true` and token permission (**global = org-global trusted**, not cross-tenant).
- Super-admin is platform catalog only; membership is required for tenant knowledge access.
- Database writes, migrations, dependency additions, pushes, and deployments require explicit approval.

## Residual Gaps (current)

### Runtime / config notes

- Onboarding friction pack **code shipped on `main`**: Admin Daily mint + connect panel; dashboard “Get ready for agents”; `request_base` prefers `QUERIA_PUBLIC_BASE_URL` (prod `https://queria.fjulian.id`). Spec (historical): [`archive/superpowers/specs/2026-07-20-onboarding-friction-pack-design.md`](./archive/superpowers/specs/2026-07-20-onboarding-friction-pack-design.md). **Docs** lead with Daily **3-step** default (mint → env+MCP → retrieve); Admin Git / index-here optional; direnv not required for Daily. Live copy: `GET …/docs/agent-setup`. Operator path: [`runbooks/onboarding.md`](./runbooks/onboarding.md). Ops residual: confirm host image has Daily UI + `QUERIA_PUBLIC_BASE_URL` set (not a re-open of UI implementation).
- **CLI / Homebrew / deploy residual (one list — do not oversell):**
  1. **CLI Release not verified from unauth API** — `queria-backend` is private; unauthenticated curl/API on Release assets returns **404** (not proof assets are missing). Operator confirms in Actions UI / Releases UI (logged in) or `GH_TOKEN` / `gh release view`. Stage 2 [`.github/workflows/release-cli.yml`](../.github/workflows/release-cli.yml) builds on tag `cli-v*` (or `workflow_dispatch` with tag). **Push `main` with a Cargo.toml version bump** triggers Stage 1 detect-and-tag → Stage 2 → Stage 3 Homebrew; plain feature pushes do not release.
  2. **First arm64 GHCR deploy seeds `buildcache`** — deploy workflow uses native `ubuntu-24.04-arm` + BuildKit mounts + registry tags `backend:buildcache` / `admin:buildcache` (since `bf76180`). First green run after that change still pays a full compile to seed cache; later warm rebuilds expected faster. Details: [`runbooks/deployment.md`](./runbooks/deployment.md) § Build speed / cache.
  3. **Homebrew after Release** — Stage 3 **CLI Homebrew formula** direct-pushes `nandocoeg2/homebrew-queria` when `HOMEBREW_TAP_TOKEN` is set; else manual `scripts/generate_homebrew_formula.sh` → push tap. Laptop still needs `HOMEBREW_GITHUB_API_TOKEN` while backend private. Runbook: [`runbooks/queria-cli-homebrew.md`](./runbooks/queria-cli-homebrew.md).
  4. **Daily onboard is independent of CLI/Brew** — Default Daily path (mint token → env + MCP → `retrieve_context`) needs **no** `queria-cli` and **no** Homebrew. CLI install is only for optional laptop hub/index-here (or maintainers).
  5. **queria-cli hub TUI** — Laptop path (no `SETUP_TOKEN`): `queria-cli tui` → **Doctor / Index / Status / Config**. Residual: version bump (careful — triggers release chain) so Release assets include hub TUI. Spec: [`archive/superpowers/specs/2026-07-21-queria-cli-config-design.md`](./archive/superpowers/specs/2026-07-21-queria-cli-config-design.md). Not required for Daily MCP.

| Gap | Priority | Notes |
|---|---|---|
| Production empty seed | **Resolved** | `fjulian-me` seeded 2026-07-17; 1213 items / 1229 chunks; golden eval **3/3**. |
| Production embeddings residual | **Low** | Restatus 2026-07-17: ready **1226** / pending **0** / failed **3** (all Voyage 429). Backfill job **succeeded**. Optional later retry of 3 failed only if needed; no wipe. |
| Production acceptance pack | Medium | Healthz, stack identity, embeddings status, probe, **eval 3/3** recorded. Remaining Phase 7: MCP client smoke, backup restore drill, SLO spot-check. |
| Edge still `queria-proxy` | **Resolved** | Live edge is Caddy `queria-edge` after 2026-07-17 redeploy. |
| Prod container env drift | Medium | CLI still prefers host `--env-file` for some flags; compose `env_file` mostly aligned post-redeploy. |
| TruffleHog config in image | **Low** (code done) | Committed on `main` (Dockerfile COPY + env). Prod image may lag; seed used host `/config` bind-mount when needed. |
| Hard simplification cuts | Done (P0–P3) | See [`SIMPLIFICATION.md`](./SIMPLIFICATION.md). |
| Admin UI dedicated routes | Low | Sources form + Trigger Ingest and tokens (name + project_slugs) shipped. Embedding / backup remain embedded or CLI-only. Playground for retrieval probe. |
| Maintainer MCP tools | Deferred by design | Approve/reject/reindex/token admin remain Admin HTTP; agent MCP does not expose maintainer mutations. |
| Production redeploy for retrieval quality / multi-org / Admin polish | Medium | Local `main` ahead of live host image. Redeploy only when operator requests (out of this docs feature). |
| Future product improvements | REFERENCE backlog | IMP-01/02/03 done on local main; IMP-L1…L5 done on feature branch. Still open: IMP-04 metrics, IMP-15/16 Admin scratch/promote, agent DX, IMP-L6 auto-promote (deferred). [`IMPROVEMENTS.md`](./IMPROVEMENTS.md) / [`PRODUCT.md`](./PRODUCT.md). |
| Multi-org on production | Medium | Code + docs on local `main`; host image still pre-multi-org. After local validators green: redeploy, migrate, flag super-admin, smoke Team B path. **Not** share grants / switcher / SMTP. |
| index-here on production | Medium | Code on local `main`. Needs Path A/B redeploy + `queria-cli database migrate` for enum `needs_review`. Smoke: `python3 scripts/e2e_index_here_edge.py` with `QUERIA_AGENT_TOKEN` (`index_local`). |

## Post-audit simplification

Ponytail-audit (over-engineering) findings are tracked in
[`SIMPLIFICATION.md`](./SIMPLIFICATION.md). Hard mode agreed cuts:

| Band | Intent | Status |
|---|---|---|
| P0 | Drop dead shadcn kit + Three.js dashboard graph | **DONE** 2026-07-16 |
| P1 | Replace Pingora with Caddy; fold observability; prune dead db traits | **DONE** 2026-07-16 |
| P2 | Defer evaluation Admin UI + HTTP; restore-drill CLI-only; drop `proxy_addr` | **DONE** 2026-07-16 |
| P3 | enowx-rag Qdrant-only; remove Chroma/pgvector/OpenAI stubs | **DONE** 2026-07-16 |
| Closeouts | mockall demotion, runbook sync, leftover trait/cfg work | **DONE** 2026-07-16 |
| Impact | Fold auth into core; demote search mockall to dev-deps via hand fakes | **DONE** 2026-07-16 |
| Deep cuts | Kill mockall; nest AppConfig; split repositories; move restore_drill to CLI | **DONE** 2026-07-16 |

Do not treat archived e2e plans under [`archive/superpowers/`](./archive/superpowers/)
as the active roadmap.

## Continue From Here

Feature scaffolding for Phases 1–6 is done. Immediate work:

**Ops acceptance (status after 2026-07-17 seed + restatus)**

1. ~~Measure edge health + stack identity~~ **done** (healthz 200; Caddy edge; dual-lane image).
2. ~~Create project / Git ingest / one eval~~ **done** (`fjulian-me`, eval 3/3).
3. ~~Embedding backfill restatus~~ **done** (ready 1226 / failed 3; job succeeded; HANDOFF residual updated).
4. Remaining acceptance: MCP client smoke, scopes, backup restore drill, SLO spot-check (still open).
5. Optional ops: bake embedding pacing into prod `.env`; **redeploy** so live image picks up TruffleHog bake-in, multi-org, retrieval quality, and Admin sources/tokens UX (prod image may lag local `main`).

**Post-cut**

6. SIMPLIFICATION P0–P3 applied 2026-07-16; prod redeployed to Caddy + dual-lane image 2026-07-17.
7. Keep maintainer tools off the agent MCP surface unless product requires otherwise.

**Product improvements**

8. Retrieval quality IMP-01/02 + Admin Playground IMP-03 shipped on **local main** (2026-07-18); docs/runbook aligned. Next backlog: durable metrics (`IMP-04`), Admin scratch/promote (`IMP-15`/`16`), agent DX. Contract: [`PRODUCT.md`](./PRODUCT.md). Do not mark done without updating this handoff.

9. Local multi-git **index-here** IMP-L1…L5 on `main` (2026-07-19/20): nested ls-files filter + `scripts/e2e_index_here_edge.py`. Next: host migrate/redeploy, mint token with `index_local`, run smoke; **not** IMP-L6.

10. **Laptop hub TUI shipped** (`feat/queria-cli-hub-tui`): `queria-cli tui` → Doctor / Index / Status / Config. Laptop Status = agent projects-status; full embeddings status remains server-maintainer. Docs updated in onboarding + this handoff. Next: merge, optional package bump, tag `cli-v0.3.0` per release workflow (not required for Daily MCP).

**Multi-org (local complete; prod follow-up)**

11. Isolation MVP is on local `main` (schema, session, orgs/invites, Admin, enforce). Bootstrap + Team B path: **Multi-org isolation MVP** section and [`runbooks/onboarding.md`](./runbooks/onboarding.md) Part D. After validators: production redeploy + migrate + super-admin flag only when operator requests.

