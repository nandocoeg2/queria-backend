# Queria Backend Documentation

> Status: CURRENT
> Last verified: 2026-07-05
> Canonical repository: `https://github.com/nandocoeg2/queria-backend`

This index defines which Queria documents are authoritative. Start with
[`HANDOFF.md`](./HANDOFF.md), then follow the active implementation plan.

## Status Legend

| Status | Meaning |
|---|---|
| `CURRENT` | Matches the current implementation and operating model. |
| `PARTIAL` | Some documented behavior exists; listed gaps remain. |
| `COMPLETED` | The scoped plan has been implemented and verified. |
| `PLANNED` | Approved direction that has not been implemented. |
| `SUPERSEDED` | Kept for decision history; do not execute as the active plan. |
| `REFERENCE` | Design or research input, not an implementation-status claim. |

## Read Order

1. [`HANDOFF.md`](./HANDOFF.md) - current implementation, runtime state, known gaps, and continuation rules.
2. [`superpowers/plans/2026-07-05-queria-end-to-end.md`](./superpowers/plans/2026-07-05-queria-end-to-end.md) - active roadmap from retrieval hardening through production acceptance.
3. [`runbooks/local-development.md`](./runbooks/local-development.md) - local startup and verification commands.
4. [`runbooks/hybrid-retrieval.md`](./runbooks/hybrid-retrieval.md) - embedding, Qdrant, FTS, evaluation, and rate-limit operations.

## Document Inventory

| Document | Status | Purpose |
|---|---|---|
| [`HANDOFF.md`](./HANDOFF.md) | `CURRENT` | Canonical current-state handoff. |
| [`superpowers/plans/2026-07-05-queria-end-to-end.md`](./superpowers/plans/2026-07-05-queria-end-to-end.md) | `CURRENT` | Active execution order and acceptance gates. |
| [`runbooks/local-development.md`](./runbooks/local-development.md) | `CURRENT` | Local infrastructure and command reference. |
| [`runbooks/hybrid-retrieval.md`](./runbooks/hybrid-retrieval.md) | `PARTIAL` | Implemented hybrid retrieval; relaxed FTS and CLI evaluation persistence remain. |
| [`superpowers/specs/2026-07-04-hybrid-retrieval-design.md`](./superpowers/specs/2026-07-04-hybrid-retrieval-design.md) | `PARTIAL` | Implemented design with remaining reliability work. |
| [`superpowers/plans/2026-07-04-hybrid-retrieval.md`](./superpowers/plans/2026-07-04-hybrid-retrieval.md) | `PARTIAL` | Tasks 1-4 and 6 completed; lexical reliability and real backfill remain partial. |
| [`superpowers/plans/2026-07-04-git-ingestion-indexing-mvp.md`](./superpowers/plans/2026-07-04-git-ingestion-indexing-mvp.md) | `COMPLETED` | Git ingestion, parsing, stale cleanup, and trusted auto-approval MVP. |

## Product-Level References Outside This Repository

The parent workspace is not a Git repository. These documents are useful but
are not shipped when someone clones `queria-backend` alone:

- `../../README.md` - product overview.
- `../../../docs/centralized-team-knowledge-rag.md` - architecture and research decisions.
- `../../../docs/queria-mvp-implementation-spec.md` - detailed target-state specification.
- `../../../docs/queria-ui-mockup-flow.md` - approved UI flow and Stitch references.
- `../../../docs/mcp-clients/` - Codex and Claude MCP configuration.
- `../../../DESIGN.md` - Sahara visual design rules.

The current implementation state must always be taken from `HANDOFF.md`, not
from a target-state section in those product references.

## Update Rules

When behavior changes:

1. Update code and tests.
2. Update the relevant runbook.
3. Update `HANDOFF.md` current state and known gaps.
4. Mark the corresponding roadmap checkbox only after current verification.
5. Include the verification command and observed result in the commit message or handoff log.
