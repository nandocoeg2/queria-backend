# Design: queria-cli release automation (detect → tag → build → Homebrew)

> Status: REFERENCE (approved design; not implemented)  
> Last verified: 2026-07-23  
> Runtime truth: [`../../HANDOFF.md`](../../HANDOFF.md)  
> Related runbooks: [`../../runbooks/deployment.md`](../../runbooks/deployment.md) § queria-cli Release; [`../../runbooks/queria-cli-homebrew.md`](../../runbooks/queria-cli-homebrew.md)  
> Existing workflow (unchanged matrix): [`.github/workflows/release-cli.yml`](../../../.github/workflows/release-cli.yml)  
> Formula generator: [`../../../scripts/generate_homebrew_formula.sh`](../../../scripts/generate_homebrew_formula.sh)

## 1. Goal

Remove the human glue between a version bump on `main` and a laptop-installable CLI:

**Operator today (manual):**

1. Bump `crates/queria-cli/Cargo.toml` (+ `Cargo.lock`)
2. Commit + push `main`
3. `git tag -a cli-vX.Y.Z` + push tag
4. Wait for **Release queria-cli** CI
5. Run `generate_homebrew_formula.sh`
6. Commit + push `homebrew-queria`
7. `brew reinstall` on each machine

**Operator after this design:**

1. Bump version in Cargo.toml, commit, push `main`
2. Wait (tag → build → GitHub Release → formula on tap)
3. `brew update && brew reinstall nandocoeg2/queria/queria-cli` (still per machine; requires `HOMEBREW_GITHUB_API_TOKEN` while backend is private)

Push of plain feature commits without a version change must **not** release.

## 2. Non-goals

- Auto-deploy of API/edge/worker images (separate from CLI release)
- Auto `brew reinstall` on developer laptops
- Merging Homebrew updates via PR review (direct push to tap `main` is intentional)
- Changing the multi-arch matrix / runner pins in `release-cli.yml`
- Semver policy bots (major/minor policy stays human; only wire existing bumps)

## 3. Architecture (Approach A)

Three stages. Existing stage 2 stays the source of truth for builds.

```text
main push
  │
  ▼
┌─────────────────────────────────────┐
│ 1. detect-and-tag (NEW)             │
│    if Cargo.toml version has no     │
│    cli-v{version} tag → create+push │
│    annotated tag cli-v{version}     │
└──────────────┬──────────────────────┘
               │ tag push refs/tags/cli-v*
               ▼
┌─────────────────────────────────────┐
│ 2. Release queria-cli (EXISTING)    │
│    release-cli.yml — UNCHANGED      │
│    matrix build + GitHub Release    │
└──────────────┬──────────────────────┘
               │ release: published (cli-v*)
               ▼
┌─────────────────────────────────────┐
│ 3. homebrew-formula (NEW)           │
│    generate formula from assets     │
│    direct-push homebrew-queria main │
└─────────────────────────────────────┘
```

Rationale for splitting (not a mega-workflow):

- Matrix reliability stays isolated in the known-good workflow
- Partial re-runs: re-tag/dispatch still works for builds; formula can re-run on `workflow_dispatch` without rebuilding
- Failures are stage-scoped in Actions UI

## 4. Stage 1 — Detect and tag

### Trigger

```yaml
on:
  push:
    branches: [main]
    paths:
      - 'crates/queria-cli/Cargo.toml'
      - 'Cargo.lock'
```

Path filter reduces noise; **authoritative gate is still the version comparison** (lockfile-only pushes without version change must no-op).

Also allow `workflow_dispatch` with optional version override for recovery (empty = read from Cargo.toml).

### Logic (single job, ubuntu-latest)

1. Checkout `main` at the pushed SHA (full history not required; fetch tags required).
2. Parse version from `crates/queria-cli/Cargo.toml`:
   - Prefer a tiny stable parse: `rg '^version = "([^"]+)"'` in that file’s package table, or `cargo metadata --no-deps --format-version 1` filtered to `queria-cli`.
   - Record as `VERSION` (e.g. `0.3.4`). Reject empty / non-semver-looking strings.
3. Set `TAG=cli-v${VERSION}`.
4. List existing tags: if `git ls-remote --tags origin "refs/tags/${TAG}"` already exists → **exit 0 success with message “tag exists; skip”** (idempotent; no force).
5. If missing: create annotated tag on **exactly** `github.sha`:
   ```bash
   git tag -a "$TAG" -m "queria-cli ${VERSION}"
   git push origin "refs/tags/${TAG}"
   ```
6. Concurrency: `group: cli-detect-tag`, `cancel-in-progress: false` so two main pushes serialize and the second no-ops if the first already created the tag.

### Permissions / identity

- `contents: write` (or a fine-scoped deploy key / GitHub App) so Actions can push tags.
- Prefer `GITHUB_TOKEN` with `contents: write` if org/repo settings allow tag push; if branch protection blocks, document use of a PAT stored as `RELEASE_BOT_TOKEN` with `contents: write` only.
- Tag must point at the commit that introduced the version bump (the push SHA), not a later unrelated commit.

### Failure modes

| Case | Behavior |
|------|----------|
| Version unchanged vs existing tag | Skip (success) |
| Cargo.toml version invalid | Fail job (do not invent tag) |
| Tag push rejected (perms) | Fail with clear log; operator fixes token |
| Race: two workflows create same tag | Second push fails “already exists” → treat as skip success if remote tag points at expected SHA, else fail |

## 5. Stage 2 — Build and GitHub Release (existing)

**No matrix or publish-step changes** to `release-cli.yml` as part of this design.

- Continues to trigger on `push: tags: ['cli-v*']` and `workflow_dispatch` with tag input.
- Still builds darwin arm/intel + linux x64 (linux arm optional).
- Still creates/updates GitHub Release with tarballs + sha256.
- Manual operator paths remain valid: local `git tag` + push, or Actions “Run workflow” with tag input (unstick).

Stage 1 must not reimplement builds.

## 6. Stage 3 — Homebrew formula push

### Trigger

```yaml
on:
  release:
    types: [published]
  workflow_dispatch:
    inputs:
      tag:
        description: 'cli-vX.Y.Z (empty = latest cli-v* release)'
        required: false
```

Guard: only run when `github.event.release.tag_name` (or dispatch input) matches `cli-v*`. Ignore other releases if any appear later.

### Logic

1. Resolve `TAG` (from event or input).
2. Checkout **queria-backend** at the tag (or default branch + run generator; generator only needs network access to Release assets).
3. Ensure secrets present:
   - **Asset download** (private backend): `GH_TOKEN` / `GITHUB_TOKEN` with `contents: read` on `nandocoeg2/queria-backend` (Actions default token on same repo is enough for API asset download when workflow runs in backend repo).
   - **Tap push**: secret `HOMEBREW_TAP_TOKEN` (PAT or fine-grained) with `contents: write` on `nandocoeg2/homebrew-queria` only.
4. Checkout `homebrew-queria` (or clone) into a sibling path expected by the generator, **or** invoke:

   ```bash
   GH_TOKEN="${{ secrets.GITHUB_TOKEN }}" \
     ./scripts/generate_homebrew_formula.sh "$TAG" \
     --out homebrew-queria/Formula/queria-cli.rb
   ```

   Use the fixed generator behavior: asset IDs persisted via tmp files (private-safe API URLs + `headers:` block).
5. In the tap clone:
   - If `git diff` empty (formula already at this version/SHAs) → skip push success.
   - Else commit `chore: queria-cli X.Y.Z formula (private API assets)` and `git push origin main`.
6. Do **not** invent sha256; generator already exits non-zero if assets missing.

### Permissions

| Secret | Scope |
|--------|--------|
| `GITHUB_TOKEN` (backend workflow) | Read release assets on same repo |
| `HOMEBREW_TAP_TOKEN` | Write push to `nandocoeg2/homebrew-queria` main |

Never store a broad personal token in the formula text. Formula continues to use `ENV.fetch("HOMEBREW_GITHUB_API_TOKEN", …)` for **brew install** time on private assets (laptop-side).

### Failure modes

| Case | Behavior |
|------|----------|
| Release published before all assets ready | Softprops uploads with release; if incomplete, generator fails → job red; re-run `workflow_dispatch` after assets complete |
| Linux arm missing | Generator warns; formula odie arm (existing contract) |
| Tap push rejected | Fail; human fixes token / branch protection on tap |
| Formula identical | Skip push (idempotent re-run) |

## 7. Secrets and repo settings checklist

Document in runbooks (implementation must not leave these only in chat):

| Item | Where |
|------|--------|
| Allow Actions to create tags on `queria-backend` | Repo settings / token |
| `HOMEBREW_TAP_TOKEN` on `queria-backend` | Actions secrets |
| Tap repo: allow the bot identity to push `main` | `homebrew-queria` |
| Operators: `HOMEBREW_GITHUB_API_TOKEN` for private `brew install` | Laptop env (unchanged) |

Optional later: GitHub App with least privilege instead of PAT.

## 8. Operator workflow (after automation)

### Happy path

```bash
# on main, after feature is ready
# 1) edit crates/queria-cli/Cargo.toml version → next patch/minor
# 2) cargo build -p queria-cli  # refresh Cargo.lock version line if needed
git add crates/queria-cli/Cargo.toml Cargo.lock
git commit -m "chore(cli): bump queria-cli to X.Y.Z"
git push origin main
# Actions: detect-and-tag → Release queria-cli → homebrew-formula
# When green:
export HOMEBREW_GITHUB_API_TOKEN=$(gh auth token)
brew update && brew reinstall nandocoeg2/queria/queria-cli
queria-cli --version
```

### Unstick

| Problem | Fix |
|---------|-----|
| Version bump pushed but no tag | Check detect-and-tag logs; re-run dispatch or create tag manually on the bump commit |
| Tag exists, no assets | Existing deployment.md unstick (dispatch release-cli with tag) |
| Assets OK, formula not updated | Re-run homebrew-formula `workflow_dispatch` with tag |
| Wrong version tagged | Do not force-move tags lightly; cut new patch version |

Manual tag push without Cargo.toml bump still works (stage 2+3 only); stage 1 no-ops on next main push if tag already exists.

## 9. Documentation deliverables (required)

Automation without docs is incomplete for this project. Implementation PR **must** update:

| Document | Change |
|----------|--------|
| **This design** | Stay REFERENCE until implemented; then mark SUPERSEDED/archive or PARTIAL→done note |
| [`runbooks/deployment.md`](../../runbooks/deployment.md) | Replace “Cut a new CLI release (happy path)” with auto pipeline; keep unstick; add stage diagram + secret names |
| [`runbooks/queria-cli-homebrew.md`](../../runbooks/queria-cli-homebrew.md) | “After every CLI release” becomes mostly automatic; keep manual generator path as fallback; document CI secret + direct-push |
| [`HANDOFF.md`](../../HANDOFF.md) | One CURRENT note: CLI release chain stages + residual (laptop brew still manual; private token for brew) |
| Workflow header comments | Short “stage 1/3 of release chain” pointers in new YAMLs; one line in `release-cli.yml` that stage 2 is triggered by tags (manual or detect-and-tag) |
| Optional: `docs/README.md` / parent index | Link the design while REFERENCE |

Do **not** dual-maintain a long second copy of the full design in HANDOFF—pointer + residual only.

## 10. Testing and acceptance

### Local / CI checks before merge of automation

1. Parse unit: script or job step that given a fixture Cargo.toml extracts version correctly.
2. Dry-run detect: on a branch, job mode `DRY_RUN=1` prints TAG decision without pushing (optional flag for dev).
3. Generator still green against a live `cli-v*` (existing script contract).
4. Idempotent re-run of formula job does not create empty commits.

### Acceptance criteria (done when)

- [ ] Bump `queria-cli` version on `main` alone produces tag `cli-v{version}` without human tag command
- [ ] That tag produces green **Release queria-cli** with required assets
- [ ] Formula on `homebrew-queria` main updates to same version with private API asset URLs
- [ ] Second push of same version does nothing harmful (no force-tag, no bogus release)
- [ ] Feature-only main push (no version change) creates no CLI release
- [ ] Manual unstick paths still documented and work
- [ ] Runbooks + HANDOFF updated as in §9

## 11. Risks and mitigations

| Risk | Mitigation |
|------|------------|
| Accidental version bump releases unfinished CLI | Code review on Cargo.toml version bumps; keep features mergeable without bump |
| Token over-privilege | Separate `HOMEBREW_TAP_TOKEN` scoped to tap only |
| Private asset 404 in formula job | Reuse generator + `GITHUB_TOKEN`; fail loud; dispatch re-run |
| Tag on wrong commit | Tag only `github.sha` of the push that contains the bump file change |
| Concurrent bumps | Concurrency group; reject non-ff tag push |

## 12. Implementation outline (for planning skill)

Files likely added/changed (not implemented by this design alone):

1. `.github/workflows/cli-detect-and-tag.yml` (new)
2. `.github/workflows/cli-homebrew-formula.yml` (new)
3. Possibly tiny `scripts/cli_version.sh` shared by detect job
4. Repo secrets + docs in §9
5. No change to release matrix unless a discovered bug blocks automation

Suggested implement order: detect-and-tag (can validate with dry tag on a patch) → formula job (re-run on existing `cli-v0.3.3`) → docs → full e2e with `0.3.4` bump.

## 13. Decisions log

| Decision | Choice |
|----------|--------|
| Scope | Full pipe: version on main → tag → build → Homebrew |
| Detect trigger | `crates/queria-cli/Cargo.toml` version has no matching `cli-v*` tag |
| Homebrew update | Direct push to `homebrew-queria` `main` |
| Architecture | Approach A: thin wrappers; leave `release-cli.yml` matrix untouched |
| Docs | First-class: update deployment + homebrew runbooks + HANDOFF with residual |
