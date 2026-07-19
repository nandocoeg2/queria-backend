# queria-backend

> Status: CURRENT - core product live; residual ops acceptance.
> Last verified: 2026-07-19.
> Start with [`docs/HANDOFF.md`](docs/HANDOFF.md). Product: [`docs/PRODUCT.md`](docs/PRODUCT.md). Cuts: [`docs/SIMPLIFICATION.md`](docs/SIMPLIFICATION.md). Backlog: [`docs/IMPROVEMENTS.md`](docs/IMPROVEMENTS.md).

Queria backend workspace for centralized team and agent knowledge.

## Status

Implementation matrix, production host, and residual gaps: **[`docs/HANDOFF.md`](docs/HANDOFF.md)** only.

Short pointers:

- Dual-lane (scratch / trusted): [`docs/PRODUCT.md`](docs/PRODUCT.md)
- Multi-org rules: PRODUCT § Multi-organization tenancy; ops steps: [`docs/runbooks/onboarding.md`](docs/runbooks/onboarding.md) Part D
- Rerank / compress / Playground: [`docs/runbooks/hybrid-retrieval.md`](docs/runbooks/hybrid-retrieval.md) (prod image may lag; see HANDOFF)

## Docs

| Doc | Role |
|---|---|
| [`docs/HANDOFF.md`](docs/HANDOFF.md) | Canonical current state |
| [`docs/PRODUCT.md`](docs/PRODUCT.md) | Product contract |
| [`docs/runbooks/onboarding.md`](docs/runbooks/onboarding.md) | Admin → agent onboard |
| [`docs/runbooks/agent-onboard-prompt.md`](docs/runbooks/agent-onboard-prompt.md) | One-paste agent client setup (dialogs) |
| [`docs/runbooks/`](docs/runbooks/) | Local, deploy, retrieval, backup, rollback |
| [`docs/README.md`](docs/README.md) | Full docs index |

## Production deploy (short)

- **Primary:** push `main` → GHCR arm64 (`backend`, `admin`) → SSH compose pull/up.
- **Public:** Caddy host `:17674`; Nginx + Certbot `https://queria.fjulian.id` → `127.0.0.1:17674`.
- Detail: [`docs/runbooks/deployment.md`](docs/runbooks/deployment.md). Residual: HANDOFF.

## Local services

```bash
docker compose up -d postgres qdrant minio
cargo run -p queria-api
cargo run -p queria-worker
```

Copy `.env.example`, set secrets / `QUERIA_SETUP_TOKEN`. Worker needs `git` + TruffleHog 3.x on `PATH`.

Ports, migrate, embeddings pacing: [`docs/runbooks/local-development.md`](docs/runbooks/local-development.md).

| Service | Address |
|---|---|
| API | `http://127.0.0.1:17671` |
| MCP | `http://127.0.0.1:17672` |
| Worker health | `127.0.0.1:17673` |
| Edge (Caddy) | `http://127.0.0.1:17674` |
| Postgres | `127.0.0.1:17675` |
| Qdrant | `127.0.0.1:17676` |
| MinIO | `http://127.0.0.1:17678` |

## Agent client: keys for one workspace, many repos

Retrieve is always **per `project_id`**. Scratch never crosses projects. Full Admin path: [`docs/runbooks/onboarding.md`](docs/runbooks/onboarding.md). One-paste client setup: [`docs/runbooks/agent-onboard-prompt.md`](docs/runbooks/agent-onboard-prompt.md).

### Default setup (recommended)

1. **Admin** mints **one** agent token with all project slugs in that workspace (`project_slugs: ["repo-a", "repo-b", …]`) and tools needed (`list_projects`, `retrieve_context`, `search_knowledge`, `index_memory`, …). Copy `qria_…` once.
2. **User-level shell** (once):

```bash
export QUERIA_AGENT_TOKEN='qria_…'          # never commit
export QUERIA_EDGE_URL='http://127.0.0.1:17674'   # or https://queria.fjulian.id
export QUERIA_MCP_URL="${QUERIA_EDGE_URL}/mcp"
```

3. **MCP client** once: HTTP MCP at `$QUERIA_MCP_URL` with Bearer from env (`GET $QUERIA_EDGE_URL/api/v1/setup/mcp-snippet?client=…`).
4. **Per-repo active project** (hooks). Prefer [direnv](https://direnv.net/):

```bash
# repo-a/.envrc
export QUERIA_PROJECT_SLUG=repo-a

# repo-b/.envrc
export QUERIA_PROJECT_SLUG=repo-b
```

Optional: `QUERIA_PROJECT_ID=<uuid>`. Merge `AGENTS.md` from `GET …/setup/agents-block?project_slug=…`.

| Variable | Where | Purpose |
|---|---|---|
| `QUERIA_AGENT_TOKEN` | User shell / secrets | Auth MCP + agent HTTP + hooks |
| `QUERIA_EDGE_URL` / `QUERIA_MCP_URL` | User shell | Edge base and MCP URL |
| `QUERIA_PROJECT_SLUG` or `QUERIA_PROJECT_ID` | **Per repo** | Active project for hooks / default |

### Agent loop (every repo)

```text
list_projects
retrieve_context(project_id=THIS, q)
# work
index_memory / propose_memory only on THIS project_id
```

Do **not** set one global `QUERIA_PROJECT_SLUG` for all repos when hooks are on. Do **not** expect one retrieve to merge every repo.

### Alternatives

| Pattern | When |
|---|---|
| One multi-slug token + per-repo slug (above) | Daily multi-repo workspace |
| Token per project (direnv switches both) | Least privilege |
| Token only, no slug env | Pure MCP that always `list_projects` first; weak for hooks |

### What not to do

- Commit `qria_…`
- Write scratch for project B while working in repo A
- Rely on “first project on the token” when multiple slugs are granted and hooks are enabled

## Git ingestion

Prefer Admin `/admin/sources` (Register Git + Trigger Ingest). Token mint `/admin/tokens` needs **name** + **project_slugs**. Steps: onboarding Part A.

```text
POST /api/v1/sources
POST /api/v1/sources/{source_document_id}/ingest
GET  /api/v1/ingestion-jobs?status=running&limit=50
```

## Verification

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```
