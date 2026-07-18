# Onboarding Runbook (Admin → Agent)

> Status: CURRENT  
> Last verified: 2026-07-18  
> Runtime truth: [`../HANDOFF.md`](../HANDOFF.md)  
> Local infra detail: [`local-development.md`](./local-development.md)  
> Retrieval ops: [`hybrid-retrieval.md`](./hybrid-retrieval.md)

One path to put a project into Queria, then connect coding agents over MCP.

```text
Admin (session)
  create project → register Git source → ingest/embed → issue agent token → Playground smoke
Agent (bearer token)
  configure MCP client → list_projects → retrieve_context → index_memory / propose_memory
```

## Edge URLs (do not use stale ports)

Public path routing is **Caddy** (`queria-edge`). There is **no** `queria-proxy` / Pingora path and **no** port `67671`.

| Environment | Base URL | Admin | MCP | Health |
|---|---|---|---|---|
| Local edge | `http://127.0.0.1:17674` | `/admin` | `/mcp` | `/healthz` |
| Direct local services (no edge) | API `http://127.0.0.1:17671`, MCP `http://127.0.0.1:17672` | Admin SSR often `:4321` | MCP service | API `/healthz` if exposed |
| Production host (current) | `http://168.110.214.130:17674` | `/admin` | `/mcp` | `/healthz` |

Prefer the **edge** URL for agents and browsers so path routing matches production.

```bash
curl -sS -o /tmp/queria-health.out -w "%{http_code}\n" http://127.0.0.1:17674/healthz
# expect 200 and body OK
```

If health fails, stack is not ready. Fix infra first ([`local-development.md`](./local-development.md) or [`deployment.md`](./deployment.md)). Do not onboard agents against a dead edge.

---

## Part A — Operator / Admin

Use a human session cookie via Admin UI (or authenticated Admin HTTP). Agents never perform these steps.

### A1. First-run setup (once per empty install)

1. Open `{BASE}/admin/setup` (or `/admin/login` if setup already consumed).
2. Complete first-run with the setup token from env (`QUERIA_SETUP_TOKEN` when required).
3. Log in as admin.

Production org/user for this deployment is already bootstrapped (see HANDOFF). Skip setup if login works.

### A2. Create project

Admin UI: `{BASE}/admin/projects` → **Create Project**.

| Field | Example |
|---|---|
| `slug` | `fjulian-me` (stable id used in tokens and CLI) |
| `name` | Human label |
| `include_global_default` | Enable if project retrieves may include **global trusted** knowledge |

API shape:

```bash
# Session cookie required (from browser login). Example only.
curl -sS -X POST "$API/api/v1/projects" \
  -H 'Content-Type: application/json' \
  -H "Cookie: $QUERIA_SESSION_COOKIE" \
  -d '{
    "slug": "fjulian-me",
    "name": "fjulian.me",
    "include_global_default": true
  }'
```

Record the project **slug**. MCP retrieve uses project **UUID** in `project_id` (from `list_projects` or Admin/API project detail). Keep both.

### A3. Register a Git source and ingest

Trusted code knowledge enters via the **Git pipeline** (allowlisted remote, TruffleHog, chunk, embed). Not via agent `index_memory`.

Admin UI today is strongest for **list / detail / trigger ingest** on existing sources (`/admin/sources`). Register via API if the UI has no create form:

```bash
curl -sS -X POST "$API/api/v1/sources" \
  -H 'Content-Type: application/json' \
  -H "Cookie: $QUERIA_SESSION_COOKIE" \
  -d '{
    "project_slug": "fjulian-me",
    "kind": "git_repo",
    "uri": "git@github.com:nandocoeg2/fjulian.me.git",
    "title": "fjulian.me",
    "branch": "main",
    "content_hash": "initial",
    "metadata": {}
  }'
```

Then queue ingest (UI **Ingest** on source row, or):

```bash
curl -sS -X POST "$API/api/v1/sources/$SOURCE_ID/ingest" \
  -H "Cookie: $QUERIA_SESSION_COOKIE"
```

Ensure **worker** is running (`queria-worker`) so jobs leave `queued`. Watch `{BASE}/admin/jobs`.

### A4. Embeddings

Ingest creates chunks; embeddings are a separate durable job path (Voyage).

```bash
# from backend checkout with env loaded
cargo run -p queria-cli -- embeddings status --project fjulian-me
cargo run -p queria-cli -- embeddings backfill --project fjulian-me
# worker must be up; pace if Voyage 429 (see local-development runbook)
```

Ready counts in HANDOFF / dashboard. Retrieval quality needs non-zero **ready** embeddings; lexical-only fallback is degraded mode.

### A5. Issue an agent token

Tokens are bearer credentials for MCP (`Authorization: Bearer …`). Raw token is shown **once**.

**Reliable path (API):** full payload with scopes and tools.

```bash
curl -sS -X POST "$API/api/v1/agent-tokens" \
  -H 'Content-Type: application/json' \
  -H "Cookie: $QUERIA_SESSION_COOKIE" \
  -d '{
    "name": "droid-local",
    "project_slugs": ["fjulian-me"],
    "allow_global_knowledge": true,
    "tools": [
      "retrieve_context",
      "search_knowledge",
      "propose_memory",
      "index_memory",
      "list_projects",
      "get_source"
    ],
    "expires_in": "30_days"
  }'
```

Response includes `token` (raw, e.g. `qria_…`) and metadata. Store it only in a secret env var, never in git.

Notes:

- Default tools (if `tools` omitted) are **propose-only** write path: no `index_memory`. Include `index_memory` explicitly for scratch DX.
- `project_slugs` bound the token; agents only see those projects in `list_projects`.
- `allow_global_knowledge: true` is required for retrieve with global trusted knowledge.
- Admin UI `{BASE}/admin/tokens` can generate/list/revoke. Prefer the API above when you need explicit project slugs and `index_memory`. If UI create is description-only and fails validation, use the API.

### A6. Operator smoke (before agents)

1. `{BASE}/admin/playground` — query the project; expect citations when embeddings are ready. Toggles: rerank / compress (see hybrid runbook).
2. CLI:

```bash
cargo run -p queria-cli -- retrieval probe --project fjulian-me --query "how is content loaded" --limit 5
```

3. Optional golden eval (trusted path only):

```bash
cargo run -p queria-cli -- eval run --project fjulian-me
```

---

## Part B — Agent / MCP client

Prerequisite: Part A complete for at least one project with embeddings (or accept empty retrieve until backfill finishes).

### B1. Export the token

```bash
export QUERIA_AGENT_TOKEN='qria_…'   # paste once-from-create value
export QUERIA_MCP_URL='http://127.0.0.1:17674/mcp'   # local edge
# production example:
# export QUERIA_MCP_URL='http://168.110.214.130:17674/mcp'
```

Never commit the raw token. Prefer env injection over hardcoding in project configs.

### B2. Configure clients

All clients use **Streamable HTTP MCP** at `$QUERIA_MCP_URL` with bearer auth.

#### Factory Droid

If your Droid build supports remote HTTP MCP with headers:

```bash
# Pattern (adjust to your droid mcp CLI version):
droid mcp add queria "$QUERIA_MCP_URL" \
  --header "Authorization: Bearer ${QUERIA_AGENT_TOKEN}"
```

If only stdio MCP is supported, use a thin HTTP→stdio bridge only as fallback (do not run a second Queria storage). Prefer edge HTTP when available.

#### Claude Code

```bash
claude mcp add --transport http --scope user queria "$QUERIA_MCP_URL" \
  --header "Authorization: Bearer ${QUERIA_AGENT_TOKEN}"
```

Project `.mcp.json` with token from env (preferred for sharing):

```json
{
  "mcpServers": {
    "queria": {
      "type": "http",
      "url": "http://127.0.0.1:17674/mcp",
      "headersHelper": "printf '{\"Authorization\":\"Bearer %s\"}' \"$QUERIA_AGENT_TOKEN\"",
      "timeout": 60000
    }
  }
}
```

Older client notes under parent workspace `docs/mcp-clients/claude.md` may still mention port `67671` / `queria-proxy`. **Ignore those ports;** use `17674` edge paths above.

#### Codex

`~/.codex/config.toml` or project `.codex/config.toml`:

```toml
[mcp_servers.queria]
url = "http://127.0.0.1:17674/mcp"
bearer_token_env_var = "QUERIA_AGENT_TOKEN"
startup_timeout_sec = 20
tool_timeout_sec = 60
enabled = true
enabled_tools = [
  "retrieve_context",
  "search_knowledge",
  "propose_memory",
  "index_memory",
  "list_projects",
  "get_source"
]
```

### B3. Agent workflow (contract)

Before work:

```text
list_projects                          # discover allowed projects + UUIDs
retrieve_context(project_id, query)    # default include_scratch=true for agents
```

After work:

| Goal | Tool | Lane |
|---|---|---|
| Fast personal / session memory | `index_memory` | **scratch** (project only; needs token tool grant) |
| Candidate for team truth | `propose_memory` | **proposed** → human approval → **trusted** |
| Official codebase facts | (no agent write) | Git ingest (operator) |

Rules (see [`PRODUCT.md`](../PRODUCT.md)):

- Scratch is never global and never cross-project.
- Agents must not overwrite trusted via MCP.
- Near-dup ranking prefers **trusted** over **scratch**.
- Optional per-call: `rerank`, `compress` on retrieve/search (server defaults on).

`project_id` in `retrieve_context` / `search_knowledge` / `index_memory` is the project **UUID**.  
`propose_memory` uses `project_slug`.

### B4. Agent smoke checklist

| Step | Expect |
|---|---|
| MCP `initialize` + `tools/list` | Tools match token grants |
| `list_projects` | Only allowed slugs (e.g. `fjulian-me`) |
| `retrieve_context` with a real query | Citations when knowledge ready; empty is OK if embeddings still pending |
| `index_memory` (if granted) | Item searchable with `include_scratch=true` |
| `propose_memory` | Appears in Admin Approvals, not trusted until approve |

Rough JSON-RPC style smoke against edge (adjust auth):

```bash
curl -sS -X POST "$QUERIA_MCP_URL" \
  -H "Authorization: Bearer $QUERIA_AGENT_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}'
```

CLI doctor (if available in your build):

```bash
cargo run -p queria-cli -- doctor mcp --url "$QUERIA_MCP_URL"
```

---

## Dual-lane reminder (one diagram)

```text
                    ┌─────────────────────────────┐
  Git worker        │  trusted (approved)         │
  + approvals  ───► │  searchable always          │──► retrieve_context
                    └─────────────────────────────┘
  index_memory ───► ┌─────────────────────────────┐
                    │  scratch (project only)     │──► retrieve (include_scratch)
                    └─────────────────────────────┘
  propose_memory ► proposed → Admin approve → trusted
```

---

## Troubleshooting

| Symptom | Check |
|---|---|
| Edge not 200 | Compose services; HANDOFF stack identity; wrong port `67671` |
| Token 401 on MCP | Raw token copy, `Authorization: Bearer`, revoke list |
| Empty retrieve | Embeddings `ready` count; worker running; project_id UUID |
| `index_memory` missing | Token was created without that tool; re-issue with tools list |
| Voyage 429 | Embedding batch/interval env; see local-development |
| Agent sees wrong project | Token `project_slugs` |
| Client docs disagree | This runbook + HANDOFF win over stale mcp-clients ports |

---

## Part C — Agent-driven setup (LLM paste prompt)

Once the stack is up and an operator can issue tokens, an AI coding agent can finish **client-side** onboarding by following the live setup document.

### Operator: copy-paste this to the agent

```text
You are onboarding this workspace to QuerIa (central knowledge MCP).

1. GET http://127.0.0.1:17674/api/v1/docs/agent-setup (or production edge …:17674/api/v1/docs/setup) and follow it.
2. If I do not already have QUERIA_AGENT_TOKEN, ask me for it (only an admin can mint tokens).
3. GET …/api/v1/setup/mcp-snippet?client=<claude|codex|cursor|droid> and install the MCP config on THIS machine (do not expect the QuerIa server to write my ~/.config).
4. GET …/api/v1/setup/agents-block?project_slug=<slug> and merge into AGENTS.md using the <!-- queria:start --> markers idempotently.
5. Smoke: MCP list_projects + retrieve_context.

Use edge port 17674. Never 67671 / queria-proxy.
```

### Public endpoints (no auth)

| Method | Path |
|---|---|
| GET | `/api/v1/docs/agent-setup` |
| GET | `/api/v1/docs/setup` (alias) |
| GET | `/api/v1/setup/mcp-snippet?client=` |
| GET | `/api/v1/setup/agents-block?project_slug=` |

These ship in `queria-api`. Through Caddy they are available under the public edge base. Full operator path remains Part A–B above.

**Difference from enowx-rag:** QuerIa does **not** expose `install-mcp` that mutates config on the API host for remote agents. The LLM applies files locally after fetching snippets.

---

## Related docs

| Doc | Use |
|---|---|
| [`../HANDOFF.md`](../HANDOFF.md) | What is actually deployed |
| [`../PRODUCT.md`](../PRODUCT.md) | Lanes and tool contract |
| [`local-development.md`](./local-development.md) | Compose, migrate, backfill |
| [`hybrid-retrieval.md`](./hybrid-retrieval.md) | Rerank/compress/probe |
| [`deployment.md`](./deployment.md) | Production host |
| Parent `docs/mcp-clients/*` | Client templates (ports may lag; prefer URLs in this runbook) |
