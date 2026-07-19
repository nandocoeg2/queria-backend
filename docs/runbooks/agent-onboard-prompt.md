# Agent One-Shot Onboard (paste prompt + dialogs)

> Status: CURRENT  
> Last verified: 2026-07-19  
> Runtime truth: [`../HANDOFF.md`](../HANDOFF.md)  
> Full Admin path: [`onboarding.md`](./onboarding.md) Parts A–B  
> Live client doc (edge): `GET {EDGE}/api/v1/docs/agent-setup`

Use this when a human wants a coding agent to finish **client-side** QuerIa setup on a machine/workspace with **one paste**, and any missing values collected via **question dialogs** (not guessing).

## What this covers / does not cover

| Agent can do | Human / Admin must still do |
|---|---|
| Health probe edge | Stack up on host |
| Export env (no commit of secrets) | Mint `qria_…` agent token |
| Apply MCP snippet for client | Create project, Git register, ingest, embeddings |
| Merge `AGENTS.md` block | Allowlist / SSH for private Git (if new source) |
| Optional hooks (droid/claude) | |
| Smoke `list_projects` + `retrieve_context` | |

If there is no token yet, the agent must **stop** after health and show the Admin checklist (see prompt). It cannot mint tokens.

## Before you paste

1. Prefer edge health green: `https://queria.fjulian.id/healthz` or local `http://127.0.0.1:17674/healthz`.
2. Ideal: project already has knowledge / embeddings in progress.
3. Ideal: you already have a raw agent token (shown once at mint). Tools typically: `list_projects`, `retrieve_context`, `search_knowledge`, `index_memory`, `propose_memory`, `get_source`.
4. Know which client: `droid` | `claude` | `codex` | `cursor`.

You may leave token/slug/client empty in chat; the prompt requires the agent to **ask via dialog**.

## How to use

1. Open the coding agent in the target repo/workspace.
2. Copy the full block under **Canonical paste prompt** below.
3. Answer any structured questions (token, slug, client, multi-repo, hooks).
4. Wait for **READY: yes/no**. If no, do the single next human action it names (often Admin mint token or ingest).

Multi-repo: one multi-slug token is fine; set `QUERIA_PROJECT_SLUG` **per repo** (direnv). Do not use one global slug when hooks are enabled.

---

## Canonical paste prompt

Copy everything inside the fence:

```text
You are onboarding THIS workspace to QuerIa (centralized knowledge via MCP HTTP).

## Non-negotiable
- Prefer live edge docs over any local stale notes.
- Never use port 67671 or service name queria-proxy. Edge is Caddy; local/prod host port is typically 17674.
- You CANNOT mint admin tokens, create org projects, register Git sources, or run Admin session APIs unless the human already logged you into an admin session (assume you are NOT admin).
- Never invent tokens, URLs, or project UUIDs.
- Never commit secrets (qria_… tokens). Do not write tokens into git-tracked files.

## Default edge candidates (try health in this order, stop at first 200 OK)
1) https://queria.fjulian.id
2) http://127.0.0.1:17674
3) http://168.110.214.130:17674

## Goal
Machine ready for PROJECT work: health OK, env set, MCP installed for the client, AGENTS.md block merged, hooks installed when supported, smoke pass. Then report READY yes/no.

---

## Interaction rule — use QUESTION DIALOGS (mandatory)
Whenever something is missing, wrong, or multi-choice, **do not proceed by guessing**.

Use your structured user-question UI if available (e.g. AskUser / questionnaire):
- 1–4 short questions per round
- 2–4 options each when possible
- Always allow free-text / “Own answer” if options miss the case
- Show why the answer matters in the question text

If structured UI is unavailable, print a clear **DIALOG** block and wait:

```
DIALOG
Q1. …
  a) …
  b) …
  c) Own answer: ___
```

Ask in **batches** (prefer one round of needed fields), then continue. Do not spam one question per message if several are independent.

### Ask (at least) when any of these are unknown
| Field | Example options |
|---|---|
| EDGE base URL | prod HTTPS / local 17674 / OCI IP / paste other |
| QUERIA_AGENT_TOKEN | “I’ll paste in chat” / “already in env” / “I don’t have one yet → STOP with admin steps” |
| PROJECT_SLUG | list from list_projects after token works / human paste / create-later (not by agent) |
| CLIENT | droid / claude / codex / cursor |
| Multi-repo? | single project / multi-repo with direnv per folder |
| Hooks? | yes (droid/claude) / skip / codex = AGENTS only |
| Persist env? | direnv .envrc (no commit) / shell profile / session-only |

If user says they have **no token**:
- STOP client install beyond health.
- Show short admin checklist: open Admin → projects → sources ingest → tokens mint → paste token back here.
  Example Admin bases: https://queria.fjulian.id/admin or http://127.0.0.1:17674/admin
- Offer to wait for next message with token.

---

## Procedure (after required answers)

0) **Discover or confirm EDGE**
   - Probe candidates with: `curl -sS -o /tmp/queria-hz.out -w "%{http_code}" "$EDGE/healthz"`
   - Expect HTTP 200 and body roughly OK.
   - If all fail → DIALOG: which EDGE should we use? or is stack down?

1) **Token**
   - If `QUERIA_AGENT_TOKEN` already in environment, confirm with DIALOG: reuse it? (don’t print full token).
   - Else DIALOG: paste token once.
   - Export for this session:
     ```bash
     export QUERIA_EDGE_URL='…'
     export QUERIA_MCP_URL="${QUERIA_EDGE_URL}/mcp"
     export QUERIA_AGENT_TOKEN='…'   # never commit
     ```

2) **Live setup truth**
   - `GET ${QUERIA_EDGE_URL}/api/v1/docs/agent-setup` (alias `/api/v1/docs/setup`)
   - Follow that document for client-specific details.

3) **MCP on THIS machine**
   - `GET ${QUERIA_EDGE_URL}/api/v1/setup/mcp-snippet?client=<client>`
   - Apply snippet locally (server does not write the user’s home config).
   - DIALOG if client ambiguous.

4) **Project slug**
   - If unknown: after MCP/Bearer works, call `list_projects` (MCP or `GET ${EDGE}/api/v1/agent/projects` with Bearer).
   - DIALOG: pick project from list (or free-text slug if listed).
   - `export QUERIA_PROJECT_SLUG='…'`
   - Multi-repo: recommend direnv `.envrc` **per repo**; DIALOG before writing any env file; never one global slug for all folders if hooks will run.

5) **AGENTS.md**
   - `GET ${QUERIA_EDGE_URL}/api/v1/setup/agents-block?project_slug=…`
   - Merge between `<!-- queria:start -->` and `<!-- queria:end -->` into this repo’s `AGENTS.md` (create if needed). Idempotent replace.

6) **Hooks** (only droid/factory/claude; skip codex unless user insists docs-only)
   - DIALOG: install auto-retrieve hooks? yes/skip
   - If yes:
     - `GET …/setup/hooks-snippet?client=…`
     - `GET …/setup/hook-script` → write e.g. `.factory/hooks/queria-retrieve-hook.sh`, `chmod +x`
     - Merge hooks JSON into client config
   - Hooks are soft inject; they do **not** replace deep `retrieve_context`.

7) **Smoke (required for READY=yes)**
   - tools/list (or client MCP list) with Bearer
   - `list_projects` → slug present
   - `retrieve_context(project_id UUID, short real query)`
     - Hits OK if indexed
     - Empty OK only if embeddings pending — say so
   - Optional: tiny `index_memory` if tool granted and user allows (DIALOG yes/no)

8) **Final report (short)**
   - EDGE used, client, project_slug + project_id
   - env location (without printing token)
   - MCP path, AGENTS.md, hooks yes/no
   - smoke results
   - **READY: yes/no** + single next human action if no

## Daily usage (after READY)
Before work: `retrieve_context` for active project UUID.
After work: optional `index_memory` (scratch) or `propose_memory` (approval).
Cross-repo: second retrieve with other project_id — never assume one call covers whole workspace.

Start now: probe health candidates; then open the first DIALOG for anything still missing.
```

---

## Short variant (experienced operators)

Only when EDGE, token, slug, and client are already known and will be filled before paste:

```text
Onboard this workspace to QuerIa.

EDGE=<https://queria.fjulian.id or http://127.0.0.1:17674>
TOKEN=<qria_…>
SLUG=<project-slug>
CLIENT=droid

1) GET $EDGE/healthz (must 200)
2) export QUERIA_AGENT_TOKEN / QUERIA_EDGE_URL / QUERIA_MCP_URL / QUERIA_PROJECT_SLUG (direnv ok; never commit token)
3) GET $EDGE/api/v1/docs/agent-setup and follow it
4) Apply GET $EDGE/api/v1/setup/mcp-snippet?client=$CLIENT on this machine
5) Merge GET $EDGE/api/v1/setup/agents-block?project_slug=$SLUG into AGENTS.md (<!-- queria:start/end -->)
6) If droid/claude: install hooks from setup/hooks-snippet + setup/hook-script (ask first)
7) Smoke: list_projects + retrieve_context for $SLUG
8) Reply READY yes/no with paths + project UUID

No port 67671. Do not mint tokens. If anything missing, use a question dialog — do not guess.
```

---

## Public setup endpoints (no auth)

| Method | Path |
|---|---|
| GET | `/api/v1/docs/agent-setup` |
| GET | `/api/v1/docs/setup` (alias) |
| GET | `/api/v1/setup/mcp-snippet?client=` |
| GET | `/api/v1/setup/agents-block?project_slug=` |
| GET | `/api/v1/setup/hooks-snippet?client=` |
| GET | `/api/v1/setup/hook-script` |

Through edge: `{EDGE}/api/v1/...`. Details: [`onboarding.md`](./onboarding.md) Part C.

## Related

| Doc | Use |
|---|---|
| [`onboarding.md`](./onboarding.md) | Admin → agent full path |
| [`../HANDOFF.md`](../HANDOFF.md) | Deployed edge URLs and residual gaps |
| [`../PRODUCT.md`](../PRODUCT.md) | Tool contract, dual-lane |
| Backend README § agent multi-repo | One token, many repos, per-repo slug |
