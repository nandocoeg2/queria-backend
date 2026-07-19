# Local multi-git `index-here` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship hybrid local-git index UX: CLI discovers every git root under cwd, quality-filters tracked files, uploads to central Queria as **needs_review** (not trusted), embeds via async jobs, then Admin UI + privileged MCP can **Promote** to trusted or **Reject**. Zero per-repo Admin Git forms for self-hosted / laptop clones.

**Architecture:** Client-side `queria-cli index-here` (multi-git discover + gates) → agent-bearer `POST /api/v1/agent/index-local` (validate + enqueue) → worker embed path reusing existing embed jobs → Postgres `knowledge_status = needs_review` (lane derived from status, same pattern as `scratch`) → retrieve defaults exclude it → promote elevates to approved/trusted path. Server does **not** clone unreachable remotes.

**Tech Stack:** Rust (`queria-cli`, `queria-api`, `queria-db`, `queria-worker`, `queria-search`, `queria-mcp`, `queria-core`, `queria-ingestion` gates reuse), Postgres migrations, Astro Admin, existing agent tokens + job table, Voyage embed.

**Spec:** [`../specs/2026-07-19-local-git-index-here-design.md`](../specs/2026-07-19-local-git-index-here-design.md)  
**Backlog:** IMP-L1…L6 in [`../../IMPROVEMENTS.md`](../../IMPROVEMENTS.md)

---

## Locked product decisions (sync with spec)

| Topic | Decision (2026-07-19) |
|---|---|
| Schema | New enum value **`needs_review`** on `knowledge_status` (human term; same YAGNI as scratch: **no separate lane column**; retrieve derives lane from status) |
| Terms UI/docs | **"Needs review"** (not “quarantine” in user-facing copy); code may use `needs_review` |
| Missing project | **Auto-create** project in token home org; slug from origin last path segment |
| Embed | **Async job queue** (accept batch → `job_id`; worker processes) |
| Who can **read** needs_review | **All members of home org** (Admin session + any agent token scoped to that project) |
| Who can **promote/reject** | Privileged only: Admin session (org_admin powers) + MCP tools on **explicitly granted** tokens (not default mint) |
| Slug from origin | **Last path segment only**: `git@host:group/app.git` → `app`; `https://host/group/app.git` → `app`; strip `.git` |
| Unit of index | Git roots only (`git ls-files` tracked) |
| Default retrieve | **Exclude** `needs_review` unless `include_needs_review=true` |
| Auto-promote scores | **Out of P0–P2** (IMP-L6 later; default off) |

### Slug algorithm (normative)

```text
input: origin_url OR directory basename if no remote
1. If URL-like or scp-like git@host:path:
   - take path after host (after ':' for scp, or URL path)
   - trim leading /
   - take last path segment
   - strip trailing .git (case-insensitive)
2. Else use directory basename
3. Lowercase
4. Replace non [a-z0-9-] with -
5. Collapse -- ; trim - ; if empty → "repo"
6. If slug already exists in org with different origin identity → use existing project if origin matches metadata; else create `slug-2`, `slug-3`, … (last-segment-first still holds)
```

Origin identity stored on source/index record for re-runs (do not invent second project for same origin).

### Who may call index-local

- Agent token with new permission **`IndexLocal`** (or tool name `index_local` / CLI uses bearer regardless of MCP tools list—**API permission** separate from MCP tool enum).
- Mint UI/API: optional checkbox; document for index-here.
- Token `project_slugs`: if empty meaning “all in org” does not exist today—**auto-created projects** must be **added to token scope** on create **or** allow IndexLocal to write any project in home org created by that token.  
  **v1 rule:** IndexLocal may auto-create project in home org and write needs_review for that project; token does not need pre-seeded slug. Read retrieve still respects token project allowlist when set; if token has explicit slugs only, auto-created slug must be in list **or** IndexLocal expands allowlist for projects it created (simpler: **IndexLocal implies write any home-org project it creates; list_projects returns them**). Prefer: on auto-create, attach project to token’s allowed set if token uses slug allowlist table.

### Payload / jobs

```text
POST /api/v1/agent/index-local
→ validate roots/files (gates)
→ upsert source_document kind=local_git_index (or metadata)
→ create knowledge items + chunks status=needs_review (or pending chunks)
→ enqueue embedding job(s)
→ 202 { "job_ids": [...], "roots": [{ "project_slug", "project_id", "files_accepted", "files_skipped" }] }
```

Large roots: split by file count/bytes into multiple jobs server-side. Client may also send multiple requests per root if body limit hit (CLI loops).

---

## Global constraints

- Edge port **17674**; never 67671 / queria-proxy.
- Do **not** require `QUERIA_GIT_ALLOWED_ROOTS` for this path (server-side worker only).
- Dual-lane scratch + trusted Git worker **unchanged**.
- Prefer trusted over scratch over needs_review when multi-lane fetch enabled.
- Eval/golden: trusted only (no needs_review).
- No browser disk scan.
- TruffleHog on client optional P1; server denylist mandatory.
- Fail closed on auth; clear errors on payload too large.
- Match existing Admin SSR patterns (native forms, no React islands).

---

## File map (expected)

| Area | Paths |
|---|---|
| Migration | `migrations/YYYYMMDDHHMMSS_knowledge_status_needs_review.sql` |
| Types/repos | `crates/queria-db/src/repositories/types.rs`, knowledge/chunk writers, admin queries |
| Agent API | `crates/queria-api/src/http/agent_index_local.rs` (or extend agent module), `app.rs` |
| Permissions | `crates/queria-core/src/auth/permissions.rs`, agent token mint |
| Gates shared | Prefer small shared helpers in `queria-ingestion` or `queria-core` used by CLI + API |
| CLI | `crates/queria-cli/src/…` command `index-here` |
| Worker | Existing embed job consumer; ensure needs_review chunks embeddable |
| Retrieve | `queria-search` filters: exclude `needs_review` unless flag |
| Promote API | Admin session routes + MCP tools |
| Admin UI | `admin/src/pages/…` needs-review / local-indexes list + promote/reject |
| MCP | `crates/queria-mcp/src/tools.rs`, tool dispatch |
| Docs | PRODUCT, HANDOFF, onboarding, IMPROVEMENTS status, this plan/spec links |

---

## Phases ↔ tasks

| Phase | Tasks | IMP |
|---|---|---|
| P0 | 1–4 | L1, L2 |
| P1 | 5–6 | L3, L4 |
| P2 | 7 | L5 |
| P3 | 8 (optional polish) | L6 defer; wizard copy-command only |

---

### Task 1: Schema — `needs_review` status

**Files:**
- Create: `migrations/<ts>_knowledge_status_needs_review.sql`
- Modify: any match on `knowledge_status` in Rust enums / SQL checks
- Test: migrate idempotent

**Produces:** Postgres enum value `needs_review`; code maps display **"Needs review"**.

- [ ] **Step 1: Migration**

```sql
-- Lane derived from status (same YAGNI as scratch).
ALTER TYPE knowledge_status ADD VALUE IF NOT EXISTS 'needs_review';
```

- [ ] **Step 2: Rust status enum + display**
- [ ] **Step 3:** `cargo test` / compile paths that exhaustively match status
- [ ] **Step 4:** Note HANDOFF residual when shipping (not in this task alone)

---

### Task 2: Shared quality gates + slug normalize

**Files:**
- Create or extend: e.g. `crates/queria-ingestion/src/local_index_gates.rs` or `queria-core` util
- Unit tests for slug + gates

**Produces:** Pure functions used by CLI and API.

- [ ] **Step 1: Slug normalize** — implement algorithm in Locked decisions; tests:

| input | slug |
|---|---|
| `git@github.com:nandocoeg2/fjulian.me.git` | `fjulian.me` → after replace non alnum: check rules → prefer keep dots? **v1:** replace non `[a-z0-9-]` with `-` so `fjulian-me` |
| `git@selfhosted:group/app.git` | `app` |
| `https://gitlab.example/x/y/z.git` | `z` |
| `(none)` + basename `My App` | `my-app` |

Clarify: last segment `fjulian.me` → lower → `fjulian.me` → non `[a-z0-9-]` → `fjulian-me`.

- [ ] **Step 2: Gate file** — extension allowlist (md, mdx, astro, ts, tsx, js, jsx, json, yaml, yml, toml); denylist path components; max bytes (use existing ingest max or 1_000_000); empty drop; content_hash sha256 hex of content
- [ ] **Step 3: Unit tests** for allow/deny

---

### Task 3: Agent API `POST /api/v1/agent/index-local` + jobs

**Files:**
- Create: `crates/queria-api/src/http/agent_index_local.rs`
- Modify: `app.rs` route mount under `/api/v1`
- DB: insert knowledge/chunks `needs_review`, source metadata origin/commit/branch, enqueue embed job
- Permission: `IndexLocal` on agent token

**Request sketch:**

```json
{
  "roots": [
    {
      "origin_url": "git@selfhosted:team/api.git",
      "local_path_hint": "services/api",
      "commit_sha": "abc…40",
      "branch": "main",
      "files": [
        { "path": "src/main.ts", "content": "…", "content_hash": "…" }
      ]
    }
  ]
}
```

**Behavior:**

1. Authenticate agent token; require IndexLocal.
2. Per root: compute slug; find project by (org, slug) **or** by origin_url metadata; else **create** project in home org; ensure token can access (attach slug if needed).
3. Server re-run gates; drop bad files; count skips.
4. Upsert items/chunks status=`needs_review`; store origin, commit, branch, logical path; actor = token id.
5. Enqueue embedding job(s); return **202** + job ids + per-root stats.
6. Limits: max roots per request, max files, max body (config); CLI splits.

- [ ] **Step 1: Permission + mint wiring**
- [ ] **Step 2: Handler + validation**
- [ ] **Step 3: Project resolve/create**
- [ ] **Step 4: Persist needs_review + jobs**
- [ ] **Step 5: Tests** — 401/403, gate drop, auto-create slug, org isolation, 202 job id

---

### Task 4: CLI `index-here` (IMP-L1)

**Files:**
- Modify: `crates/queria-cli` command tree
- Use Task 2 gates + discover logic
- HTTP client → edge `/api/v1/agent/index-local`

**CLI:**

```bash
queria-cli index-here \
  --token-env QUERIA_AGENT_TOKEN \
  [--edge-url-env QUERIA_EDGE_URL] \
  [--depth 4] \
  [--yes] \
  [--dry-run]
```

**Discover (normative, from spec):**

1. If cwd in work tree → add root (`git rev-parse --show-toplevel`)
2. Walk descendants depth ≤ N; on `.git` dir or file → add root; do not index files inside nested root as parent’s files
3. Dedupe canonical paths
4. Per root: origin, HEAD, branch, `git ls-files -z`

- [ ] **Step 1: Discover unit tests** (fixture dirs)
- [ ] **Step 2: Dry-run output** — list roots + file accept/skip counts
- [ ] **Step 3: Upload path** — batch by size; progress; never print token
- [ ] **Step 4: Manual smoke** against local edge

Alias: document `queria` as `queria-cli` if binary name is `queria-cli` only.

---

### Task 5: Retrieve exclude `needs_review` + flag (IMP-L3)

**Files:**
- `queria-search` hybrid filters / status allowlist
- MCP/API retrieve params: `include_needs_review` default **false**
- Authz: if true, principal must be **org member** with project access (agent token project scope or Admin session). Spec locked: **all org members may read** when flag set—not owner-only.

- [ ] **Step 1: Default query excludes needs_review** (same family as excluding draft/rejected)
- [ ] **Step 2: Flag include_needs_review**
- [ ] **Step 3: Tests** — default miss; flag hit for member token
- [ ] **Step 4: Eval path** never sets include_needs_review

**Ranking:** when included, rank after trusted and scratch (prefer trusted > scratch > needs_review).

---

### Task 6: Admin list + Promote / Reject (IMP-L4)

**Files:**
- Admin SSR page e.g. `/admin/needs-review` or section under knowledge
- API session: `GET` list, `POST` promote, `POST` reject
- Audit log events

**Promote:** set knowledge/chunks from `needs_review` → **approved** (trusted path used by Git auto-approve); ensure Qdrant payload status updated if stored.

**Reject:** status rejected or soft-delete per existing patterns; remove/hide from needs_review queue.

- [ ] **Step 1: List UI** group by project, origin, commit
- [ ] **Step 2: Promote / Reject actions** (dialog confirm like approvals)
- [ ] **Step 3: Nav link** "Needs review"
- [ ] **Step 4: Smoke** index → invisible default probe → promote → probe hit

---

### Task 7: MCP privileged promote tools (IMP-L5)

**Files:**
- `queria-mcp` tools: `list_needs_review`, `promote_knowledge`, `reject_needs_review` (names exact in tools.rs)
- Permissions: new `PromoteKnowledge` / `ListNeedsReview` (or single `ManageNeedsReview`)
- **Default mint tools array does NOT include these**

- [ ] **Step 1: Tool defs + permission**
- [ ] **Step 2: Dispatch → same service as Admin promote**
- [ ] **Step 3: Tests** — grant vs no grant
- [ ] **Step 4: Onboarding note** — privileged only

---

### Task 8: Docs + optional wizard polish (P3 lite)

**Files:**
- `docs/PRODUCT.md` — needs_review lane + index-here + promote surfaces
- `docs/HANDOFF.md` — matrix rows when shipped
- `docs/runbooks/onboarding.md` — Part “index-here” one-command
- `docs/runbooks/agent-onboard-prompt.md` — optional mention
- Admin page: copy-paste block for `index-here` command (no form fields)
- IMPROVEMENTS IMP-L* status when done
- Spec open items → resolved (this plan)

- [ ] **Step 1: Living docs**
- [ ] **Step 2: Admin copy-command panel**
- [ ] **Step 3: IMP-L6 left `proposed` / deferred** unless product reopens auto-promote

---

## Testing matrix

| Layer | Cases |
|---|---|
| Unit | slug, gates, discover nested / no-git / worktree |
| API | auth, auto-create, skip bad files, job 202, org isolation |
| Retrieve | default exclude; include_needs_review member ok |
| Promote | Admin + MCP grant; no-grant 403; post-promote trusted hit |
| Regression | existing Git worker ingest + scratch index_memory |

Commands:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

---

## Out of order / do not

- Do not auto-promote to trusted in P0–P2
- Do not add browser file picker full-tree index
- Do not route self-hosted through `QUERIA_GIT_ALLOWED_ROOTS` for this feature
- Do not put promote on default agent tokens
- Do not use user-facing word “quarantine” in Admin (use **Needs review**)

---

## Open items → **resolved**

| # | Resolution |
|---|---|
| 1 Schema | `knowledge_status` += `needs_review`; no extra lane column |
| 2 Project | Auto-create in home org from slug |
| 3 Embed | Async jobs, return job_ids |
| 4 Visibility | All org members may read with flag / Admin list; promote privileged |
| 5 Slug | Last path segment only + normalize; collision `-2`, `-3` if needed |

---

## Execution order

```text
Task 1 (schema) → Task 2 (gates/slug) → Task 3 (API+jobs) → Task 4 (CLI)
  → Task 5 (retrieve) → Task 6 (Admin promote) → Task 7 (MCP) → Task 8 (docs)
```

Ship P0 (1–4) behind feature usable with CLI + API even if Admin promote comes in P1 (operator can use SQL only as emergency—not documented). Prefer ship 1–6 together for usable loop.

---

## Acceptance (end-to-end)

```text
1. Workspace with 2 nested git repos (self-hosted remotes OK offline to OCI)
2. queria-cli index-here --yes --token-env QUERIA_AGENT_TOKEN
3. Jobs complete; Admin "Needs review" shows both origins
4. Default retrieval probe: no needs_review chunks
5. Promote one project → probe hits trusted citations
6. Agent token without promote tools cannot promote via MCP
7. Classic GitHub cloud ingest still works
```
