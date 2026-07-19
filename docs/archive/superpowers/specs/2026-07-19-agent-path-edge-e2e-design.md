# Design: Full agent-path E2E against production edge

> Status: REFERENCE  
> Last verified: 2026-07-19  
> Product contract: [`../../PRODUCT.md`](../../PRODUCT.md)  
> Runtime truth: [`../../HANDOFF.md`](../../HANDOFF.md)  
> Related hooks: [`2026-07-19-agent-auto-retrieve-hooks-design.md`](./2026-07-19-agent-auto-retrieve-hooks-design.md)  
> MCP probe reuse: [`../../../../scripts/mission_dl_pending_e2e.py`](../../../../scripts/mission_dl_pending_e2e.py)

## Problem

No single automated proof that the agent path works on the live edge: token → list projects → retrieve → index_memory → re-retrieve scratch → agent HTTP retrieve → hook script smoke. Partial coverage only: unit/isolation tests, `mission_dl_pending_e2e.py` (local MCP), CLI hybrid smoke, thin Admin Playwright.

## Locked decisions

| Knob | Choice |
|---|---|
| Scope | Full agent path: pre-minted token → MCP list/retrieve/index → agent HTTP → hook-script smoke |
| Environment | Production edge: `http://168.110.214.130:17674` (suite uses edge only, not direct 17672) |
| Auth | **Pre-minted smoke token only** (no `--mint` in v1) |
| Write safety | Scratch only, unique markers, dedicated project |
| Out of v1 | Admin mint/revoke, Git ingest, embed backfill, golden hit-rate, multi-org, backup, Playwright, real Droid/Claude GUI |

## Goals

1. One command proves agent path on live edge.
2. Failures name step IDs (E0…).
3. No secrets in git; never print raw token.
4. Scratch writes use unique markers only.

## Architecture

```text
Runner
  env: QUERIA_EDGE_URL, QUERIA_AGENT_TOKEN, QUERIA_SMOKE_PROJECT_SLUG
  → scripts/e2e_agent_path_edge.py
  → Edge :17674
      GET  /healthz
      GET  /api/v1/setup/hook-script
      GET  /api/v1/setup/hooks-snippet?client=droid
      GET  /api/v1/agent/projects          (Bearer)
      POST /api/v1/agent/retrieve-context  (Bearer)
      POST /mcp                            (Bearer; same MCP path as agents)
```

## Prod fixtures (operator, once)

Required outside git:

- **Project** slug: `queria-smoke` (or fixed slug in `QUERIA_SMOKE_PROJECT_SLUG`)
- **Token** tools: `list_projects`, `retrieve_context`, `search_knowledge`, **`index_memory`** (hard requirement; propose-only tokens fail the suite)
- **Secrets:** `QUERIA_EDGE_URL`, `QUERIA_AGENT_TOKEN` (password manager / CI; rotate on leak)

Optional: one small approved note so retrieve is non-empty; empty `items` still OK if structure asserts pass.

```bash
export QUERIA_EDGE_URL='http://168.110.214.130:17674'
export QUERIA_AGENT_TOKEN='qria_…'
export QUERIA_SMOKE_PROJECT_SLUG='queria-smoke'
```

Hardcoded suite behavior (no extra knobs): marker prefix `e2e-agent-`, retrieve-after-index retries **5 × 2s**, optional CLI `--skip-hooks` only.

## Entrypoint

```text
queria/backend/scripts/e2e_agent_path_edge.py
```

Reuse MCP JSON-RPC / SSE `data:` parsing from `mission_dl_pending_e2e.py`. Stdlib only (urllib, json). No bash-primary client.

```bash
python3 scripts/e2e_agent_path_edge.py --edge "$QUERIA_EDGE_URL"
python3 scripts/e2e_agent_path_edge.py --skip-hooks   # if bash/jq missing
```

Exit `0` if all hard steps pass; else non-zero with `E{n} FAIL: reason` on stderr (no token).

## Test cases

| ID | Step | Expect |
|---|---|---|
| E0 | `GET /healthz` | 200, body has `OK` |
| E1 | `GET /api/v1/setup/hook-script` | 200, starts with `#!/usr/bin/env bash` |
| E2 | `GET /api/v1/setup/hooks-snippet?client=droid` | 200 JSON; contains `SessionStart` and `UserPromptSubmit` |
| E3 | `GET /api/v1/agent/projects` no auth | 401, `agent_token_required` |
| E4 | `GET /api/v1/agent/projects` Bearer | 200; smoke slug present |
| E5 | `POST /api/v1/agent/retrieve-context` bad Bearer | 401 |
| E6 | `POST /api/v1/agent/retrieve-context` valid | 200; `items` array, `retrieval`, `project_id` |
| E7 | MCP `initialize` + `tools/list` | includes `retrieve_context`, `list_projects`, `index_memory` |
| E8 | MCP `list_projects` | smoke project present; no unexpected cross-token projects |
| E9 | MCP `retrieve_context` | success payload, not 401/403 |
| E10 | MCP `index_memory` with body marker `e2e-agent-{ts}-{uuid}` | success create or idempotent (**hard fail** if tool missing) |
| E11 | MCP `retrieve_context` `include_scratch=true`, query marker | marker visible within 5×2s retries (**hard**) |
| E12 | Hook script smoke (unless `--skip-hooks`) | write E1 script to temp, `chmod +x`; `bash -n` clean **or** UserPromptSubmit stdin `{"hook_event_name":"UserPromptSubmit","prompt":"ok"}` with env set → **exit 0** |

No E13–E15 in v1 (fail-open matrix, inject header assert, mint/revoke).

## MCP / agent HTTP details

- MCP: `POST {edge}/mcp`, headers `Authorization: Bearer`, `Content-Type: application/json`, `Accept: application/json, text/event-stream`; prefer `structuredContent`.
- Agent retrieve body:

```json
{
  "project_slug": "queria-smoke",
  "query": "…",
  "limit": 5,
  "include_scratch": true,
  "include_global": false
}
```

## Safety (prod)

1. Writes: only `index_memory` (scratch); marker prefix `e2e-agent-`.
2. Never: propose trusted, reindex, cancel jobs, delete sources, multi-org.
3. Token scoped to smoke project only.
4. Redact `Authorization` and `qria_` in logs.
5. Sequential single run.

## Report

```text
E0 PASS
…
E12 PASS
RESULT: PASS
```

## Docs (with implementation)

One paragraph in [`../../runbooks/onboarding.md`](../../runbooks/onboarding.md) (mint smoke token + run command). HANDOFF acceptance row when green once. No separate e2e runbook.

## Success criteria

```bash
export QUERIA_EDGE_URL=http://168.110.214.130:17674
export QUERIA_AGENT_TOKEN=…   # smoke token with index_memory
python3 scripts/e2e_agent_path_edge.py
# exit 0; E0–E12 pass (E12 skip only with --skip-hooks)
```

## Explicitly deferred

- `--mint` + admin session + revoke (add only if mint becomes a real operational need)
- `--json-out`, local compose CI job, Playwright mint UI, golden eval coupling

## Open note for implementer

Confirm live Admin token create/revoke paths only if mint is revived later. v1 needs no session cookie discovery.
