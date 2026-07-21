# queria-cli Homebrew tap

> Status: CURRENT (process); formula SHAs only valid after a live `cli-v*` Release  
> Last verified: 2026-07-21  
> Runtime: [`../HANDOFF.md`](../HANDOFF.md)  
> CLI install overview: [`onboarding.md`](./onboarding.md) § Install `queria-cli`  
> Deploy / release triggers: [`deployment.md`](./deployment.md)

## Goal

Let laptop users install `queria-cli` without Rust:

```bash
brew install nandocoeg2/queria/queria-cli
```

## Architecture

```text
queria-backend tag cli-v*
  → Actions "Release queria-cli"
  → GitHub Release assets (tar.gz + sha256)

homebrew-queria (separate GitHub repo)
  → Formula/queria-cli.rb (url + sha256 per arch)
  → brew tap nandocoeg2/queria
```

| Repo | Role |
|---|---|
| `nandocoeg2/queria-backend` | Builds binaries; **does not** auto-update Brew |
| `nandocoeg2/homebrew-queria` | Homebrew tap (`brew tap nandocoeg2/queria`) |
| Workspace path (scaffold) | `queria/homebrew-queria/` (publish as that GitHub repo) |

Push **`queria-backend` `main` never** updates Homebrew. Only:

1. CLI Release assets exist, then  
2. Formula regenerated + pushed to **homebrew-queria**.

## Preconditions

Release first — do **not** generate a formula until assets exist. Full cut/unstick sequence (cancel stuck run → re-tag or `workflow_dispatch` → wait for assets → **then** this page): [`deployment.md`](./deployment.md) § *queria-cli Release — operator checklist*.

- [ ] Actions run **Release queria-cli** green for tag `cli-vX.Y.Z` (or green after dispatch with tag input)
- [ ] Required assets present: Darwin aarch64 + Linux x86_64 (Darwin x86_64 expected; Linux arm optional)
- [ ] Assets downloadable — **public** HTTP 200, or **private** with token / logged-in UI (unauth 404 is not proof of missing assets)
- [ ] Archive layout: top-level dir `queria-cli-<triple>/` contains binary named **`queria-cli`** (Homebrew `bin.install "queria-cli"`)

If the backend repo is **private**, downloads need auth (see below). Live private verification without `GH_TOKEN` / UI is **BLOCKED** — confirm assets in Releases UI before generating formula.

## One-time: create the tap on GitHub

Workspace scaffold: `queria/homebrew-queria/` (sibling of `queria/backend`; **not** in the backend git remote). Full maintainer flow lives in that repo’s **README**. Summary:

```bash
# After reviewing workspace scaffold:
cd queria/homebrew-queria   # sibling of queria/backend in the monorepo workspace
git init                    # if not already a repo; local only is fine
git add README.md Formula .gitignore
git commit -m "chore: initial queria-cli homebrew tap scaffold"
# create empty repo nandocoeg2/homebrew-queria on GitHub (do not force without credentials), then:
git branch -M main
git remote add origin git@github.com:nandocoeg2/homebrew-queria.git
git push -u origin main
```

Scaffold `Formula/queria-cli.rb` is a **NOT READY** placeholder: every platform branch calls Homebrew `odie` so `brew install` fails loudly until the generator rewrites real `url`/`sha256`. Do **not** put this under `queria-backend` as the only copy long-term: Homebrew expects repo name `homebrew-queria`.

## After every CLI release (operator checklist)

```bash
cd queria/backend
# private release (any one of these; script precedence: GH_TOKEN → GITHUB_TOKEN → HOMEBREW_GITHUB_API_TOKEN):
# export GH_TOKEN=ghp_…
# export GITHUB_TOKEN=ghp_…
# export HOMEBREW_GITHUB_API_TOKEN=ghp_…

./scripts/generate_homebrew_formula.sh cli-v0.1.0
# default OUT: first existing homebrew-queria (main: ../ ; worktree under .worktrees: ../../../ ), else ../homebrew-queria/…; override with --out
# custom path:
# ./scripts/generate_homebrew_formula.sh cli-v0.1.0 --out /tmp/queria-cli.rb

cd ../homebrew-queria
git add Formula/queria-cli.rb
git commit -m "queria-cli 0.1.0"
git push origin main
```

### Generator behavior (usage edges)

| Case | Behavior |
|---|---|
| Required assets missing / HTTP 404 | **Exit 1**, lists missing files; **does not write** a partial formula or invent sha256 |
| No token on private Release | **Exit 1** with hint to set `GH_TOKEN` / `GITHUB_TOKEN` / `HOMEBREW_GITHUB_API_TOKEN` |
| Token 401/403 | **Exit 1** with auth/scopes hint |
| Linux arm64 asset missing | Warning only; formula `odie` on Linux arm for that version |
| Tag not `cli-v*` | Exit 2 usage error |
| `--out` without path | Exit 2 usage error |

Ship-gate / generator hard-required downloads (all three **Exit 1** if missing): `queria-cli-aarch64-apple-darwin.tar.gz`, `queria-cli-x86_64-apple-darwin.tar.gz`, `queria-cli-x86_64-unknown-linux-gnu.tar.gz` (workflow builds Darwin x86_64 non-optional). Linux aarch64 optional.

**Do not** re-run with inventing zeros if download fails — fix the Release/token, then re-run.

Smoke:

```bash
# private assets (brew download uses HOMEBREW_GITHUB_API_TOKEN):
# export HOMEBREW_GITHUB_API_TOKEN=ghp_…
brew tap nandocoeg2/queria
brew install queria-cli
# or upgrade:
# brew update && brew upgrade queria-cli
queria-cli index-here --help
```

## User install

### Recommended (when formula SHAs real — not while scaffold placeholder remains)

Scaffold / zero-sha formulas are **not** installable. After generator + tap push:

```bash
brew install nandocoeg2/queria/queria-cli
```

Private backend Releases:

```bash
export HOMEBREW_GITHUB_API_TOKEN='ghp_…'  # read access to queria-backend
brew install nandocoeg2/queria/queria-cli
```

Parent install overview points here only: [`onboarding.md`](./onboarding.md) § Install `queria-cli`.

### Fallback (no Brew)

curl + tar from GitHub Releases — see [`onboarding.md`](./onboarding.md).

## Formula contract

| Field | Rule |
|---|---|
| `version` | Semver without `cli-v` prefix (`0.1.0` from `cli-v0.1.0`) |
| `url` | Exact asset name from release-cli workflow |
| `sha256` | Of the **tarball**, not the binary inside (generator only writes real downloads) |
| `install` | `bin.install "queria-cli"` — release tarball is `queria-cli-<triple>/queria-cli`; Homebrew auto-extracts and **chdirs into the single top-level directory**, so the binary is at the stage root (not a nested path) |
| Linux arm missing | Formula `odie` on arm Linux for that version |

Generator: [`../../scripts/generate_homebrew_formula.sh`](../../scripts/generate_homebrew_formula.sh).

## Non-goals

| Out | Why |
|---|---|
| homebrew-core PR | Heavy review; private binary path awkward |
| Auto PR from queria-backend CI | Nice later; v1 is script + human push tap |
| `.dmg` / notarized Mac app | CLI only |
| Publish on every backend `main` push | Only `cli-v*` + formula bump |

## Automation later (optional)

When stable: workflow_dispatch after CLI release that checks out both repos, runs `generate_homebrew_formula.sh`, opens PR on `homebrew-queria`. Not required for first ship.

## Troubleshooting

| Symptom | Check |
|---|---|
| curl/brew 404 on asset | Release not published; still private without token; wrong tag |
| Checksum mismatch | Formula stale; re-run generator after new asset build |
| Waiting on macos-13 in CLI CI | Deprecated runner; use fixed `release-cli.yml` on macos-14 |
| `brew` wants to build from source | Formula should be bottle-less binary only; ensure `url` points at tar.gz with binary |
