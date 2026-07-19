# Design: Agent auto-retrieve hooks (hybrid Opsi C)

> Status: REFERENCE  
> Last verified: 2026-07-19  
> Implementation: plan `../plans/2026-07-19-agent-auto-retrieve-hooks.md`  
> Runtime truth: [`../../HANDOFF.md`](../../HANDOFF.md)

## Problem

Agents only received soft “retrieve before work” instructions via `AGENTS.md`. Need deterministic context inject on Droid + Claude without hard-blocking edits.

## Locked bundle

| Knob | Choice |
|------|--------|
| Trigger | T4 SessionStart + UserPromptSubmit |
| Throttle | R6 30s + query-hash + char/top-k cap + skip trivial |
| Hard-block | H1 soft inject + strong AGENTS only |
| Fail | open |
| Clients | Droid, Claude, AGENTS default (Codex soft only) |

## Design summary

Shell hooks cannot call Streamable HTTP MCP cleanly. Add thin agent-bearer retrieve:

```text
POST /api/v1/agent/retrieve-context
Authorization: Bearer qria_…
→ RetrievalPrincipal::Agent → hybrid pipeline → JSON citations
→ hook stdout / additionalContext
```

Optional:

```text
GET /api/v1/agent/projects
```

Setup:

```text
GET /api/v1/setup/hooks-snippet?client=droid|claude
```

## Non-goals v1

Hard Edit deny, auto index_memory on Stop, Codex native hooks, server-side install into ~/.factory.

## Security

Same authz as MCP retrieve_context. No token in committed configs. Fail-open. Quote shell vars.
