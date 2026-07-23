# Queria Simplification Plan

> Status: CURRENT  
> Last verified: 2026-07-23  
> Source audits: ponytail-audit 2026-07-16 (Wave 1, **done**); ponytail-audit 2026-07-23 (Wave 2, **active**)  
> Product: [`PRODUCT.md`](./PRODUCT.md) · Architecture: [`ARCHITECTURE.md`](./ARCHITECTURE.md) · Runtime: [`HANDOFF.md`](./HANDOFF.md)  
> Backlog freezes into this file; open product work stays in [`IMPROVEMENTS.md`](./IMPROVEMENTS.md)

**Executable cut list.** Code/disk cuts run only when someone explicitly executes a priority band. Do not re-add Wave 1 cuts (Three.js, Pingora, mockall, multi-store, eval Admin product).

---

## Wave 1 — complete (2026-07-16)

| Band | Result |
|---|---|
| P0 Admin lean | Three.js + shadcn/React islands gone; pure Astro |
| P1 Edge + structure | Caddy edge; `queria-proxy` deleted; observability/auth folded into core; dead db traits removed |
| P2 Defer + shrink | Eval Admin/HTTP removed (CLI eval kept); `restore_drill` out of lib; nested AppConfig; repos split |
| P3 enowx-rag | Qdrant-only; chroma/pgvector + OpenAI stubs gone |
| Closeouts | mockall fully removed; hand fakes only |

See git history / progress log at end for detail. **Do not re-litigate Wave 1.**

---

## Wave 2 — residual (2026-07-23)

Hard mode: free disk, collapse dual agent/CLI surfaces, cut god modules, freeze speculative backlog. Correctness/security out of scope.

### P0 — Disk and dead artifacts

| Tag | Cut | Replace with | Path | Acceptance |
|---|---|---|---|---|
| `delete` | Stale git worktrees (multi-GB) | `git worktree remove` after branch merge/drop | `queria/backend/.worktrees/*` (hub-tui, index-here, onboarding-friction, …) | `git worktree list` only keeps active branches; disk reclaimed |
| `delete` | Design leftovers not in runtime | Design vault or trash | parent `queria/shaders/`, `queria/queria-dashboard.pen` | Not referenced by admin build or compose |
| `delete` | Parent MVP spec duplicate (~3.2k LOC) if backend archive enough | Keep backend `docs/archive/` only | parent `docs/archive/queria-mvp-implementation-spec.md` | Parent archive inventory updated; no dual giant spec |

**Note:** parent paths sit outside `queria-backend` git. Cut them from the monorepo workspace only; do not invent commits outside backend without operator intent.

**Hands-off: `enowx-rag/`.** Do not delete, simplify, gitignore, or restructure the sibling tree (including its checked-in binary). Wave 1 Qdrant-only already applied; leave enowx alone unless the operator explicitly opts in.

### P1 — Dual surfaces (YAGNI)

| Tag | Cut | Replace with | Path | Acceptance |
|---|---|---|---|---|
| `yagni` | Thick agent HTTP + thick MCP doing the same work | **MCP primary** for tools; HTTP only for hooks that need REST (`retrieve-context`, `index-local`, setup snippets); shared service helpers, no second business logic copy | `crates/queria-api/src/http/agent_*.rs`, `crates/queria-mcp/` | Agent path e2e green; one shared retrieval/index helper; LOC on agent HTTP drops or is thin wrappers |
| `yagni` | Full hub TUI **and** full non-TUI command surface for same flows | **One** primary laptop UX: either `queria-cli tui` *or* subcommands, not both feature-complete forever. Prefer: TUI = thin wrappers over same fns as `index-here` / `doctor` / config file | `crates/queria-cli/src/{tui_hub,index_tui,config_tui,status_tui,doctor_tui,index_here}.rs` | Same acceptance via TUI **or** flags; no duplicated business rules; preferred path documented in onboarding |
| `yagni` | `restore_drill` as permanent CLI product surface (~390 LOC) | Runbook + one-off script, or keep behind `queria-cli backup restore-drill` only if ops runs it weekly | `crates/queria-cli/src/restore_drill.rs` | Core backup/restore path still documented; drill not required for default install |
### P2 — Code shrink

| Tag | Cut | Replace with | Path | Acceptance |
|---|---|---|---|---|
| `shrink` | God module `repositories/projects.rs` (~2.6k LOC) | Split by domain (projects / sources / knowledge / tokens / …) **only** if still growing; no new interface layer | `crates/queria-db/src/repositories/projects.rs` | `cargo test -p queria-db` green; files have one concern each |
| `shrink` | `retrieval.rs` (~1.6k) with in-file test fakes | Production path stays; move fakes to `tests/` or bottom-of-crate test module file | `crates/queria-search/src/retrieval.rs` | Tests pass; production file shorter |
| `shrink` | Multi-org HTTP bulk if prod still single-org | Keep schema + session binding; thin unused org admin routes only if no second customer | `crates/queria-api/src/http/{orgs,isolation,auth}.rs` | Isolation still enforced; no behavior regression for one-org deploy |
| `shrink` | `AppConfig` / ~88 env knobs still fat (~844 LOC) | Per-binary settings subsets or drop knobs no binary reads | `crates/queria-core/src/config.rs` | Each bin starts; dead env names removed from `.env.example` + runbooks |
| `yagni` | Single-impl `async_trait`s kept only for fakes | Concrete types + local test doubles where one real impl forever (Voyage, Qdrant, Pg hybrid) | `queria-search`, `queria-worker`, `queria-ingestion` | Tests green; fewer generic params on hot path if safe |
| `shrink` | `index_here` orchestration (~1k LOC) | Happy path first; nested multi-root edge cases stay gated by flags | `crates/queria-cli/src/index_here.rs` | Smoke script still passes; fewer branches for default cwd single-root |

### P3 — Docs hygiene

| Tag | Cut | Replace with | Path | Acceptance |
|---|---|---|---|---|
| `shrink` | `HANDOFF.md` archaeology + live state in one scroll | Split: short **Now** (≤~200 lines) + `HANDOFF-HISTORY.md` or dated append log | `docs/HANDOFF.md` | Agents open HANDOFF for current only; history linked |
| `yagni` | Speculative `IMPROVEMENTS` sprawl | Freeze open product list to **IMP-04, IMP-15, IMP-16** (+ explicit ops residuals). Mark rest `deferred` one-liners; no new IMP until usage | `docs/IMPROVEMENTS.md` | Top of IMPROVEMENTS lists only active proposed; rest table-deferred |
| `delete` | Dual-maintained parent thin mirrors that drift | Keep one-line pointers only; never status tables | parent `docs/*.md` | Parent README still points; no conflicting status |

### Prefer build over cut (not simplification debt)

These are **product** next, not cut targets—listed so Wave 2 does not starve them:

1. GHCR deploy green + host pull  
2. Multi-org image on prod if still local-only  
3. Embedding 429 residual hygiene  
4. Operator-green `e2e_agent_path_edge` / `e2e_index_here`  
5. **IMP-15 / IMP-16** (Admin scratch + promote) when dual-lane ops need them  

Do **not** default-build: IMP-06 stdio adapter, IMP-07 extra MCP tools, IMP-09–12 knobs, durable metrics (IMP-04) until logs hurt.

---

## Execution order (Wave 2)

1. **P0** disk/worktrees/binary/design artifacts (no cargo risk)  
2. **P1** choose CLI primary UX; thin second surface  
3. **P1** agent path: extract shared helpers; shrink HTTP wrappers  
4. **P2** move retrieval test fakes; trim config knobs not read  
5. **P2** split/shrink god modules only if still actively edited  
6. **P3** HANDOFF split + IMPROVEMENTS freeze  
7. Optional: defer/remove restore_drill surface  

Verify after each code band:

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
# if admin/docs only: skip cargo; still update HANDOFF residual gaps
```

After each band: update this progress log + HANDOFF residual gaps (one line).

---

## Out of scope

- **Sibling `enowx-rag/`** (any update, cleanup, binary delete, or docs touch) unless operator explicitly opts in
- Correctness / security review (separate pass)
- Replacing Voyage / Qdrant / Postgres
- Reintroducing Pingora, Three.js, shadcn, multi vector store, Evaluation Admin product
- Visual redesign beyond Violet Void
- Implementing any band without an explicit “execute P*” request

---

## Progress log

| Date | Band | Result |
|---|---|---|
| 2026-07-16 | Wave 1 (all) | See Wave 1 table; full detail in prior revisions of this file / git |
| 2026-07-23 | Wave 2 docs | Residual cut list shipped from ponytail-audit |
| 2026-07-23 | Wave 2 P1 agent | Shared `retrieve_for_agent` + agent list/status helpers; thin HTTP/MCP wrappers |
| 2026-07-23 | Wave 2 P1 CLI hub-primary | Onboarding/HANDOFF: `queria-cli tui` default laptop path; flags = automation; `doctor_mcp` → `edge_agent::mcp_tools_list`; top-level `config` = hub Config (`config_tui`) alias |
| 2026-07-23 | Wave 2 P1 CLI index-here core | Preserved `index-here --yes/--dry-run` + token/edge env; hub Index → `index_here` only (`upload_selected_plans`); single reqwest upload stack; cargo test -p queria-cli green; e2e_index_here blocked (no token/edge) |
| 2026-07-23 | Wave 2 P1 CLI restore_drill hide | `backup restore-drill` clap-hidden (still invocable for ops); not onboarding/install pitch; server ops `--help` still parse (database/embeddings/retrieval/eval/backup); runbook primary |
| 2026-07-23 | Wave 2 P2 retrieval tests move | Fat unit fakes/tests out of production `retrieval.rs` → `retrieval_tests.rs` (~1.3k LOC tests); production ~444 LOC. `retrieve_for_agent` shared entry unchanged (VAL-SHRINK-001/002) |
| 2026-07-23 | Wave 2 P2 projects façade split | Monolith `repositories/projects.rs` → `repositories/projects/` domain modules (crud/sources/tokens/…); same `PgProjectRepository` façade via `pub use`; no call-site renames; no new trait layer (VAL-SHRINK-003) |
| 2026-07-23 | Wave 2 P2 multi-org + config matrix | Org routes kept (`orgs::router`); fat HTTP tests moved `orgs.rs` → `orgs_tests.rs` (~790 LOC); `isolation.rs` already pure tests (unchanged). Env-by-binary ownership matrix in `.env.example` + pointer in `docs/runbooks/local-development.md` (VAL-SHRINK-004/005) |
| 2026-07-23 | Wave 2 P2 optional trait collapse | **Skipped (abort, no balloon):** search `async_trait`s (`HybridRetrievalStore`, `EmbeddingProvider`, `VectorIndex`, `EvaluationRetriever`) each still need prod + unit fakes (and worker/scratch doubles). Collapsing one would rewrite `RetrievalService<S,E,V>` + ~1.3k retrieval tests. VAL-SHRINK-006 N/A. |
| 2026-07-23 | Wave 2 P1–P2 code seal | **P1–P2 code bands landed on `feat/wave2-p1-p2-cuts`.** Structural: MCP+HTTP → one `retrieve_for_agent`; hub Index → `index_here::upload_selected_plans` only; workspace still **9** crates; **no** `enowx-rag` edits. Workspace fmt/clippy/test green earlier this band. Residual: live e2e when stack+token (may need API restart off branch binary); Wave 2 **P0 disk** + **P3 HANDOFF split** still out of this mission. |
| 2026-07-23 | Wave 2 cross e2e residual | **PASS** local: `scripts/e2e_agent_path_edge.py` (E0–E12) + `scripts/e2e_index_here_edge.py` (E0/I1–I6). Preconditions: compose postgres/qdrant/minio; host `queria-api`/`queria-mcp` rebuilt from `feat/wave2-p1-p2-cuts` (not stale `:17671`); edge `:17674` → host; `QUERIA_AGENT_TOKEN` smoke with `index_memory`+`index_local`; non-empty local `QDRANT_API_KEY` when Qdrant requires API key. No product code changes. |
