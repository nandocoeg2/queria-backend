# Design: Local multi-git `index-here` → needs_review → promote

> Status: REFERENCE (approved product direction; not implemented)
> Last verified: 2026-07-19
> Implementation plan: [`../plans/2026-07-19-local-git-index-here.md`](../plans/2026-07-19-local-git-index-here.md)
> Runtime truth: [`../../HANDOFF.md`](../../HANDOFF.md)
> Product contract: [`../../PRODUCT.md`](../../PRODUCT.md)
> Backlog: [`../../IMPROVEMENTS.md`](../../IMPROVEMENTS.md) (IMP-L1…L6 when landed)
> Related DX: [`../../runbooks/agent-onboard-prompt.md`](../../runbooks/agent-onboard-prompt.md)

## Problem

1. **Admin Git source forms** (URI, branch, `source_path`, instance allowlists) are too heavy for “just use it.”
2. Many repos are **self-hosted / unreachable** from the central Queria worker. Clone exists only on the **developer machine**.
3. A coding workspace is often **multi-root / nested**: cwd is not always a single git root; several git projects live under one tree.
4. Dumping a full tree into **trusted** shared knowledge without gates pollutes retrieval (garbage chunks).

**Need:** zero form per-repo; one CLI on the machine that has the clones; multi-git discover; quality gates; default **needs_review** (not trusted); **promote** via Admin UI and privileged MCP.

## Non-goals (v1)

| Out | Why |
|---|---|
| Browser Admin scanning local disk | Browser cannot see workspace `.git` safely |
| Central worker cloning unreachable self-hosted remotes | Hybrid: index where the clone lives |
| enowx-style index of **any** folder without git | Product: **git repos only** |
| Auto-trust entire tree as team truth | Garbage / secret / generated risk |
| Default agent tokens with promote | Promote is privileged |
| Replacing cloud Git worker for public/reachable remotes | Keep existing pipeline; this is an **additional** path |
| Multiprovider / multi vector-store | SIMPLIFICATION |

## Locked decisions (2026-07-19)

| Knob | Choice |
|---|---|
| Primary user | Human Admin wants **wizard / one command**, not multi-field forms |
| Runtime | **Hybrid**: central retrieve; **index on agent/dev machine** |
| Unit of index | **Git repository only** (must have valid `.git` / worktree) |
| Discover | From **cwd**: nested scan + current root; **not** “cwd always one monorepo” |
| Remote | Auto from `git remote get-url origin` (identity/metadata); server need not fetch |
| CLI | `queria index-here --token-env QUERIA_AGENT_TOKEN` |
| Shared vs garbage | Upload → **`needs_review`** (user-facing: **Needs review**); trusted only after promote (auto-score off in v1) |
| Promote | Admin UI **and** privileged MCP tools (explicit grant; not default mint) |
| Content | Tracked files only (`git ls-files`), same family of allow/deny as Git worker |
| Schema (locked) | `knowledge_status` enum value **`needs_review`** (no separate lane column; same YAGNI as `scratch`) |
| Missing project | **Auto-create** in token home org; slug = origin **last path segment** (normalized) |
| Embed | **Async job** queue; API returns `job_id`(s) |
| Read needs_review | **All home-org members** (Admin list; retrieve with `include_needs_review`) |
| Slug (locked) | Last segment only, e.g. `group/app.git` → `app` → normalize to `[a-z0-9-]+` |

## Design summary

```text
Dev machine (has self-hosted / private clones)
  queria index-here --token-env QUERIA_AGENT_TOKEN
       │
       ├─ discover all git roots under cwd (depth-limited)
       ├─ per root: origin, HEAD, branch, ls-files
       ├─ client quality gate (extensions, denylist, size, optional TruffleHog)
       └─ POST batch → central API (Bearer agent token)
              │
              ▼
         status needs_review (project-scoped, not trusted)
              │
              ├─ retrieve: default excludes needs_review;
              │            optional include_needs_review (org members)
              │
              ▼
         Promote (Admin UI or privileged MCP)
              │
              ▼
         trusted shared knowledge (prefer-trusted ranking)
```

Cloud SSH Git ingest (existing worker) remains for remotes the **server** can reach.

## Multi-git discovery

Start at `cwd` (process working directory when CLI runs).

```text
candidates = {}

1. If cwd is inside a git work tree → add that work tree root
2. Walk descendants up to --depth (default 4):
   - if directory contains .git (dir or file worktree) → add root
   - do not descend into another git root's internals beyond recording the root
3. Dedupe by canonical path
4. For each root collect:
   - absolute path
   - origin URL (if any; allow missing remote)
   - commit SHA (rev-parse HEAD)
   - branch (symbolic-ref or "detached")
```

**Rules:**

- Paths **without** git are **skipped** (never walked as enowx folder index).
- Nested multi-project workspaces index **each** git root as its own project/source mapping.
- Optional prompt listing found repos; `--yes` indexes all (CI).
- `--dry-run` lists only.

**Slug mapping (locked — see plan):**

- Last path segment only: `git@host:group/app.git` → `app` → normalize (`fjulian.me` → `fjulian-me`).
- No remote: directory basename, same normalize.
- Collision / distinct origin: suffix `-2`, `-3`; same origin reuses project.

## CLI surface

Binary: extend `queria-cli` (preferred; already shipped) or thin alias `queria` → same binary.

```bash
queria index-here \
  --token-env QUERIA_AGENT_TOKEN \
  [--edge-url-env QUERIA_EDGE_URL] \
  [--depth 4] \
  [--yes] \
  [--dry-run]
```

| Flag | Role |
|---|---|
| `--token-env` | Env var name holding raw agent token (default `QUERIA_AGENT_TOKEN`) |
| Edge URL | `QUERIA_EDGE_URL` / default local `http://127.0.0.1:17674` or prod public base from docs |
| `--depth` | Nested git scan limit |
| `--yes` | Non-interactive accept all discovered roots |
| `--dry-run` | Discover + gate counts; no upload |

**Does not** ask user for URI, branch, or `source_path` forms.

Auth: `Authorization: Bearer` from token env. Never print full token.

## Client quality gate (anti-garbage, pre-upload)

Per file (after `git ls-files`):

| Gate | Behavior |
|---|---|
| Tracked only | Untracked ignored |
| Extension allowlist | Align with worker: md/mdx/astro/ts/tsx/js/jsx/json/yaml/yml/toml (+ expandable in plan) |
| Path denylist | `.git`, `node_modules`, `dist`, `build`, `target`, coverage, lockfiles, `.env*` |
| Max bytes | Drop over limit |
| Empty / whitespace-only | Drop |
| Optional TruffleHog | Skip or fail file; do not upload secrets |
| content_hash | Skip if server already has same hash for path+project |

CLI reports skipped counts (denied path, size, secret, unchanged).

## Server ingest API (sketch)

```text
POST /api/v1/agent/index-local
Authorization: Bearer qria_…
Content-Type: application/json

{
  "roots": [
    {
      "origin_url": "git@selfhosted:team/api.git",
      "local_path_hint": "services/api",
      "commit_sha": "…",
      "branch": "main",
      "files": [
        { "path": "src/main.ts", "content": "…", "content_hash": "…" }
      ]
    }
  ]
}
```

**Server duties:**

1. Authz agent token + project resolution/create (home org only).
2. Re-apply denylist/size (never trust client alone).
3. Chunk + embed (Voyage path existing).
4. Persist with **`knowledge_status = needs_review`**:
   - not in default retrieve
   - attributable to token subject + origin + commit
5. **Enqueue embed job(s)**; respond **202** with `job_ids` (not sync embed for large trees)
6. Rate / payload limits; reject huge bodies with clear error; CLI splits batches

**Idempotency:** `(organization_id, project_id, origin, path, content_hash)` upsert; re-index same origin updates/supersedes stale paths (plan).

**Project:** auto-create in home org when slug/origin unknown; IndexLocal permission required.

## Knowledge lanes (product)

Extend dual-lane thinking:

| Lane / status | Write | Default `retrieve_context` |
|---|---|---|
| **trusted** (`approved`, etc.) | Git worker, promote, approvals | Yes |
| **scratch** | `index_memory` (short notes) | Yes if `include_scratch` (agent default true) |
| **needs_review** (new) | `index-here` bulk local git | **No** by default; yes if `include_needs_review` (**any org member** with project access) |
| **proposed** | `propose_memory` | No until approve |

**Ranking:** prefer trusted over scratch over needs_review when multi-status fetch is enabled.

**Eval / golden:** trusted-only (no needs_review).

**User-facing label:** "Needs review" (avoid "quarantine" in Admin UI).

## Promote

### Admin UI

- Surface **Needs review** queue:
  - group by project, origin, commit
  - actions: **Promote** (→ trusted/approved), **Reject**, optional bulk
- One-click mental model: select row → Promote
- Optional: copy `index-here` command block (no multi-field form)

### MCP (privileged)

Tools (see plan for exact names):

- `list_needs_review`
- `promote_knowledge` / promote by ids or origin+commit filter
- `reject_needs_review`

**Token grant:** explicit; **not** in default agent mint. Admin session always can promote/reject.

Promote writes audit_log. After promote, chunks join normal trusted retrieval.

### Auto-promote (optional, v1 default off)

Only if all score hard gates pass (e.g. docs-only path prefix, no secrets, size band). Prefer **off** at ship; needs_review + human/privileged promote first.

## Admin “wizard” without long forms

Browser cannot index disk. Wizard = **copy one command** + status of Needs review jobs:

```text
1. Install/use queria-cli
2. In workspace root:
   export QUERIA_AGENT_TOKEN=…
   export QUERIA_EDGE_URL=https://queria.fjulian.id
   queria index-here --token-env QUERIA_AGENT_TOKEN
3. Open Admin → Needs review → Promote when ready
```

Optional later: “Generate agent token with IndexLocal scope” single button (still not per-repo fields).

## Security

| Risk | Mitigation |
|---|---|
| Secret exfil via upload | Client + server denylist; optional TruffleHog; never index `.env` / keys |
| Cross-tenant index | Token home org only; project scope |
| Promote abuse | Privileged tools only; audit |
| Huge payload DoS | Body limits, file counts, embed budget |
| Path escape | Server ignores client absolute paths for FS; uses content + logical path only |
| Token in shell history | Document env var; same as MCP onboarding |

Central server **never** requires `QUERIA_GIT_ALLOWED_ROOTS` for this path (those remain for **server-side** Git worker only).

## Relation to `QUERIA_GIT_ALLOWED_ROOTS`

| Mechanism | Purpose |
|---|---|
| `QUERIA_GIT_ALLOWED_ROOTS` | Server worker may read paths when doing **central** git_repo ingest |
| `index-here` | Client reads only its own cwd tree; uploads content over HTTPS |

Do not conflate in user-facing docs for this feature.

## Phased delivery

| Phase | Deliverable | Acceptance (sketch) |
|---|---|---|
| **P0** | CLI discover + dry-run + gates; API needs_review ingest; **async** embed jobs | Multi-git lists N roots; dry-run no write; upload → needs_review + job_ids |
| **P1** | Default retrieve ignores needs_review; Admin list + Promote/Reject | After promote, probe hits trusted; before, default miss |
| **P2** | MCP promote/list (privileged grants) | Token without grant cannot promote |
| **P3** | Auto-promote (optional, off); copy-command polish | Defaults remain safe |

Suggested backlog IDs (IMPROVEMENTS):

| ID | Item |
|---|---|
| IMP-L1 | CLI `index-here` multi-git discover + gates |
| IMP-L2 | API + storage `needs_review` + async embed |
| IMP-L3 | Retrieve `include_needs_review` (org members) |
| IMP-L4 | Admin promote/reject UI (“Needs review”) |
| IMP-L5 | MCP promote tools (privileged) |
| IMP-L6 | Optional auto-promote rules (default off) |

## Testing

- Unit: discovery fixtures (nested repos, no-git dirs, worktree file `.git`)
- Gate: denylist, size, hash skip
- API: authz, org isolation, payload limit
- E2E: index-here → needs_review invisible to default retrieve → promote → hit
- No regression on existing cloud Git worker path

## Open items

**Resolved in plan** [`../plans/2026-07-19-local-git-index-here.md`](../plans/2026-07-19-local-git-index-here.md):

| # | Resolution |
|---|---|
| 1 Schema | `knowledge_status` += `needs_review` (no lane column) |
| 2 Project | Auto-create home org; last-segment slug |
| 3 Embed | Async jobs + `job_ids` |
| 4 Visibility | All org members may read (flag / Admin); promote privileged |
| 5 Slug | Last segment + normalize; collision `-2`… |

## Changelog

| Date | Note |
|---|---|
| 2026-07-19 | Initial REFERENCE design (hybrid multi-git, index-here, needs_review, dual promote) |
| 2026-07-19 | Locked open items + linked implementation plan; user-facing **Needs review** |
