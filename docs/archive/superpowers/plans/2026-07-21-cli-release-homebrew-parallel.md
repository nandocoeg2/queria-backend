# CLI Release + Homebrew Tap Readiness Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish parallel residual: verifiable CLI GitHub Release path, real Homebrew formula generation path, tap repo ready to publish, docs accurate for operator next actions.

**Architecture:** `queria-backend` tag `cli-v*` → Actions Release queria-cli → GitHub Release assets; operator runs `scripts/generate_homebrew_formula.sh` → push separate `homebrew-queria` tap. Push `main` never auto-brews.

**Tech Stack:** GitHub Actions, Rust queria-cli, Homebrew Formula (Ruby), bash generator, Markdown runbooks.

## Global Constraints

- Do **not** auto-publish Homebrew on every `main` push.
- CLI release only via **`cli-v*`** tag or workflow_dispatch with tag.
- Never use **`macos-13`** runners (retired).
- Formula `sha256` must come from real downloaded assets (no invented hashes).
- Placeholder formula must not be advertised as installable until SHAs are real.
- Prefer native arm runners for GHCR image build (already on main); do not regress QEMU-on-amd64.
- No secrets in commits; private download via `GH_TOKEN` / `HOMEBREW_GITHUB_API_TOKEN` env only.
- Parent workspace `queria/homebrew-queria` is **not** inside the backend git repo; generator writes there by default.
- Work only on an isolated worktree branch for code/docs changes under `queria/backend`; tap files live outside backend git (sibling dir).
- Do not force-push `main`; integrate via merge when tasks complete.
- Daily 3-step onboard docs already shipped — do not rewrite that path.

---

## File map

| Area | Path |
|---|---|
| CLI release workflow | `.github/workflows/release-cli.yml` |
| Formula generator | `scripts/generate_homebrew_formula.sh` |
| Homebrew runbook | `docs/runbooks/queria-cli-homebrew.md` |
| Deploy/CI notes | `docs/runbooks/deployment.md` |
| Tap scaffold (sibling) | `../homebrew-queria/` (workspace) |

---

### Task 1: Verify CLI release pipeline + operator smoke checklist in docs

**Files:**
- Modify: `docs/runbooks/deployment.md` (release operator checklist if gaps)
- Modify: `docs/runbooks/queria-cli-homebrew.md` if release preconditions incomplete
- Modify: `.github/workflows/release-cli.yml` only if a **concrete** bug remains (macos-13 gone, optional arm, native darwin)

**Steps:**

- [ ] **Step 1:** Confirm `release-cli.yml` matrix has no `macos-13`; both Darwin targets on `macos-14`; linux arm optional.
- [ ] **Step 2:** Confirm archive layout matches formula: top-level dir contains binary named `queria-cli`.
- [ ] **Step 3:** Add/verify operator checklist: cancel stuck run → re-tag or workflow_dispatch → wait for assets → then generate formula.
- [ ] **Step 4:** Run static validation (yamllint not required): read workflow for syntax issues; note that live Actions need human/token for private repo verification.
- [ ] **Step 5:** Commit docs/workflow fixes if any.

**Done when:** Operator can follow one page to unstick/get Release assets without reading chat history.

---

### Task 2: Harden `generate_homebrew_formula.sh` for real + private assets

**Files:**
- Modify: `scripts/generate_homebrew_formula.sh`
- Modify: `docs/runbooks/queria-cli-homebrew.md` (usage edges)
- Optional unit-less: `bash -n` the script

**Requirements:**

- [ ] Fail with clear exit if required assets missing (darwin arm + linux x86_64).
- [ ] Support `GH_TOKEN` / `GITHUB_TOKEN` / `HOMEBREW_GITHUB_API_TOKEN` for private download.
- [ ] Write formula with correct nested-archive install (if brew needs `prefix` / strip path).
- [ ] `--out` path works; default out to sibling `homebrew-queria/Formula/queria-cli.rb`.
- [ ] `bash -n` passes.
- [ ] Commit under backend repo.

**Homebrew install note:** if tarball root is `queria-cli-<triple>/queria-cli`, Formula should `bin.install "queria-cli"` when brew chdirs into single top-level directory. Confirm generator docs state this.

**Done when:** Script is production-ready; with live assets + token it produces installable formula.

---

### Task 3: Tap scaffold ready for GitHub publish

**Files (sibling, not backend git):**
- `queria/homebrew-queria/README.md`
- `queria/homebrew-queria/Formula/queria-cli.rb`
- Add: `queria/homebrew-queria/.gitignore` if useful
- Optional: init git in that folder only if not present; **do not** force create remote without credentials.

**Backend docs touch (if needed):**
- Point parent install to runbook only.

**Steps:**

- [ ] Ensure README install/maintainers steps match Task 2 script.
- [ ] Formula placeholder clearly marked NOT READY (odie or comments) so brew fails loudly with zeros.
- [ ] Prefer: `ode` / raise if sha256 is all zeros so users don't install broken bottle.
- [ ] Backend commit only for docs that reference the scaffold; tap files stay outside backend remote.

**Done when:** Human can `git init` + push `homebrew-queria` after generating real formula in one readme flow.

---

### Task 4: Residual accuracy pass (deployment + HANDOFF + onboarding install)

**Files:**
- `docs/runbooks/deployment.md`
- `docs/HANDOFF.md`
- `docs/runbooks/onboarding.md` install section
- `docs/README.md` index line if missing homebrew

**Steps:**

- [ ] One residual list: Release not verified from unauth API; first arm64 deploy seeds buildcache; Brew after formula SHA real; Daily onboard independent.
- [ ] No claim that Brew works today if SHAs still zero.
- [ ] Commit.

**Done when:** Docs do not oversell Brew/auto-release.

---

## Out of scope

- Auto PR to homebrew-queria from CI
- homebrew-core
- Daily token mint UI changes
- Host Path B rsync deploy unless Release requires it
- `gh auth login` on behalf of user

## Test plan

| Check | How |
|---|---|
| Workflow YAML sensible | Manual read; matrix runners |
| Generator | `bash -n scripts/generate_homebrew_formula.sh` |
| Live assets | Operator Actions UI / token curl (subagent notes BLOCKED if private 404) |
| Formula Ruby | Placeholder has sentinel zeros or `odie` |

## Progress ledger

Track in `.superpowers/sdd/progress.md` for this plan only (new entries after “Plan: 2026-07-21-cli-release-homebrew-parallel”).
