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

```bash
# After reviewing workspace scaffold:
cd queria/homebrew-queria   # sibling of queria/backend in the monorepo workspace
git init
git add README.md Formula
git commit -m "chore: initial queria-cli homebrew tap"
# create empty repo nandocoeg2/homebrew-queria on GitHub, then:
git branch -M main
git remote add origin git@github.com:nandocoeg2/homebrew-queria.git
git push -u origin main
```

Do **not** put this under `queria-backend` as the only copy long-term: Homebrew expects repo name `homebrew-queria`.

## After every CLI release (operator checklist)

```bash
cd queria/backend
# private release:
# export GH_TOKEN=ghp_…   # or HOMEBREW_GITHUB_API_TOKEN

./scripts/generate_homebrew_formula.sh cli-v0.1.0
# default writes ../homebrew-queria/Formula/queria-cli.rb

cd ../homebrew-queria
git add Formula/queria-cli.rb
git commit -m "queria-cli 0.1.0"
git push origin main
```

Smoke:

```bash
# private assets:
# export HOMEBREW_GITHUB_API_TOKEN=ghp_…
brew tap nandocoeg2/queria
brew install queria-cli
# or upgrade:
# brew update && brew upgrade queria-cli
queria-cli index-here --help
```

## User install

### Recommended (when formula SHAs real)

```bash
brew install nandocoeg2/queria/queria-cli
```

Private backend Releases:

```bash
export HOMEBREW_GITHUB_API_TOKEN='ghp_…'  # read access to queria-backend
brew install nandocoeg2/queria/queria-cli
```

### Fallback (no Brew)

curl + tar from GitHub Releases — see [`onboarding.md`](./onboarding.md).

## Formula contract

| Field | Rule |
|---|---|
| `version` | Semver without `cli-v` prefix (`0.1.0` from `cli-v0.1.0`) |
| `url` | Exact asset name from release-cli workflow |
| `sha256` | Of the **tarball**, not the binary inside |
| `install` | `bin.install "queria-cli"` (archive root contains that file) |
| Linux arm missing | Formula may `odie` on arm Linux for that version |

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
