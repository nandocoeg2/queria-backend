# Queria Simplification Plan

> Status: CURRENT
> Last verified: 2026-07-16
> Source audit: ponytail-audit (over-engineering only)
> Product boundaries: [`PRODUCT.md`](./PRODUCT.md)
> Architecture: [`ARCHITECTURE.md`](./ARCHITECTURE.md)

Hard mode: medium cuts plus replace Pingora, collapse thin crates, defer evaluation product UI and restore-drill automation.

**This document is the executable cut list.** Docs-phase complete when living docs ship. Code cuts start only when someone explicitly executes a priority band.

## Priority bands

### P0 — Admin lean

| Tag | Cut | Replace with | Path | Acceptance |
|---|---|---|---|---|
| `delete` | Dead shadcn kit (`ui/button`, `ui/card`, `cn`, radix, cva, clsx, tailwind-merge, lucide) | Plain Astro markup + existing CSS tokens | `admin/src/components/ui`, `admin/src/lib/utils.ts`, `admin/package.json` | No unused deps in package.json; admin still builds |
| `delete` | Three.js knowledge graph (~760 LOC + three/r3f stack) | Dashboard stat cards only | `admin/src/components/ThreeCanvas.tsx`, `NodeCloud.tsx`, `EdgeLines.tsx`, `dashboard.astro` | Dashboard loads without WebGL island; Playwright smoke passes |

### P1 — Edge and structure

| Tag | Cut | Replace with | Path | Acceptance |
|---|---|---|---|---|
| `native` | `queria-proxy` + Pingora | Caddy or nginx path routing; compose exposes edge only | `crates/queria-proxy`, compose files, deployment runbook | Traffic via edge config; no `pingora` dep; health on public port |
| `yagni` | Thin `queria-observability` (16 LOC) | Module in core or shared bin helper | `crates/queria-observability` | Workspace builds; tracing still JSON |
| `yagni` | Optional fold `queria-auth` if still thin | core or db | `crates/queria-auth` | Auth tests green |
| `yagni` | mockall traits with single real impl | Concrete types; mockall only in `dev-dependencies` where needed | worker, search, ingestion, db | Tests still pass; fewer automock traits |
| `delete` | Dead `KnowledgeRepository` / `SourceRepository` traits | Methods on concrete repos only | `queria-db/src/repositories.rs` | No trait, no automock for unused interfaces |

### P2 — Defer product bloat and shrink

| Tag | Cut | Replace with | Path | Acceptance |
|---|---|---|---|---|
| `yagni` | Evaluation Admin product + heavy HTTP surface | CLI golden script only; mark deferred in HANDOFF | evaluation modules, admin evaluation page | CLI eval still runnable; HANDOFF says deferred |
| `yagni` | `restore_drill` as library feature | Script / CLI later | `queria-backup/src/restore_drill.rs` | Backup/restore runbook still valid for core backup |
| `shrink` | `AppConfig` ~600 LOC / ~53 fields | Per-binary env or split structs | `queria-core/src/config.rs` | Each bin still starts with its needed env |
| `shrink` | `repositories.rs` after dead-trait removal | Split by domain only if still huge | `queria-db/src/repositories.rs` | Compile; no new interface layer |

### P3 — enowx-rag (sibling tree)

| Tag | Cut | Replace with | Path | Acceptance |
|---|---|---|---|---|
| `yagni` | Chroma + pgvector providers | Qdrant-only | `enowx-rag/mcp-server/pkg/rag` | MCP starts with Qdrant |
| `delete` | OpenAI embedder env without implementation | Remove fields | `enowx-rag/mcp-server/cmd/mcp-server/main.go` | Config only lists voyage/tei |

## Execution order

1. P0 Admin lean  
2. P1 Edge (Caddy/nginx) + drop proxy crate  
3. P1 Trait prune + demote mockall  
4. P1 Crate collapse (observability, then auth if trivial)  
5. P2 Defer evaluation UI / restore_drill  
6. P2 Config shrink  
7. P3 enowx-rag alt stores  

Verify after each band:

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
# admin when UI touched:
# cd admin && npm run build
```

Then update HANDOFF completion matrix and residual gaps.

## Out of scope

- Correctness and security review
- Replacing Voyage / Qdrant / Postgres
- Sahara redesign
- Implementing cuts without an explicit execution request

## Progress log

| Date | Band | Result |
|---|---|---|
| 2026-07-16 | Docs pack | Living PRODUCT, ARCHITECTURE, SIMPLIFICATION, DOCS_POLICY shipped; plans archived |
| 2026-07-16 | P0 Admin lean | Removed Three.js graph + shadcn kit/react islands; `npm run build` green; 115 packages removed |
| 2026-07-16 | P1 Edge + structure | Caddy edge replaces Pingora (`queria-proxy` deleted); `queria-observability` folded into `queria-core`; dead db repository traits removed; `cargo test --workspace` green |
| 2026-07-16 | P2 Defer + shrink | Admin evaluation page removed (CLI eval kept); restore_drill marked CLI-only; dropped dead `proxy_addr` / `QUERIA_PROXY_ADDR` |
| 2026-07-16 | P3 enowx-rag | Qdrant-only; deleted chroma/pgvector + OpenAI stubs; `go mod tidy` dropped pgx; build green |
| 2026-07-16 | Closeouts | Removed eval HTTP routes; mockall demoted (cfg test / dev-deps) for worker+ingestion+most search; unused auth mockall removed; runbooks CLI-only eval |
