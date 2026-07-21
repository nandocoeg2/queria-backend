# Onboarding Runbook (Admin → Agent)

> Status: CURRENT  
> Last verified: 2026-07-20  
> Runtime truth: [`../HANDOFF.md`](../HANDOFF.md)  
> Local infra detail: [`local-development.md`](./local-development.md)  
> Retrieval ops: [`hybrid-retrieval.md`](./hybrid-retrieval.md)

Default path: mint a **Daily** agent token, connect the client once, then retrieve.
Knowledge ingest (Admin Git or laptop `index-here`) is optional and separate.

```text
Default (Daily agent)
  Admin: create project → mint Daily token → copy connect panel
  User (once): env + MCP
  Work: list_projects → retrieve_context → index_memory / propose_memory

Optional knowledge
  Admin Git register + ingest/embed
  or Custom + index_local → queria-cli index-here → Promote Needs review
```

## Default: 3 steps for Daily users

1. **Admin mints Daily** — Admin → Tokens → **Daily agent (recommended)** → select project(s) → generate. Copy values from the once-only **Connect this agent** panel (raw token + env export). Daily includes `list_projects`, `retrieve_context`, `search_knowledge`, `propose_memory`, `get_source`, `index_memory`. It does **not** include `index_local` or `manage_needs_review`.
2. **User sets env once + MCP** — On the laptop (session or shell profile / secrets store; no required per-repo file):

   ```bash
   export QUERIA_AGENT_TOKEN='qria_…'          # from connect panel; never commit
   export QUERIA_EDGE_URL='https://queria.fjulian.id'   # or local edge
   export QUERIA_MCP_URL="${QUERIA_EDGE_URL}/mcp"
   ```

   Install HTTP MCP with Bearer from that env (`GET $QUERIA_EDGE_URL/api/v1/setup/mcp-snippet?client=…`). Optional: `QUERIA_PROJECT_SLUG` only if auto-retrieve **hooks** need an active project.
3. **Work** — Agent: `list_projects` → `retrieve_context(project_id, query)`. Optional: `index_memory` (scratch) / `propose_memory` (approval).

**Connect works empty; useful when chunks are ready.** MCP + `list_projects` can succeed with zero embeddings. Useful answers need trusted knowledge (Admin Git ingest/embed, or index-here then Promote). Empty retrieve is not a client-setup failure.

Client one-paste (dialogs for missing fields): [`agent-onboard-prompt.md`](./agent-onboard-prompt.md). Live client doc: `GET {EDGE}/api/v1/docs/agent-setup`.

## Edge URLs (do not use stale ports)

Public path routing is **Caddy** (`queria-edge`). There is **no** `queria-proxy` / Pingora path and **no** port `67671`.

| Environment | Base URL | Admin | MCP | Health |
|---|---|---|---|---|
| **Production (primary)** | `https://queria.fjulian.id` | `/admin` | `/mcp` | `/healthz` |
| Local edge | `http://127.0.0.1:17674` | `/admin` | `/mcp` | `/healthz` |
| Direct local services (no edge) | API `http://127.0.0.1:17671`, MCP `http://127.0.0.1:17672` | Admin SSR often `:4321` | MCP service | API `/healthz` if exposed |
| Production host IP (fallback) | `http://168.110.214.130:17674` | `/admin` | `/mcp` | `/healthz` |

Prefer the **public hostname** for agents and browsers so path routing and TLS match production.

Production **must** set `QUERIA_PUBLIC_BASE_URL=https://queria.fjulian.id` so agent-setup markdown and MCP snippet absolute URLs use the public edge (not the internal Host). Local: leave default `http://127.0.0.1:17674` or unset to use headers.

```bash
curl -sS -o /tmp/queria-health.out -w "%{http_code}\n" https://queria.fjulian.id/healthz
# expect 200 and body OK (local: http://127.0.0.1:17674/healthz)
```

If health fails, stack is not ready. Fix infra first ([`local-development.md`](./local-development.md) or [`deployment.md`](./deployment.md)). Do not onboard agents against a dead edge.

---

## Optional knowledge ingest

Knowledge is **not** part of the Daily 3-step connect path. Choose one (or both later):

| Path | Who | When |
|---|---|---|
| **Admin Git** | Operator with Admin session | Server can clone the remote (allowlist + SSH if private) — Part A3 |
| **Laptop index-here** | Dev on the machine with the git clone; token with **`index_local`** (Custom mint) | Self-hosted / unreachable remotes; land in Needs review until Promote — Part E |

### Install `queria-cli` (laptop)

Users should **not** need a Rust toolchain for `index-here`.

**Preferred (when Homebrew formula is published):**

```bash
# After live Release + homebrew-queria formula push — see queria-cli-homebrew.md
brew install nandocoeg2/queria/queria-cli
# private Release assets: export HOMEBREW_GITHUB_API_TOKEN=… first
queria-cli index-here --help
```

Tap process / generator: [`queria-cli-homebrew.md`](./queria-cli-homebrew.md). Workspace scaffold: `queria/homebrew-queria/` → GitHub repo `nandocoeg2/homebrew-queria`.

**Fallback — GitHub Release binaries (curl):**

| Platform | Asset (latest `cli-v*` release) |
|---|---|
| macOS Apple Silicon | `queria-cli-aarch64-apple-darwin.tar.gz` |
| macOS Intel | `queria-cli-x86_64-apple-darwin.tar.gz` |
| Linux x86_64 | `queria-cli-x86_64-unknown-linux-gnu.tar.gz` |
| Linux arm64 | `queria-cli-aarch64-unknown-linux-gnu.tar.gz` |

Releases: https://github.com/nandocoeg2/queria-backend/releases  

```bash
# Example: macOS Apple Silicon (replace TAG with latest cli-v*)
TAG=cli-v0.1.0
curl -fsSL -o queria-cli.tar.gz \
  "https://github.com/nandocoeg2/queria-backend/releases/download/${TAG}/queria-cli-aarch64-apple-darwin.tar.gz"
tar -xzf queria-cli.tar.gz
sudo install -m 755 queria-cli-aarch64-apple-darwin/queria-cli /usr/local/bin/queria-cli
queria-cli index-here --help
```

**Maintainers (do not forget):**

1. Push **`main` does not publish CLI binaries.** Tag **`cli-v*`** (or Actions → **Release queria-cli**) via [`.github/workflows/release-cli.yml`](../../.github/workflows/release-cli.yml).  
2. After Release assets are live: `./scripts/generate_homebrew_formula.sh cli-vX.Y.Z` → commit/push **homebrew-queria**.  
3. Host deploy Path A is separate ([`deployment.md`](./deployment.md)).

Dev alternative: `cargo build -p queria-cli --release` in this repo.

### Fast first knowledge (laptop)

For a laptop clone without Admin Git registration:

1. Install `queria-cli` from GitHub Releases (above).
2. Create project (Admin → Projects) if missing.
3. Mint **Custom** token with `index_local` checked (warning: uploads land in **Needs review only**).
4. From the repo (or monorepo root):

   ```bash
   export QUERIA_AGENT_TOKEN='…'   # Custom token with index_local
   export QUERIA_EDGE_URL='https://queria.fjulian.id'   # or local edge
   queria-cli index-here --token-env QUERIA_AGENT_TOKEN
   ```

5. Admin → Needs review → **Promote** (trusted path).
6. Use a **Daily** token for normal retrieve + `index_memory` scratch (do not give Daily users `index_local`).

Full contract: [Part E — Local multi-git `index-here`](#part-e--local-multi-git-index-here-needs-review). No demo corpus seed. Dual-lane (trusted vs Needs review) unchanged.

---

## Part A — Operator / Admin

Use a human session cookie via Admin UI (or authenticated Admin HTTP). Agents never perform these steps.

**Multi-org note:** every Admin session is bound to **one** home organization (`active_organization_id`). Projects, tokens, sources, knowledge, and retrieval stay inside that home. Creating a second tenant (Team B) is **Part D** (platform super-admin), not Part A.

### A1. First-run setup (once per empty install)

1. Open `{BASE}/admin/setup` (or `/admin/login` if setup already consumed).
2. Complete first-run with the setup token from env (`QUERIA_SETUP_TOKEN` when required).
3. Log in as admin (setup creates one org + membership; login binds that org as session home).

Setup does **not** create a project; continue with **A2** (Create project).

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

Tokens are bearer credentials for MCP (`Authorization: Bearer …`). Raw token is shown **once** in the connect panel.

**Prefer Admin UI:** `{BASE}/admin/tokens` → **Daily agent (recommended)** for normal retrieve/scratch, or **Custom** for privileged tools. Form requires **name** + **project_slugs**. Optional: `allow_global_knowledge`, `expires_in` (default no expiry). After mint, copy **token + env export** from **Connect this agent** (once-only).

| Mode | Tools | Use |
|---|---|---|
| **Daily** (default) | `list_projects`, `retrieve_context`, `search_knowledge`, `propose_memory`, `get_source`, `index_memory` | Everyday agent work |
| **Custom** | Checkbox list; `index_local` / `manage_needs_review` default off with warnings | Laptop `index-here` or Needs-review promote via MCP |

**API** when you need automation (Admin UI always POSTs explicit `tools`; omit-`tools` on API stays propose-only without `index_memory`):

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
    "expires_in": "no_expire"
  }'
```

Response includes `token` (raw, e.g. `qria_…`) and metadata. Store it only in a secret env var, never in git.

Notes:

- API omit `tools` → **propose-only** (no `index_memory`). Daily path = Admin Daily mode or explicit tools list including `index_memory`.
- `project_slugs` bound the token; agents only see those projects in `list_projects`. Multi-slug tokens are fine; pick project via `list_projects` (no required per-repo env).
- `allow_global_knowledge: true` is required for retrieve with global trusted knowledge.
- Full-repo ingest is **not** a Daily grant: use Custom + `index_local` or Admin Git (Optional knowledge ingest above).

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

Same as **Default: 3 steps** above. Prerequisite: a Daily (or equivalent) token exists. Project may have zero embeddings yet — connect still works; answers improve when chunks are ready.

### B1. Export the token

Prefer the connect panel **Copy env** after mint. Manual equivalent:

```bash
export QUERIA_AGENT_TOKEN='qria_…'          # paste once-from-create value; never commit
export QUERIA_EDGE_URL='https://queria.fjulian.id'   # or http://127.0.0.1:17674
export QUERIA_MCP_URL="${QUERIA_EDGE_URL}/mcp"
# optional, hooks only:
# export QUERIA_PROJECT_SLUG='my-project'
```

User-level session or shell profile is enough. Do **not** require a per-repo env file for Daily retrieve.

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

Parent templates: [`docs/mcp-clients/`](../../../../docs/mcp-clients/) (edge `:17674`). Prefer live `GET …/setup/mcp-snippet` when available.

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

### B6. Agent-path edge E2E (prod smoke)

Pre-minted smoke token only (no auto-mint). Dedicated project recommended (`queria-smoke`) with tools: `list_projects`, `retrieve_context`, `search_knowledge`, `index_memory`.

```bash
export QUERIA_EDGE_URL='http://168.110.214.130:17674'
export QUERIA_AGENT_TOKEN='qria_…'          # smoke token, never commit
export QUERIA_SMOKE_PROJECT_SLUG='queria-smoke'

# from queria/backend checkout
python3 scripts/e2e_agent_path_edge.py
# optional: python3 scripts/e2e_agent_path_edge.py --skip-hooks
```

Expect `E0`…`E12 PASS` and `RESULT: PASS`. Design: [`../archive/superpowers/specs/2026-07-19-agent-path-edge-e2e-design.md`](../archive/superpowers/specs/2026-07-19-agent-path-edge-e2e-design.md).

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

Client-side onboard (MCP, env, AGENTS.md, hooks, smoke) with **one paste** and **question dialogs** for missing token/slug/client:

→ **[`agent-onboard-prompt.md`](./agent-onboard-prompt.md)** (canonical prompt + short variant + setup endpoint list)

Operator still does Part A (project, Git ingest, mint token) before the agent can finish Part C. Live doc on edge: `GET {BASE}/api/v1/docs/agent-setup`. QuerIa does not write the agent machine’s MCP config from the API host.

---

## Part E — Local multi-git `index-here` (Needs review)

Use when the code lives on **this machine** (laptop / self-hosted remotes the OCI worker cannot clone). No per-repo Admin Git form. Uploads land as **Needs review**, not trusted.

### E1. Token with `index_local`

Mint an agent token that includes permission **`index_local`** (API tool list / grants). Default mint is propose-only and does **not** include index-local. Store raw token once:

```bash
export QUERIA_AGENT_TOKEN='qria_…'
export QUERIA_EDGE_URL='https://queria.fjulian.id'   # or http://127.0.0.1:17674
```

Never commit the token. Edge base only — path is `/api/v1/agent/index-local` under that host.

### E2. One-command index

From the workspace root (nested git roots discovered up to depth 4):

```bash
export QUERIA_AGENT_TOKEN=…
export QUERIA_EDGE_URL=https://queria.fjulian.id   # or http://127.0.0.1:17674
queria-cli index-here --token-env QUERIA_AGENT_TOKEN --yes
```

Dry-run: `queria-cli index-here --token-env QUERIA_AGENT_TOKEN --dry-run --yes`. Expect `job_ids` + per-root accepted/skipped counts on real upload. Worker must be up so embed jobs leave `queued`. Nested git roots: parent does **not** upload paths that live under another discovered nested root (same run).

Edge smoke (optional): mint token with `index_local` (+ retrieve), then:

```bash
export QUERIA_EDGE_URL=http://127.0.0.1:17674   # or https://queria.fjulian.id
export QUERIA_AGENT_TOKEN=qria_…
# optional promote: export QUERIA_PROMOTE_TOKEN=…  # manage_needs_review
cargo build -p queria-cli
python3 scripts/e2e_index_here_edge.py
# RESULT: PASS (or PASS with promote skipped)
```

### E3. Review and promote

1. Admin: `{BASE}/admin/needs-review` — groups by project / origin / commit.
2. **Promote** → approved (trusted retrieve path). **Reject** removes from queue.
3. Optional privileged MCP (explicit grant **`manage_needs_review`**, not default mint): `list_needs_review`, `promote_knowledge`, `reject_needs_review`.

Default `retrieve_context` **excludes** Needs review. Opt-in: `include_needs_review=true` (org members with project access). Playground / CLI probe same flag.

### E4. Non-goals for this path

- No auto-promote to trusted (IMP-L6 deferred)
- No browser full-tree file picker
- No `QUERIA_GIT_ALLOWED_ROOTS` requirement for index-here (server re-validates gates; does not clone remotes)
- Does **not** replace allowlisted Git cloud ingest (Part A3) for remotes the worker can reach

Copy-paste block also lives on Admin **Needs review** page.

---

## Related docs

| Doc | Use |
|---|---|
| [`../HANDOFF.md`](../HANDOFF.md) | What is actually deployed; multi-org bootstrap + isolation smoke; index-here residual |
| [`../PRODUCT.md`](../PRODUCT.md) | Lanes (incl. Needs review), tool contract, multi-org v1 non-goals |
| [`agent-onboard-prompt.md`](./agent-onboard-prompt.md) | One-paste client onboard + dialog questions |
| [`local-development.md`](./local-development.md) | Compose, migrate, backfill |
| [`hybrid-retrieval.md`](./hybrid-retrieval.md) | Rerank/compress/probe |
| [`deployment.md`](./deployment.md) | Production host |
| Parent `docs/mcp-clients/*` | Thin Claude/Codex templates (edge `:17674`) |
