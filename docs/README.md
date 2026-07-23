# Queria Backend Documentation

> Status: CURRENT
> Last verified: 2026-07-23
> Canonical repository: `https://github.com/nandocoeg2/queria-backend`

**Start here:** [`HANDOFF.md`](./HANDOFF.md) is the only source of truth for what is implemented.

## Read order

1. [`HANDOFF.md`](./HANDOFF.md) — current implementation, production host, residual gaps  
2. [`PRODUCT.md`](./PRODUCT.md) — product contract and post-cut boundaries  
3. [`ARCHITECTURE.md`](./ARCHITECTURE.md) — as-is vs post-hard-cut target  
4. [`SIMPLIFICATION.md`](./SIMPLIFICATION.md) — hard cut plan (Wave 2 residual from 2026-07-23 audit)  
5. [`IMPROVEMENTS.md`](./IMPROVEMENTS.md) — post-MVP improvement backlog (enowx-informed, REFERENCE)  
6. [`DOCS_POLICY.md`](./DOCS_POLICY.md) — status tags and update rules  
7. [`runbooks/`](./runbooks/) — local-dev, hybrid retrieval, deployment, rollback, backup-restore  

## Living documents

| Document | Status | Purpose |
|---|---|---|
| [`HANDOFF.md`](./HANDOFF.md) | `CURRENT` | Canonical current-state handoff |
| [`PRODUCT.md`](./PRODUCT.md) | `CURRENT` | Product contract |
| [`ARCHITECTURE.md`](./ARCHITECTURE.md) | `CURRENT` / planned target | As-is and post-cut architecture |
| [`SIMPLIFICATION.md`](./SIMPLIFICATION.md) | `CURRENT` | Executable cut list (Wave 1 done; Wave 2 residual) |
| [`IMPROVEMENTS.md`](./IMPROVEMENTS.md) | `REFERENCE` | Improvement backlog: dual-lane scratch + enowx quality/DX (not runtime truth) |
| [`DOCS_POLICY.md`](./DOCS_POLICY.md) | `CURRENT` | Doc ownership and archive rules |
| [`runbooks/local-development.md`](./runbooks/local-development.md) | `CURRENT` | Local infrastructure and commands |
| [`runbooks/onboarding.md`](./runbooks/onboarding.md) | `CURRENT` | **Default 3-step Daily path** (no CLI/Brew required); optional Admin Git / index-here; **queria-cli** via Releases today, Brew only after real formula SHAs |
| [`runbooks/queria-cli-homebrew.md`](./runbooks/queria-cli-homebrew.md) | `CURRENT` | Homebrew tap process; scaffold **odie** placeholder — installable only after generator writes real SHAs from live `cli-v*` assets |
| [`runbooks/agent-onboard-prompt.md`](./runbooks/agent-onboard-prompt.md) | `CURRENT` | One-paste client MCP after Daily mint; dialogs for missing fields (direnv optional) |
| [`runbooks/hybrid-retrieval.md`](./runbooks/hybrid-retrieval.md) | `PARTIAL` | Hybrid retrieval ops |
| [`runbooks/deployment.md`](./runbooks/deployment.md) | `CURRENT` | Production deploy (Path A/B); **CLI release is tag `cli-v*` only** (not push main; no auto-Brew); residual CLI/cache/Brew/Daily table |
| [`runbooks/rollback.md`](./runbooks/rollback.md) | `CURRENT` | Rollback |
| [`runbooks/backup-restore.md`](./runbooks/backup-restore.md) | `CURRENT` | Backup and restore |

## Archive

Historical plans, specs, and walkthroughs: [`archive/`](./archive/).

Do not execute archived plans as the active roadmap. Prefer HANDOFF residual gaps for ops acceptance, [`SIMPLIFICATION.md`](./SIMPLIFICATION.md) for complexity cuts, and [`IMPROVEMENTS.md`](./IMPROVEMENTS.md) for approved post-MVP product improvements.

## Parent workspace references

The parent workspace is not a Git repository. Product REFERENCE docs (research, UI flow, MCP client notes) live under workspace `docs/` and always defer status to this HANDOFF.
