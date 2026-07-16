# Queria Backend Documentation

> Status: CURRENT
> Last verified: 2026-07-16
> Canonical repository: `https://github.com/nandocoeg2/queria-backend`

**Start here:** [`HANDOFF.md`](./HANDOFF.md) is the only source of truth for what is implemented.

## Read order

1. [`HANDOFF.md`](./HANDOFF.md) — current implementation, production host, residual gaps  
2. [`PRODUCT.md`](./PRODUCT.md) — product contract and post-cut boundaries  
3. [`ARCHITECTURE.md`](./ARCHITECTURE.md) — as-is vs post-hard-cut target  
4. [`SIMPLIFICATION.md`](./SIMPLIFICATION.md) — ranked hard cut plan from ponytail-audit  
5. [`DOCS_POLICY.md`](./DOCS_POLICY.md) — status tags and update rules  
6. [`runbooks/`](./runbooks/) — local-dev, hybrid retrieval, deployment, rollback, backup-restore  

## Living documents

| Document | Status | Purpose |
|---|---|---|
| [`HANDOFF.md`](./HANDOFF.md) | `CURRENT` | Canonical current-state handoff |
| [`PRODUCT.md`](./PRODUCT.md) | `CURRENT` | Product contract |
| [`ARCHITECTURE.md`](./ARCHITECTURE.md) | `CURRENT` / planned target | As-is and post-cut architecture |
| [`SIMPLIFICATION.md`](./SIMPLIFICATION.md) | `CURRENT` | Executable over-engineering cut list |
| [`DOCS_POLICY.md`](./DOCS_POLICY.md) | `CURRENT` | Doc ownership and archive rules |
| [`runbooks/local-development.md`](./runbooks/local-development.md) | `CURRENT` | Local infrastructure and commands |
| [`runbooks/hybrid-retrieval.md`](./runbooks/hybrid-retrieval.md) | `PARTIAL` | Hybrid retrieval ops |
| [`runbooks/deployment.md`](./runbooks/deployment.md) | `CURRENT` | Production deploy |
| [`runbooks/rollback.md`](./runbooks/rollback.md) | `CURRENT` | Rollback |
| [`runbooks/backup-restore.md`](./runbooks/backup-restore.md) | `CURRENT` | Backup and restore |

## Archive

Historical plans, specs, and walkthroughs: [`archive/`](./archive/).

Do not execute archived plans as the active roadmap. Prefer [`SIMPLIFICATION.md`](./SIMPLIFICATION.md) for next engineering cuts, and HANDOFF residual gaps for ops acceptance.

## Parent workspace references

The parent workspace is not a Git repository. Product REFERENCE docs (research, UI flow, MCP client notes) live under workspace `docs/` and always defer status to this HANDOFF.
