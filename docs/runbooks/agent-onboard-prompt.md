# Agent One-Shot Onboard (paste prompt + dialogs)

> Status: CURRENT  
> Last verified: 2026-07-20  
> Runtime truth: [`../HANDOFF.md`](../HANDOFF.md)  
> Full Admin path: [`onboarding.md`](./onboarding.md) (default 3-step Daily + Parts A–B)  
> Live client doc (edge): `GET {EDGE}/api/v1/docs/agent-setup`

Use this when a human wants a coding agent to finish **client-side** QuerIa setup on a machine/workspace with **one paste**, and any missing values collected via **question dialogs** (not guessing).

## What this covers / does not cover

| Agent can do | Human / Admin must still do |
|---|---|
| Health probe edge | Stack up on host |
| Export env (no commit of secrets) | Mint **Daily** `qria_…` agent token (connect panel) |
| Apply MCP snippet for client | Create project (Admin) |
| Merge `AGENTS.md` block | Optional: Admin Git ingest or laptop `index-here` + Promote for knowledge |
| Optional hooks (droid/claude) | Allowlist / SSH for private Git (if Admin Git path) |
| Smoke `list_projects` + `retrieve_context` | |
| Optional: mention / run `index-here` only if human asks and token has `index_local` | Promote **Needs review** in Admin (or privileged MCP grant) — Daily cannot promote |

If there is no token yet, the agent must **stop** after health and show the Admin checklist (see prompt). It cannot mint tokens.

**Default:** session or shell-profile env for token/edge/MCP. Do **not** require per-repo env files or direnv for Daily retrieve.

**Optional bulk local index:** if the human wants many local git roots indexed without cloud clone, point them at [`onboarding.md`](./onboarding.md) Part E (`queria-cli index-here` + Custom `index_local`). That path writes **Needs review** only; it does not make trusted knowledge until promote.

## Before you paste

1. Prefer edge health green: `https://queria.fjulian.id/healthz` or local `http://127.0.0.1:17674/healthz`.
2. Ideal: Admin already minted a **Daily** token (connect panel: tools include `list_projects`, `retrieve_context`, `search_knowledge`, `index_memory`, `propose_memory`, `get_source`).
3. Useful answers need ready chunks, but **connect works empty** (embeddings pending or no knowledge yet is OK for READY if MCP + list_projects work).
4. Know which client: `droid` | `claude` | `codex` | `cursor`.

You may leave token/slug/client empty in chat; the prompt requires the agent to **ask via dialog**.

## How to use

1. Open the coding agent in the target repo/workspace.
2. Copy the full block under **Canonical paste prompt** below.
3. Answer any structured questions (token, client, project pick, hooks).
4. Wait for **READY: yes/no**. If no, do the single next human action it names (often Admin mint Daily token).

**Multi-repo:** one multi-slug Daily token is enough. Default project selection is `list_projects` + UUID on each tool call. Set `QUERIA_PROJECT_SLUG` (or direnv per repo) **only if** auto-retrieve **hooks** need an active project across multi-root folders.

---

## Canonical paste prompt

Copy everything inside the fence:

```text
You are onboarding THIS workspace to QuerIa (centralized knowledge via MCP HTTP).

## Non-negotiable
- Prefer live edge docs over any local stale notes.
- Never use port 67671 or service name queria-proxy. Edge is Caddy; prefer https://queria.fjulian.id (local host port typically 17674).
- You CANNOT mint admin tokens, create org projects, register Git sources, or run Admin session APIs unless the human already logged you into an admin session (assume you are NOT admin).
- Never invent tokens, URLs, or project UUIDs.
- Never commit secrets (qria_… tokens). Do not write tokens into git-tracked files.
- Do not require per-repo .envrc / direnv for normal Daily setup. User-level or session env is enough.
- Full-repo index-here needs Custom grant index_local; Daily tokens do not include it. Never invent privileged tools.

## Default edge candidates (try health in this order, stop at first 200 OK)
1) https://queria.fjulian.id
2) http://127.0.0.1:17674
3) http://168.110.214.130:17674   # IP fallback only

## Goal
Machine ready for Daily work: health OK, 2–3 env vars set, MCP installed for the client, AGENTS.md merged when useful, optional hooks, smoke pass.
Connect works with empty retrieve; note when chunks are missing. Then report READY yes/no.

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
| EDGE base URL | prod HTTPS / local 17674 / OCI IP fallback / paste other |
| QUERIA_AGENT_TOKEN | “I’ll paste in chat” / “already in env” / “I don’t have one yet → STOP with admin steps” |
| CLIENT | droid / claude / codex / cursor |
| Multi-repo? | single project / multi-slug token + list_projects (default) / hooks need per-repo slug (advanced) |
| Hooks? | skip (default fine) / yes (droid/claude) / codex = AGENTS only |
| Persist env? | session-only / shell profile or secrets store / direnv only if hooks multi-repo |

If user says they have **no token**:
- STOP client install beyond health.
- Show short admin checklist: open Admin → Projects (if needed) → Tokens → **Daily agent** → copy connect panel → paste token back here.
  Example Admin bases: https://queria.fjulian.id/admin or http://127.0.0.1:17674/admin
- Offer to wait for next message with token. Do not require Git ingest before mint for connect-only readiness.

---

## Procedure (after required answers)

0) **Discover or confirm EDGE**
   - Probe candidates with: `curl -sS -o /tmp/queria-hz.out -w "%{http_code}" "$EDGE/healthz"`
   - Expect HTTP 200 and body roughly OK.
   - If all fail → DIALOG: which EDGE should we use? or is stack down?

1) **Token + env (once on this machine)**
   - If `QUERIA_AGENT_TOKEN` already in environment, confirm with DIALOG: reuse it? (don’t print full token).
   - Else DIALOG: paste token once (from Admin Daily connect panel when possible).
   - Export for this session (prefer session or shell profile — not a required tracked file):
     ```bash
     export QUERIA_EDGE_URL='…'
     export QUERIA_MCP_URL="${QUERIA_EDGE_URL}/mcp"
     export QUERIA_AGENT_TOKEN='…'   # never commit
     ```
   - Do **not** require `QUERIA_PROJECT_SLUG` unless hooks are installed (step 6).

2) **Live setup truth**
   - `GET ${QUERIA_EDGE_URL}/api/v1/docs/agent-setup` (alias `/api/v1/docs/setup`)
   - Follow that document for client-specific details.

3) **MCP on THIS machine**
   - `GET ${QUERIA_EDGE_URL}/api/v1/setup/mcp-snippet?client=<client>`
   - Apply snippet locally (server does not write the user’s home config).
   - DIALOG if client ambiguous.

4) **Project selection**
   - After MCP/Bearer works, call `list_projects` (MCP or `GET ${EDGE}/api/v1/agent/projects` with Bearer).
   - DIALOG if multiple: pick project (or free-text slug if listed).
   - Use **project UUID** for `retrieve_context` / `search_knowledge` / `index_memory`.
   - Multi-repo default: keep multi-slug token; pick project per tool call. Only if hooks will run multi-root: DIALOG about optional `QUERIA_PROJECT_SLUG` per folder (direnv is advanced, not required).

5) **AGENTS.md**
   - `GET ${QUERIA_EDGE_URL}/api/v1/setup/agents-block?project_slug=…`
   - Merge between `<!-- queria:start -->` and `<!-- queria:end -->` into this repo’s `AGENTS.md` (create if needed). Idempotent replace.

6) **Hooks** (only droid/factory/claude; skip codex unless user insists docs-only)
   - DIALOG: install auto-retrieve hooks? yes/skip (skip is fine for Daily)
   - If yes:
     - Export `QUERIA_PROJECT_SLUG` or `QUERIA_PROJECT_ID` for the active repo
     - `GET …/setup/hooks-snippet?client=…`
     - `GET …/setup/hook-script` → write e.g. `.factory/hooks/queria-retrieve-hook.sh`, `chmod +x`
     - Merge hooks JSON into client config
   - Hooks are soft inject; they do **not** replace deep `retrieve_context`.

7) **Smoke (required for READY=yes)**
   - tools/list (or client MCP list) with Bearer
   - `list_projects` → at least one project the human expects
   - `retrieve_context(project_id UUID, short real query)`
     - Hits OK if indexed
     - Empty OK if embeddings pending or no knowledge yet — still READY=yes for **connect**; say “useful answers need chunks (Admin Git or index-here + promote)”
   - Optional: tiny `index_memory` if tool granted and user allows (DIALOG yes/no)

8) **Final report (short)**
   - EDGE used, client, project_slug + project_id
   - env location (without printing token) — session / profile / optional direnv
   - MCP path, AGENTS.md, hooks yes/no
   - smoke results (empty vs hits)
   - **READY: yes/no** + single next human action if no

## Daily usage (after READY)
Before work: `list_projects` if needed, then `retrieve_context` for active project UUID.
After work: optional `index_memory` (scratch) or `propose_memory` (approval).
Cross-repo: second retrieve with other project_id — never assume one call covers whole workspace.
Full-repo ingest: not Daily — Admin Git or Custom + index-here (human path only unless asked + index_local).

Start now: probe health candidates; then open the first DIALOG for anything still missing.
```

---

## Short variant (experienced operators)

Only when EDGE, token, and client are already known and will be filled before paste:

```text
Onboard this workspace to QuerIa (Daily client path).

EDGE=<https://queria.fjulian.id or http://127.0.0.1:17674>
TOKEN=<qria_…>   # Daily connect panel
CLIENT=droid

1) GET $EDGE/healthz (must 200)
2) export QUERIA_AGENT_TOKEN / QUERIA_EDGE_URL / QUERIA_MCP_URL (session or shell profile; never commit token). QUERIA_PROJECT_SLUG only if hooks need it.
3) GET $EDGE/api/v1/docs/agent-setup and follow it
4) Apply GET $EDGE/api/v1/setup/mcp-snippet?client=$CLIENT on this machine
5) list_projects → pick slug; optional AGENTS.md from setup/agents-block
6) If droid/claude and user wants hooks: hooks-snippet + hook-script (ask first; set slug/id for hooks)
7) Smoke: list_projects + retrieve_context (empty OK if no chunks yet — note it)
8) Reply READY yes/no with paths + project UUID

No port 67671. Do not mint tokens. No required direnv. If anything missing, use a question dialog — do not guess.
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
| [`onboarding.md`](./onboarding.md) | Default 3-step Daily + optional knowledge paths |
| [`../HANDOFF.md`](../HANDOFF.md) | Deployed edge URLs and residual gaps |
| [`../PRODUCT.md`](../PRODUCT.md) | Tool contract, dual-lane |
| Backend README § agent multi-repo | Multi-slug token; slug env optional for hooks |
