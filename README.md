# queria-backend

> Status: CURRENT - core product live; residual ops acceptance.
> Last verified: 2026-07-20.
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
| [`docs/runbooks/onboarding.md`](docs/runbooks/onboarding.md) | Default 3-step Daily onboard; optional Git / index-here |
| [`docs/runbooks/agent-onboard-prompt.md`](docs/runbooks/agent-onboard-prompt.md) | One-paste client setup after Daily mint (dialogs) |
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

Retrieve is always **per `project_id`**. Scratch never crosses projects. **Default path:** [`docs/runbooks/onboarding.md`](docs/runbooks/onboarding.md) (3-step Daily). One-paste client: [`docs/runbooks/agent-onboard-prompt.md`](docs/runbooks/agent-onboard-prompt.md).

### Default setup (recommended)

1. **Admin** mints **Daily** token with project slug(s) (`project_slugs: ["repo-a", "repo-b", …]`). Copy env from connect panel once.
2. **User-level shell** (once — session or profile; no required per-repo file):

```bash
export QUERIA_AGENT_TOKEN='qria_…'          # never commit
export QUERIA_EDGE_URL='https://queria.fjulian.id'   # or http://127.0.0.1:17674
export QUERIA_MCP_URL="${QUERIA_EDGE_URL}/mcp"
```

3. **MCP client** once: HTTP MCP at `$QUERIA_MCP_URL` with Bearer from env (`GET $QUERIA_EDGE_URL/api/v1/setup/mcp-snippet?client=…`).
4. **Work:** `list_projects` → `retrieve_context(project_id, …)`. Optional `AGENTS.md` from `GET …/setup/agents-block?project_slug=…`.

**Optional — hooks only:** if auto-retrieve hooks need an active project in a multi-root workspace, set `QUERIA_PROJECT_SLUG` (or `QUERIA_PROJECT_ID`) per repo (e.g. direnv). Not required for Daily MCP retrieve.

| Variable | Where | Purpose |
|---|---|---|
| `QUERIA_AGENT_TOKEN` | User shell / secrets | Auth MCP + agent HTTP + hooks |
| `QUERIA_EDGE_URL` / `QUERIA_MCP_URL` | User shell | Edge base and MCP URL |
| `QUERIA_PROJECT_SLUG` or `QUERIA_PROJECT_ID` | **Optional** (hooks) | Active project for auto-retrieve hooks |

### Agent loop (every repo)

```text
list_projects
retrieve_context(project_id=THIS, q)
# work
index_memory / propose_memory only on THIS project_id
```

Connect works with empty retrieve; useful answers need ready chunks. Do **not** expect one retrieve to merge every repo. Do **not** set one global slug for all folders when hooks are on.

### Alternatives

| Pattern | When |
|---|---|
| One multi-slug Daily token + `list_projects` | Default multi-repo |
| + per-repo slug / direnv | Auto-retrieve hooks multi-root |
| Token per project | Least privilege |
| Custom + `index_local` | Laptop `index-here` only (not Daily) |

### What not to do

- Commit `qria_…`
- Write scratch for project B while working in repo A
- Require direnv for plain Daily retrieve
- Rely on “first project on the token” when multiple slugs are granted and hooks are enabled

## queria-cli (index-here)

Laptop bulk index without building from source.

| Path | When |
|---|---|
| **Homebrew** | After formula published: `brew install nandocoeg2/queria/queria-cli` |
| **GitHub Release** | curl tar.gz for your OS (`cli-v*` tags) |
| **cargo** | Dev only |

- Releases: https://github.com/nandocoeg2/queria-backend/releases  
- Workflow: [`.github/workflows/release-cli.yml`](.github/workflows/release-cli.yml) — **tag `cli-v*`** only (**not** push `main`)  
- Homebrew: [`docs/runbooks/queria-cli-homebrew.md`](docs/runbooks/queria-cli-homebrew.md) · `scripts/generate_homebrew_formula.sh` · tap scaffold `../homebrew-queria/`  
- Ops: [`docs/runbooks/onboarding.md`](docs/runbooks/onboarding.md) § Install `queria-cli`

```bash
# after install + Custom token with index_local:
export QUERIA_AGENT_TOKEN='qria_…'
export QUERIA_EDGE_URL='https://queria.fjulian.id'
cd /path/to/git/project
queria-cli index-here --dry-run
queria-cli index-here   # add --yes if multiple nested git roots
```

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
