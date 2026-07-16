# Queria Product Contract

> Status: CURRENT
> Last verified: 2026-07-16
> Implementation ledger: [`HANDOFF.md`](./HANDOFF.md)

## North star

Centralize organization-wide and project-specific knowledge for humans and AI agents. Every agent should call `retrieve_context(project_id, query)` before work and may call `propose_memory` after work. Permanent memory enters normal retrieval only through approval or a trusted Git ingestion pipeline.

## Knowledge scopes

| Scope | Meaning |
|---|---|
| `global` | Coding, security, deployment, SOP, and operational standards shared across projects |
| `project` | Business flow, technical decisions, integrations, incidents, gotchas for one project |
| `include_global` | Request flag; still requires token permission. Project-only tokens cannot read global knowledge |

## Surfaces

| Surface | Audience | Role |
|---|---|---|
| Admin HTTP + Astro UI | Operators | Setup, projects, sources, approvals, tokens, audit, jobs |
| MCP (`queria-mcp`) | Agents | `retrieve_context`, `search_knowledge`, `propose_memory`, `list_projects`, `get_source` |
| CLI | Operators | Migrate, embeddings status, retrieval probe, eval (only eval path), backup/restore-drill |

Maintainer actions (approve/reject, reindex, token admin) stay on session Admin HTTP by design, not MCP.

## Post-cut product boundaries

After the hard simplification plan in [`SIMPLIFICATION.md`](./SIMPLIFICATION.md), the following are **out of MVP product surface** until product re-opens them:

- 3D knowledge graph on the dashboard (removed P0)
- Multi vector-store backends beyond Qdrant (enowx-rag Qdrant-only P3; Queria uses Voyage + Qdrant)
- Evaluation as a first-class Admin product (page removed P2; use CLI)
- Restore drill as product API (CLI/runbook only P2)
- Pingora-in-process edge (Caddy; P1)

## Sahara UI

Visual direction: workspace [`DESIGN.md`](../../../DESIGN.md). Warm minimalism; whitespace over chrome.
