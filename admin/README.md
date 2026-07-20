# Queria Admin

Astro 7 SSR BFF console for Queria (`base: /admin`). Pure Astro — no React islands. Proxies authenticated requests to `queria-api`.

## Theme — Violet Void

Dark Centered Platform tokens:

| Token | Value |
|-------|-------|
| Ground | `#0A0A0A` |
| Cards | `#111111` |
| Primary accent | `#582CFF` |
| Fonts | Inter / Geist / Funnel Sans |

CSS custom properties live in [`src/styles/tokens.css`](./src/styles/tokens.css).

**Design references** (outside this package):

- Repo root [`DESIGN.md`](../../../DESIGN.md) — approved visual direction
- [`queria/queria-dashboard.pen`](../../queria-dashboard.pen) — product UI reference

## Prerequisites

- Node `>=22.12`
- Running `queria-api` and local Docker infra (Postgres, Qdrant)

Local stack (from `queria/backend`):

```sh
# API :17671, Postgres :17675, Qdrant :17676 — see docker-compose.yml / docs/runbooks/local-development.md
```

Admin reads API via `QUERIA_API_URL` (default `http://localhost:17671`). No secrets belong in this README; use backend env/compose docs for credentials.

## Setup

```sh
cd queria/backend/admin
npm install
# optional: export QUERIA_API_URL=http://localhost:17671
npm run dev      # http://localhost:4321/admin
npm run build
npx playwright test
```

| Command | Action |
|---------|--------|
| `npm install` | Install dependencies |
| `npm run dev` | Dev server at `localhost:4321` (routes under `/admin`) |
| `npm run build` | Production build → `./dist/` |
| `npm run preview` | Preview the build |
| `npx playwright test` | E2E tests |

## Notes

- First-run `/admin/setup` creates **org + admin only** (no project seed). Create projects at `/admin/projects`.
- **Sources** (`/admin/sources`): Register Git Source form (uri, title, branch, optional path) + Trigger Ingest per row.
- **Agent Tokens** (`/admin/tokens`): mint requires **name** + at least one **project_slugs**.
- **Approvals**: approve/reject use native confirm dialogs and SSR POST handlers.
- Theme is Violet Void throughout (see tokens above).
- Full product/route status: [`../docs/HANDOFF.md`](../docs/HANDOFF.md).
