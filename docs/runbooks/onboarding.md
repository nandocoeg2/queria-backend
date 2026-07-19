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

**Multi-org note:** every Admin session is bound to **one** home organization (`active_organization_id`). Projects, tokens, sources, knowledge, and retrieval stay inside that home. Creating a second tenant (Team B) is **Part D** (platform super-admin), not Part A.

### A1. First-run setup (once per empty install)

1. Open `{BASE}/admin/setup` (or `/admin/login` if setup already consumed).
2. Complete first-run with the setup token from env (`QUERIA_SETUP_TOKEN` when required).
3. Log in as admin (setup creates one org + membership; login binds that org as session home).

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

**Prefer Admin UI:** `{BASE}/admin/sources` → **Register Git Source** (title, uri, branch, optional `source_path`) → **Trigger Ingest** on the source row. List/detail and re-ingest also live on that page.

**API fallback** (automation / no browser session):

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

Then queue ingest:

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

**Prefer Admin UI:** `{BASE}/admin/tokens` → create form requires **name** + **project_slugs**. Optional: `allow_global_knowledge`, `expires_in`. List/revoke on the same page.

**API** when you need advanced tool lists or automation:

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

- Default tools (if `tools` omitted) are **propose-only** write path: no `index_memory`. Include `index_memory` explicitly for scratch DX (UI may not expose full tool list; use API for custom grants).
- `project_slugs` bound the token; agents only see those projects in `list_projects`.
- `allow_global_knowledge: true` is required for retrieve with global trusted knowledge.

### A6. Operator smoke (before agents)

Approvals use native HTML <dialog> confirm UI for approve/reject (SSR POST; not a custom modal framework).

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

### B5. Auto-retrieve hooks (Droid / Claude)

Hybrid inject (soft only, no Edit deny): **SessionStart** + throttled **UserPromptSubmit** shell hooks call agent-bearer HTTP retrieve and print condensed `## QuerIa context (auto)` into context. Fail-open if edge/token fails. Codex: AGENTS only.

```bash
export QUERIA_AGENT_TOKEN='qria_…'
export QUERIA_EDGE_URL='http://127.0.0.1:17674'   # or prod edge
export QUERIA_MCP_URL="${QUERIA_EDGE_URL}/mcp"
export QUERIA_PROJECT_SLUG='fjulian-me'            # or QUERIA_PROJECT_ID=<uuid>
# optional: QUERIA_HOOK_COOLDOWN_SEC=30 QUERIA_HOOK_MAX_CHARS=3500
```

```bash
# Snippet + script (public)
curl -sS "$QUERIA_EDGE_URL/api/v1/setup/hooks-snippet?client=droid"   # or client=claude
curl -sS "$QUERIA_EDGE_URL/api/v1/setup/hook-script" -o .factory/hooks/queria-retrieve-hook.sh
chmod +x .factory/hooks/queria-retrieve-hook.sh
# Merge returned JSON hooks into .factory/hooks.json (Droid) or Claude settings hooks key
```

Agent HTTP (same token as MCP):

| Method | Path |
|---|---|
| POST | `/api/v1/agent/retrieve-context` |
| GET | `/api/v1/agent/projects` |

Throttle defaults: 30s cooldown, same-query skip ~5m, skip trivial prompts (`ok`/`thanks`/…), cap ~3500 chars, top-k default 5. Script lives in-repo at `agent-tools/hooks/queria-retrieve-hook.sh`. Design: [`../archive/superpowers/specs/2026-07-19-agent-auto-retrieve-hooks-design.md`](../archive/superpowers/specs/2026-07-19-agent-auto-retrieve-hooks-design.md).

Hooks **do not** replace MCP `retrieve_context` for deep work (AGENTS.md still **MUST** call it).

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
| Cross-tenant data appears | Isolation bug: confirm session home org; multi-org migration applied; do not assume global operator |
| Create-org 403 | User is not platform super-admin — apply env/SQL in Part D0 |
| Invite accept “already in another org” | v1: one org per user; use a different email for Team B |

---

## Part D — Second organization (Team B) / multi-org isolation

Use this when a **second team** needs its own hard-isolated knowledge hub on the same QuerIa stack. Product rules and non-goals: [`../PRODUCT.md`](../PRODUCT.md) § Multi-organization tenancy. Runtime: [`../HANDOFF.md`](../HANDOFF.md) § Multi-org isolation MVP.

**Requires:** multi-org migration applied (`20260718000100_multi_org_tenancy` via `queria-cli database migrate`). Local `main` has this; production only after redeploy+migrate.

### D0. Flag a platform super-admin

One of:

```bash
# Env (restart API). Comma-separated, case-insensitive.
export QUERIA_PLATFORM_SUPER_ADMIN_EMAILS='nando@fjulian.id'
```

```sql
-- Or one-time SQL on Postgres
update user_account
set is_platform_super_admin = true
where lower(email) = lower('nando@fjulian.id');
```

Login as that user → `{BASE}/admin/orgs` should be available (Orgs nav only for super-admin). Super-admin **without** membership may create/list orgs but cannot open other tenants’ project/knowledge data until they themselves accept an invite into one org (v1: one membership max).

### D1. Create Team B + capture invite token once

**Admin UI:** `{BASE}/admin/orgs` → create form (`slug`, `name`, `first_admin_email`) → submit → **copy the raw invite token immediately**. Refresh/list will not show the secret again.

**API:**

```bash
curl -sS -X POST "$API/api/v1/orgs" \
  -H 'Content-Type: application/json' \
  -H "Cookie: $QUERIA_SESSION_COOKIE" \
  -d '{
    "slug": "team-b",
    "name": "Team B",
    "first_admin_email": "admin@teamb.example"
  }'
# Response includes invite_token (qinv_…) once + organization metadata
```

**No SMTP.** Deliver the token to the first admin out of band (chat, password manager, etc.). Invite rows store hash only.

Optional further members later: `POST $API/api/v1/orgs/team-b/invites` with `{ "email", "role" }` as org_admin of that org (or super-admin), or Admin `/admin/members` for the **home** org after login as Team B.

### D2. Accept invite (public; new Team B admin)

1. Open `{BASE}/admin/invites/accept` (optional `?token=` prefill).
2. Paste token + set password (≥12 chars) + name if new user.
3. Success redirects to login. Then log in as that email.

API equivalent:

```bash
curl -sS -X POST "$API/api/v1/invites/accept" \
  -H 'Content-Type: application/json' \
  -d '{
    "token": "qinv_…",
    "password": "correct horse battery staple",
    "name": "Team B Admin"
  }'
```

Accept rejects: expired/used/revoked tokens; users already in **another** org (v1 one-org-per-user).

### D3. Operate only inside Team B

As Team B admin (session home = Team B):

1. Continue **Part A2–A6** to create projects, register sources, issue agent tokens, smoke Playground — all data is Team B only.
2. Configure agents with **Part B** using a token minted under this session (token home = Team B).

Isolation checks:

| Check | Expect |
|---|---|
| Team A projects list | No Team B slugs |
| Team B projects list | No Team A slugs |
| Cross-org project GET | 404/403, no foreign body leak |
| Super-admin without membership | 403 on `/api/v1/projects` (not a global catalog) |

### D4. Explicit non-goals (do not wait for these)

- Outbound **email/SMTP** for invites  
- Cross-org **share grants**  
- **Per-org git** allowlist tables (instance env only)  
- Multi-membership **org switcher**  
- Super-admin silent browse of all tenants’ knowledge  

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
5. GET …/api/v1/setup/hooks-snippet?client=<droid|claude> and install SessionStart + UserPromptSubmit auto-retrieve hooks (write script, merge hooks JSON). Codex: skip hooks, AGENTS only.
6. Smoke: MCP list_projects + retrieve_context; optional new session shows QuerIa auto context inject.

Use edge port 17674. Never 67671 / queria-proxy.
```

### Public endpoints (no auth)

| Method | Path |
|---|---|
| GET | `/api/v1/docs/agent-setup` |
| GET | `/api/v1/docs/setup` (alias) |
| GET | `/api/v1/setup/mcp-snippet?client=` |
| GET | `/api/v1/setup/agents-block?project_slug=` |
| GET | `/api/v1/setup/hooks-snippet?client=` |
| GET | `/api/v1/setup/hook-script` |

These ship in `queria-api`. Through Caddy they are available under the public edge base. Full operator path remains Part A–B above.

**Difference from enowx-rag:** QuerIa does **not** expose `install-mcp` that mutates config on the API host for remote agents. The LLM applies files locally after fetching snippets.

---

## Related docs

| Doc | Use |
|---|---|
| [`../HANDOFF.md`](../HANDOFF.md) | What is actually deployed; multi-org bootstrap + isolation smoke |
| [`../PRODUCT.md`](../PRODUCT.md) | Lanes, tool contract, multi-org v1 non-goals |
| [`local-development.md`](./local-development.md) | Compose, migrate, backfill |
| [`hybrid-retrieval.md`](./hybrid-retrieval.md) | Rerank/compress/probe |
| [`deployment.md`](./deployment.md) | Production host |
| Parent `docs/mcp-clients/*` | Client templates (ports may lag; prefer URLs in this runbook) |
