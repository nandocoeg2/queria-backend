# queria-cli Release Automation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Automate queria-cli release so a Cargo.toml version bump on `main` produces tag `cli-vX.Y.Z`, reuses existing **Release queria-cli** for multi-arch builds + GitHub Release, then direct-pushes a private-safe formula to `nandocoeg2/homebrew-queria`.

**Architecture:** Three thin stages (Approach A). Stage 1 detects version and pushes annotated tag. Stage 2 is existing `release-cli.yml` (matrix unchanged). Stage 3 runs on `release: published`, runs `scripts/generate_homebrew_formula.sh`, commits + pushes formula to the tap. Docs updated so operators do not rely on chat history.

**Tech Stack:** GitHub Actions, bash, git tags, existing formula generator, Markdown runbooks.

**Spec:** [`docs/superpowers/specs/2026-07-23-cli-release-automation-design.md`](../specs/2026-07-23-cli-release-automation-design.md)

## Global Constraints

- Do **not** change the multi-arch matrix / runner pins in `.github/workflows/release-cli.yml` (no `macos-13`; Darwin on `macos-14`; linux arm optional).
- Push of plain feature commits **without** a `queria-cli` version change must **not** create a release.
- Never invent formula `sha256`; only hashes from real downloads (existing generator contract).
- Formula must keep **private** Releases API asset URLs + `headers:` Bearer from env at brew-install time (not bake a token into the formula).
- Tap update is **direct push** to `homebrew-queria` `main` (not PR).
- Tag only `github.sha` of the push that carries the version; never force-move tags.
- Secrets: `HOMEBREW_TAP_TOKEN` for tap write; `GITHUB_TOKEN` for same-repo release assets. Do not commit tokens.
- Docs deliverables required: `deployment.md`, `queria-cli-homebrew.md`, `HANDOFF.md` residual, workflow header comments.
- Work on an isolated worktree branch under `queria/backend` for code/docs; tap is a **sibling** git repo.

---

## File map

| Path | Role |
|------|------|
| `scripts/cli_version.sh` | **Create** — parse queria-cli version; shared by Stage 1 + local tests |
| `.github/workflows/cli-detect-and-tag.yml` | **Create** — Stage 1 detect + annotated tag push |
| `.github/workflows/cli-homebrew-formula.yml` | **Create** — Stage 3 formula generate + tap push |
| `.github/workflows/release-cli.yml` | **Modify** — header comment only (stage 2 pointer) |
| `scripts/generate_homebrew_formula.sh` | **Read-only** unless a CI bug is found (already private-safe) |
| `docs/runbooks/deployment.md` | **Modify** — happy path auto pipeline + secrets + unstick |
| `docs/runbooks/queria-cli-homebrew.md` | **Modify** — auto after release; manual fallback |
| `docs/HANDOFF.md` | **Modify** — short CURRENT residual for CLI release chain |
| `docs/superpowers/specs/2026-07-23-cli-release-automation-design.md` | **Modify** — note implemented / leave REFERENCE until done |

---

### Task 1: Version parse helper + unit tests

**Files:**
- Create: `scripts/cli_version.sh`
- Create: `scripts/tests/cli_version_test.sh` (or `scripts/test_cli_version.sh` if no tests dir)

**Interfaces:**
- Produces: `cli_version_from_cargo_toml [path]` → prints semver on stdout, exit 0; invalid → exit 1 stderr
- Produces: `cli_tag_for_version <version>` → prints `cli-v{version}`
- Consumes: plain Cargo.toml with first package `version = "x.y.z"`

- [ ] **Step 1: Write failing test harness**

Create `scripts/tests/cli_version_test.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=../cli_version.sh
source "$ROOT/scripts/cli_version.sh"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

cat >"$tmpdir/ok.toml" <<'EOF'
[package]
name = "queria-cli"
version = "0.3.3"
edition = "2021"
EOF

cat >"$tmpdir/bad.toml" <<'EOF'
[package]
name = "queria-cli"
edition = "2021"
EOF

got="$(cli_version_from_cargo_toml "$tmpdir/ok.toml")"
test "$got" = "0.3.3"
test "$(cli_tag_for_version "$got")" = "cli-v0.3.3"

if cli_version_from_cargo_toml "$tmpdir/bad.toml" 2>/dev/null; then
  echo "expected failure on missing version" >&2
  exit 1
fi

# reject garbage
cat >"$tmpdir/junk.toml" <<'EOF'
version = "not-a-semver!"
EOF
if cli_version_from_cargo_toml "$tmpdir/junk.toml" 2>/dev/null; then
  echo "expected failure on invalid version" >&2
  exit 1
fi

echo "cli_version_test: ok"
```

- [ ] **Step 2: Run test — expect FAIL (script missing)**

```bash
chmod +x scripts/tests/cli_version_test.sh
bash scripts/tests/cli_version_test.sh
```

Expected: fail sourcing or function not found.

- [ ] **Step 3: Implement `scripts/cli_version.sh`**

```bash
#!/usr/bin/env bash
# Shared queria-cli version helpers for release automation (Stage 1).
# Compatible with macOS /bin/bash 3.2 and Ubuntu bash.

set -euo pipefail

# Print package version from a Cargo.toml path (queria-cli crate file).
# Usage: cli_version_from_cargo_toml path/to/Cargo.toml
cli_version_from_cargo_toml() {
  local file="${1:-}"
  if [[ -z "$file" || ! -f "$file" ]]; then
    echo "cli_version_from_cargo_toml: file not found: ${file:-}" >&2
    return 1
  fi
  # First version = "..." in file (package table is first in queria-cli Cargo.toml).
  local line ver
  line="$(grep -E '^version = "' "$file" | head -n1 || true)"
  if [[ -z "$line" ]]; then
    echo "cli_version_from_cargo_toml: no version line in $file" >&2
    return 1
  fi
  ver="${line#version = \"}"
  ver="${ver%\"}"
  # Semver-ish: digits.digits.digits with optional pre-release suffix -foo
  if ! printf '%s' "$ver" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.+-]+)?$'; then
    echo "cli_version_from_cargo_toml: invalid version: $ver" >&2
    return 1
  fi
  printf '%s\n' "$ver"
}

# Print cli-v{version} for a version string.
cli_tag_for_version() {
  local ver="${1:-}"
  if [[ -z "$ver" ]]; then
    echo "cli_tag_for_version: empty version" >&2
    return 1
  fi
  printf 'cli-v%s\n' "$ver"
}

# If executed directly: print version from crates/queria-cli/Cargo.toml relative to repo root.
if [[ "${BASH_SOURCE[0]:-}" == "${0}" ]]; then
  _root="$(cd "$(dirname "$0")/.." && pwd)"
  cli_version_from_cargo_toml "${1:-$_root/crates/queria-cli/Cargo.toml}"
fi
```

- [ ] **Step 4: Run test — expect PASS**

```bash
bash scripts/tests/cli_version_test.sh
# Expected: cli_version_test: ok
bash scripts/cli_version.sh
# Expected: current package version e.g. 0.3.3
bash -n scripts/cli_version.sh scripts/tests/cli_version_test.sh
```

- [ ] **Step 5: Commit**

```bash
git add scripts/cli_version.sh scripts/tests/cli_version_test.sh
git commit -m "feat(scripts): add cli_version parse helpers for release automation"
```

---

### Task 2: Stage 1 workflow — detect-and-tag

**Files:**
- Create: `.github/workflows/cli-detect-and-tag.yml`
- Consumes: `scripts/cli_version.sh` from Task 1

**Interfaces:**
- Produces: annotated tag `cli-v{VERSION}` on `github.sha` when remote tag missing
- Produces: green no-op when tag already exists
- Env: `DRY_RUN=1` (optional input) skips push

- [ ] **Step 1: Create workflow file**

`.github/workflows/cli-detect-and-tag.yml`:

```yaml
# Stage 1 of queria-cli release chain (see docs/superpowers/specs/2026-07-23-cli-release-automation-design.md).
# On main: if crates/queria-cli Cargo.toml version has no matching cli-v* tag, push annotated tag.
# Stage 2 (release-cli.yml) is triggered by the tag push. Does not build binaries.
name: CLI detect-and-tag

on:
  push:
    branches: [main]
    paths:
      - "crates/queria-cli/Cargo.toml"
      - "Cargo.lock"
      - "scripts/cli_version.sh"
      - ".github/workflows/cli-detect-and-tag.yml"
  workflow_dispatch:
    inputs:
      dry_run:
        description: "If true, print decision and do not push tag"
        required: false
        default: "false"
        type: choice
        options:
          - "false"
          - "true"
      version_override:
        description: "Optional X.Y.Z override (empty = read Cargo.toml)"
        required: false
        default: ""

concurrency:
  group: cli-detect-tag
  cancel-in-progress: false

permissions:
  contents: write

jobs:
  detect-and-tag:
    runs-on: ubuntu-latest
    steps:
      - name: Checkout
        uses: actions/checkout@v4
        with:
          fetch-depth: 0
          # Use default GITHUB_TOKEN; if tag push is blocked, set repo secret RELEASE_BOT_TOKEN
          # and: token: ${{ secrets.RELEASE_BOT_TOKEN || secrets.GITHUB_TOKEN }}
          token: ${{ secrets.RELEASE_BOT_TOKEN || secrets.GITHUB_TOKEN }}

      - name: Resolve version and tag
        id: meta
        shell: bash
        run: |
          set -euo pipefail
          source ./scripts/cli_version.sh
          if [[ -n "${{ inputs.version_override || '' }}" ]]; then
            VERSION="${{ inputs.version_override }}"
            # validate via same helper: write temp toml
            tmp="$(mktemp)"
            printf 'version = "%s"\n' "$VERSION" >"$tmp"
            VERSION="$(cli_version_from_cargo_toml "$tmp")"
            rm -f "$tmp"
          else
            VERSION="$(cli_version_from_cargo_toml crates/queria-cli/Cargo.toml)"
          fi
          TAG="$(cli_tag_for_version "$VERSION")"
          echo "version=$VERSION" >> "$GITHUB_OUTPUT"
          echo "tag=$TAG" >> "$GITHUB_OUTPUT"
          echo "sha=${GITHUB_SHA}" >> "$GITHUB_OUTPUT"
          echo "Resolved VERSION=$VERSION TAG=$TAG SHA=$GITHUB_SHA"

      - name: Check remote tag
        id: exists
        shell: bash
        run: |
          set -euo pipefail
          TAG="${{ steps.meta.outputs.tag }}"
          # ls-remote prints "sha\trefs/tags/..." or empty
          line="$(git ls-remote --tags origin "refs/tags/${TAG}" | head -n1 || true)"
          if [[ -n "$line" ]]; then
            remote_sha="$(printf '%s' "$line" | awk '{print $1}')"
            # annotated tags may resolve to tag object; peel if possible
            echo "tag_exists=true" >> "$GITHUB_OUTPUT"
            echo "remote_sha=$remote_sha" >> "$GITHUB_OUTPUT"
            echo "Remote tag ${TAG} already exists ($remote_sha)"
          else
            echo "tag_exists=false" >> "$GITHUB_OUTPUT"
            echo "remote_sha=" >> "$GITHUB_OUTPUT"
            echo "Remote tag ${TAG} missing — will create"
          fi

      - name: Create and push tag
        if: steps.exists.outputs.tag_exists != 'true'
        shell: bash
        env:
          DRY_RUN: ${{ inputs.dry_run || 'false' }}
        run: |
          set -euo pipefail
          TAG="${{ steps.meta.outputs.tag }}"
          VERSION="${{ steps.meta.outputs.version }}"
          git config user.name "github-actions[bot]"
          git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
          # Tag exactly the checkout commit (push SHA on main)
          git tag -a "$TAG" -m "queria-cli ${VERSION}" "${{ steps.meta.outputs.sha }}"
          if [[ "$DRY_RUN" == "true" ]]; then
            echo "DRY_RUN=true — not pushing $TAG"
            git show "$TAG" --no-patch
            exit 0
          fi
          set +e
          git push origin "refs/tags/${TAG}"
          rc=$?
          set -e
          if [[ $rc -ne 0 ]]; then
            # Race: another run may have created it
            line="$(git ls-remote --tags origin "refs/tags/${TAG}" | head -n1 || true)"
            if [[ -n "$line" ]]; then
              echo "Tag push raced; remote tag now exists — treating as success"
              exit 0
            fi
            echo "Tag push failed (rc=$rc). Check contents:write / RELEASE_BOT_TOKEN." >&2
            exit "$rc"
          fi
          echo "Pushed $TAG → ${{ steps.meta.outputs.sha }}"

      - name: Skip (tag exists)
        if: steps.exists.outputs.tag_exists == 'true'
        run: echo "Tag ${{ steps.meta.outputs.tag }} already exists — no-op success"
```

- [ ] **Step 2: Validate YAML locally**

```bash
# if actionlint installed:
# actionlint .github/workflows/cli-detect-and-tag.yml
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/cli-detect-and-tag.yml'))"
# or: ruby -ryaml -e "YAML.load_file('.github/workflows/cli-detect-and-tag.yml')"
```

Expected: no parse error.

- [ ] **Step 3: Dry-run logic offline**

```bash
source scripts/cli_version.sh
V=$(cli_version_from_cargo_toml crates/queria-cli/Cargo.toml)
T=$(cli_tag_for_version "$V")
git ls-remote --tags origin "refs/tags/${T}" | head -n1
# If line non-empty for current version (e.g. cli-v0.3.3), Stage 1 would no-op — correct.
```

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/cli-detect-and-tag.yml
git commit -m "ci: add Stage 1 CLI detect-and-tag workflow"
```

- [ ] **Step 5: Operator note (not a code step)**

After merge, confirm repo allows Actions to create tags with `GITHUB_TOKEN`, or add secret `RELEASE_BOT_TOKEN` (classic PAT / fine-grained with contents:write on backend). Document in Task 5 runbooks.

---

### Task 3: Stage 3 workflow — Homebrew formula push

**Files:**
- Create: `.github/workflows/cli-homebrew-formula.yml`
- Consumes: `scripts/generate_homebrew_formula.sh` (existing)
- Requires secret: `HOMEBREW_TAP_TOKEN` (contents:write on `nandocoeg2/homebrew-queria`)

**Interfaces:**
- Trigger: `release: types: [published]` when tag matches `cli-v*`; also `workflow_dispatch` with tag input
- Produces: updated `Formula/queria-cli.rb` on tap main (or skip if identical)

- [ ] **Step 1: Create workflow**

`.github/workflows/cli-homebrew-formula.yml`:

```yaml
# Stage 3 of queria-cli release chain.
# After GitHub Release assets exist for cli-v*, regenerate formula and push homebrew-queria main.
# Manual fallback: workflow_dispatch with tag=cli-vX.Y.Z
# Requires secret HOMEBREW_TAP_TOKEN (write access to nandocoeg2/homebrew-queria).
name: CLI Homebrew formula

on:
  release:
    types: [published]
  workflow_dispatch:
    inputs:
      tag:
        description: "cli-vX.Y.Z (empty with release event uses event tag; for dispatch required or defaults to empty fail)"
        required: false
        default: ""

concurrency:
  group: cli-homebrew-formula
  cancel-in-progress: false

permissions:
  contents: read

jobs:
  formula:
    runs-on: ubuntu-latest
    # Only cli-v* releases
    if: >-
      (github.event_name == 'release' && startsWith(github.event.release.tag_name, 'cli-v')) ||
      (github.event_name == 'workflow_dispatch')
    steps:
      - name: Resolve tag
        id: meta
        shell: bash
        run: |
          set -euo pipefail
          if [[ "${{ github.event_name }}" == "release" ]]; then
            TAG="${{ github.event.release.tag_name }}"
          else
            TAG="${{ inputs.tag }}"
          fi
          if [[ -z "$TAG" || "$TAG" != cli-v* ]]; then
            echo "Need tag matching cli-v* (got: '${TAG:-empty}')" >&2
            exit 1
          fi
          VERSION="${TAG#cli-v}"
          echo "tag=$TAG" >> "$GITHUB_OUTPUT"
          echo "version=$VERSION" >> "$GITHUB_OUTPUT"
          echo "Using TAG=$TAG"

      - name: Checkout queria-backend
        uses: actions/checkout@v4
        with:
          path: queria-backend

      - name: Checkout homebrew-queria
        uses: actions/checkout@v4
        with:
          repository: nandocoeg2/homebrew-queria
          token: ${{ secrets.HOMEBREW_TAP_TOKEN }}
          path: homebrew-queria

      - name: Generate formula
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          # Same repo release assets; GITHUB_TOKEN is enough when workflow runs on queria-backend
        working-directory: queria-backend
        run: |
          set -euo pipefail
          chmod +x ./scripts/generate_homebrew_formula.sh
          ./scripts/generate_homebrew_formula.sh "${{ steps.meta.outputs.tag }}" \
            --out "${GITHUB_WORKSPACE}/homebrew-queria/Formula/queria-cli.rb"

      - name: Commit and push tap
        working-directory: homebrew-queria
        env:
          VERSION: ${{ steps.meta.outputs.version }}
          TAG: ${{ steps.meta.outputs.tag }}
        run: |
          set -euo pipefail
          if [[ -z "${{ secrets.HOMEBREW_TAP_TOKEN }}" ]]; then
            echo "HOMEBREW_TAP_TOKEN secret is not set" >&2
            exit 1
          fi
          git config user.name "github-actions[bot]"
          git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
          git add Formula/queria-cli.rb
          if git diff --cached --quiet; then
            echo "Formula already up to date for ${TAG} — skip push"
            exit 0
          fi
          # Safety: formula must reference this version
          if ! grep -q "version \"${VERSION}\"" Formula/queria-cli.rb; then
            echo "Generated formula missing version ${VERSION}" >&2
            head -40 Formula/queria-cli.rb >&2
            exit 1
          fi
          if ! grep -q "api.github.com/repos/nandocoeg2/queria-backend/releases/assets/" Formula/queria-cli.rb; then
            echo "WARN: formula may not use private API asset URLs" >&2
          fi
          git commit -m "chore: queria-cli ${VERSION} formula (private API assets)"
          git push origin HEAD:main
          echo "Pushed homebrew-queria formula for ${TAG}"
```

- [ ] **Step 2: Validate YAML**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/cli-homebrew-formula.yml'))"
```

- [ ] **Step 3: Local generator smoke (assets already live)**

```bash
export GH_TOKEN="$(gh auth token)"
./scripts/generate_homebrew_formula.sh cli-v0.3.3 --out /tmp/queria-cli-0.3.3.rb
head -30 /tmp/queria-cli-0.3.3.rb
# Expect: version "0.3.3" and api.github.com/.../releases/assets/...
```

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/cli-homebrew-formula.yml
git commit -m "ci: add Stage 3 CLI Homebrew formula auto-push"
```

- [ ] **Step 5: Configure secret (operator / cannot fully automate)**

In GitHub UI for `nandocoeg2/queria-backend`:

1. Create fine-grained PAT (or classic) with **Contents: Read and write** on `nandocoeg2/homebrew-queria` only.
2. Add Actions secret name exactly: `HOMEBREW_TAP_TOKEN`.
3. Dispatch dry: Actions → **CLI Homebrew formula** → Run workflow → tag `cli-v0.3.3` (idempotent if formula already matches).

Do not print the token in logs or commits.

---

### Task 4: Stage 2 pointer comment only

**Files:**
- Modify: `.github/workflows/release-cli.yml` (header comments only)

- [ ] **Step 1: Prepend stage pointer to workflow header**

Replace the top comment block so it reads (keep existing runner notes):

```yaml
# Stage 2 of queria-cli release chain:
#   Stage 1: .github/workflows/cli-detect-and-tag.yml (main version → tag cli-v*)
#   Stage 2: this workflow (tag push → multi-arch build → GitHub Release)
#   Stage 3: .github/workflows/cli-homebrew-formula.yml (release published → formula)
# Trigger: push tag cli-v* (e.g. cli-v0.1.0) or workflow_dispatch.
# Manual tag push still works without Stage 1.
#
# Note: do NOT use macos-13 — GitHub retired that runner image (jobs queue forever).
# x86_64 macOS builds on macos-14 via Rust target (Apple Silicon host).
```

Do **not** change `on:`, matrix, or publish steps.

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release-cli.yml
git commit -m "docs(ci): note release-cli is Stage 2 of CLI release chain"
```

---

### Task 5: Runbooks + HANDOFF documentation

**Files:**
- Modify: `docs/runbooks/deployment.md` (queria-cli Release section)
- Modify: `docs/runbooks/queria-cli-homebrew.md`
- Modify: `docs/HANDOFF.md` (short CURRENT residual)
- Modify: design header status note when all Tasks 1–5 merged and smoke-tested

**Requirements (exact content expectations):**

- [ ] **Step 1: Update deployment.md happy path**

In the § *Cut a new CLI release* / operator checklist area, replace tag-only happy path with:

```markdown
#### Cut a new CLI release (automated happy path)

```bash
# 1) Bump version only when ready to ship
# edit crates/queria-cli/Cargo.toml version → X.Y.Z
# cargo build -p queria-cli   # refresh Cargo.lock package version
git add crates/queria-cli/Cargo.toml Cargo.lock
git commit -m "chore(cli): bump queria-cli to X.Y.Z"
git push origin main
# 2) Actions chain:
#    CLI detect-and-tag → tag cli-vX.Y.Z
#    Release queria-cli → assets
#    CLI Homebrew formula → homebrew-queria main
# 3) Laptop install (still manual per machine; private backend):
export HOMEBREW_GITHUB_API_TOKEN=$(gh auth token)
brew update && brew reinstall nandocoeg2/queria/queria-cli
queria-cli --version
```

**Secrets (queria-backend Actions):**

| Secret | Purpose |
|--------|---------|
| `GITHUB_TOKEN` (default) | Stage 1 tag push (if allowed); Stage 3 download Release assets |
| `RELEASE_BOT_TOKEN` (optional) | Tag push if default token cannot create tags |
| `HOMEBREW_TAP_TOKEN` | Stage 3 push to `nandocoeg2/homebrew-queria` |

**Unstick:**

| Symptom | Action |
|---------|--------|
| Version bump, no tag | Check **CLI detect-and-tag** logs; dispatch with dry_run=false; or manual `git tag -a cli-v…` on bump commit |
| Tag, no assets | **Release queria-cli** dispatch with tag input (existing checklist) |
| Assets, formula stale | **CLI Homebrew formula** dispatch with `tag=cli-v…` |
```

Keep the existing cancel stuck / required assets tables.

Update `Last verified` date on the runbook header.

- [ ] **Step 2: Update queria-cli-homebrew.md**

Change architecture note from “backend does not auto-update Brew” to:

```markdown
## Automation

After `cli-v*` GitHub Release is published, workflow **CLI Homebrew formula** regenerates
`Formula/queria-cli.rb` and **direct-pushes** `nandocoeg2/homebrew-queria` `main`.

Manual path (fallback if CI secret missing):

```bash
export GH_TOKEN=$(gh auth token)
./scripts/generate_homebrew_formula.sh cli-vX.Y.Z
cd ../homebrew-queria && git add Formula/queria-cli.rb && git commit -m "…" && git push
```

CI secret on **queria-backend**: `HOMEBREW_TAP_TOKEN` (write to tap only).
Laptop private install still needs `HOMEBREW_GITHUB_API_TOKEN` for asset download at brew time.
```

Keep generator behavior table and archive layout.

- [ ] **Step 3: HANDOFF residual (short)**

Add under CURRENT / residual (do not paste full design):

```markdown
### CLI release automation (2026-07-23)

- Chain: **detect-and-tag** (Cargo.toml version) → **Release queria-cli** (unchanged matrix) → **Homebrew formula** direct-push.
- Design: `docs/superpowers/specs/2026-07-23-cli-release-automation-design.md`
- Residual: per-laptop `brew reinstall` + `HOMEBREW_GITHUB_API_TOKEN` while backend private; accidental version bumps still release — review Cargo.toml carefully.
```

- [ ] **Step 4: Spec status line**

In design doc header after e2e acceptance:

```markdown
> Status: REFERENCE → implemented on main (see plan Tasks 1–6); automation live when secrets set
```

(Only after Task 6 smoke succeeds; until then leave REFERENCE and add “plan: docs/superpowers/plans/2026-07-23-cli-release-automation.md”.)

- [ ] **Step 5: Commit docs**

```bash
git add docs/runbooks/deployment.md docs/runbooks/queria-cli-homebrew.md docs/HANDOFF.md docs/superpowers/specs/2026-07-23-cli-release-automation-design.md
git commit -m "docs: operator path for automated CLI release + Homebrew"
```

---

### Task 6: Merge, secrets, e2e smoke (operator + CI)

**Files:** none new (verification)

**Preconditions:** Tasks 1–5 on a branch merged to `main` (or implement straight on feature branch then PR).

- [ ] **Step 1: Merge to main**

```bash
# via PR or direct push per repo policy
git push -u origin HEAD
# open PR if required; merge to main
```

- [ ] **Step 2: Confirm secrets**

- [ ] `HOMEBREW_TAP_TOKEN` present on `queria-backend`
- [ ] Tag create works with default token **or** `RELEASE_BOT_TOKEN` set
- [ ] Tap repo allows bot push to `main`

- [ ] **Step 3: Idempotent formula re-run**

```text
Actions → CLI Homebrew formula → Run workflow → tag: cli-v0.3.3
```

Expected: green; log either “skip push” or commit with same version SHAs.

- [ ] **Step 4: Real e2e (next patch)**

Only when ready to ship a real binary:

```bash
# on main
# bump crates/queria-cli/Cargo.toml 0.3.3 → 0.3.4 (or next)
# cargo build -p queria-cli
git add crates/queria-cli/Cargo.toml Cargo.lock
git commit -m "chore(cli): bump queria-cli to 0.3.4"
git push origin main
```

Watch:

1. **CLI detect-and-tag** → creates `cli-v0.3.4`
2. **Release queria-cli** → green assets
3. **CLI Homebrew formula** → tap at 0.3.4

```bash
export HOMEBREW_GITHUB_API_TOKEN=$(gh auth token)
brew update && brew reinstall nandocoeg2/queria/queria-cli
queria-cli --version   # 0.3.4
```

- [ ] **Step 5: No false release check**

Push a doc-only or feature commit without version change (after Step 4). Confirm **CLI detect-and-tag** does not run (paths filter) or no-ops (tag exists). No new release.

- [ ] **Step 6: Acceptance checklist (spec §10)**

- [ ] Version bump alone → tag without human tag command  
- [ ] Tag → green Release with required assets  
- [ ] Formula on tap main matches version + private API URLs  
- [ ] Second same-version push harmless  
- [ ] Feature-only main push no new CLI release  
- [ ] Unstick paths still documented  
- [ ] Runbooks + HANDOFF updated  

- [ ] **Step 7: Final docs commit if status lines need “implemented”**

```bash
git commit -m "docs: mark CLI release automation live"
```

---

## Plan self-review (vs design)

| Spec section | Plan coverage |
|--------------|---------------|
| §1 Goal full pipe | Tasks 1–3 + 6 |
| §2 Non-goals | Global Constraints; no matrix change Task 4 |
| §3 Architecture A | Three workflows; Stage 2 untouched |
| §4 Detect-and-tag | Task 2 + version helper Task 1 |
| §5 Stage 2 unchanged | Task 4 comment only |
| §6 Homebrew direct push | Task 3 |
| §7 Secrets | Task 3 Step 5, Task 5 runbooks, Task 6 |
| §8 Operator happy path | Task 5 deployment.md |
| §9 Documentation | Task 5 |
| §10 Acceptance | Task 6 Steps 4–6 |
| §11 Risks | Concurrency groups, race skip, token scopes, no force-tag |
| DRY_RUN | Task 2 workflow_dispatch dry_run |

**Placeholder scan:** none intentionally left as TBD; secret creation is operator UI step called out explicitly.

**Type/name consistency:** `cli_version_from_cargo_toml`, `cli_tag_for_version`, workflows `CLI detect-and-tag` / `CLI Homebrew formula`, secret `HOMEBREW_TAP_TOKEN`, optional `RELEASE_BOT_TOKEN`.

---

## Out of scope (do not implement in this plan)

- Auto `brew reinstall` on laptops
- Homebrew PR review flow
- API/image deploy coupling
- Changing release matrix runners
- Semver policy bots
