# Queria Docs Policy

> Status: CURRENT
> Last verified: 2026-07-17

## Source of truth

| Question | Answer |
|---|---|
| What is implemented right now? | [`HANDOFF.md`](./HANDOFF.md) only |
| What should we delete or shrink next? | [`SIMPLIFICATION.md`](./SIMPLIFICATION.md) |
| How does the product work for operators? | Live files under [`runbooks/`](./runbooks/) |
| What is the product contract? | [`PRODUCT.md`](./PRODUCT.md) (includes dual-lane; HANDOFF says what is shipped) |
| As-is vs post-cut architecture? | [`ARCHITECTURE.md`](./ARCHITECTURE.md) |
| What post-MVP improvements are approved but not ledgers? | [`IMPROVEMENTS.md`](./IMPROVEMENTS.md) (`REFERENCE` backlog) |

If HANDOFF and any other document disagree, **HANDOFF wins**. Product specs and research docs never claim "done."

## Status tags

| Tag | Meaning |
|---|---|
| `CURRENT` | Matches implementation or current operating model |
| `PARTIAL` | Mix of done and not done; gaps must be listed |
| `REFERENCE` | Approved design or research; not an implementation ledger |
| `SUPERSEDED` | Historical; do not execute as the active plan |
| `ARCHIVE` | Moved out of the live path; retained for history only |

## Live tree (this repo)

```text
queria/backend/docs/
  HANDOFF.md
  README.md
  DOCS_POLICY.md
  PRODUCT.md
  ARCHITECTURE.md
  SIMPLIFICATION.md
  IMPROVEMENTS.md     # REFERENCE backlog (enowx-informed); not runtime truth
  runbooks/           # live ops only
  archive/            # SUPERSEDED plans, specs, walkthroughs
```

Parent workspace `docs/` (not a git repo) may hold product REFERENCE material and thin mirrors. It must not invent runtime status. See parent [`docs/README.md`](../../../docs/README.md).

## Update checklist

When behavior changes:

1. Code and tests
2. Relevant runbook (if ops changes)
3. `HANDOFF.md` current state and residual gaps
4. `SIMPLIFICATION.md` checkboxes only after verified cut
5. `IMPROVEMENTS.md` item status only after HANDOFF reflects the change (or when adding approved backlog)
6. Parent `docs/README.md` only if the product index needs a pointer change

## What goes to archive

- Completed or abandoned superpowers plans and design specs
- Phase walkthroughs once they are history
- Duplicate runbooks under parent `docs/` (canonical copies live here)
- Giant MVP target-state specs that no longer drive execution

Do not dual-maintain a plan in both live and archive. Move it, then leave a one-line pointer from the index if needed.

## Archive inventory (2026-07-16)

| Path | Former role |
|---|---|
| `archive/superpowers/plans/` | End-to-end and MVP execution plans |
| `archive/superpowers/specs/` | Hybrid retrieval design |
| `archive/walkthroughs/` | Phase 3–4 verification notes |

Parent workspace mirrors of the same material live under `docs/archive/`.
