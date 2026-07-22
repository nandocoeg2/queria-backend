#!/usr/bin/env bash
# Generate / refresh homebrew-queria Formula/queria-cli.rb from a live GitHub Release.
#
# Usage:
#   ./scripts/generate_homebrew_formula.sh cli-v0.1.0
#   ./scripts/generate_homebrew_formula.sh cli-v0.1.0 --out ../homebrew-queria/Formula/queria-cli.rb
#   GH_TOKEN=… ./scripts/generate_homebrew_formula.sh cli-v0.1.0   # private repo
#
# Required assets (hard fail if missing / not downloadable):
#   queria-cli-aarch64-apple-darwin.tar.gz
#   queria-cli-x86_64-unknown-linux-gnu.tar.gz
# Expected (hard fail if missing — full macOS Intel coverage):
#   queria-cli-x86_64-apple-darwin.tar.gz
# Optional:
#   queria-cli-aarch64-unknown-linux-gnu.tar.gz
#
# Private Releases: set GH_TOKEN, GITHUB_TOKEN, or HOMEBREW_GITHUB_API_TOKEN.
# Does not invent sha256 — only hashes real downloads. Does not commit or push the tap.
# Compatible with macOS /bin/bash 3.2 (no associative arrays).
#
# Archive layout (release-cli.yml): tarball root is queria-cli-<triple>/queria-cli.
# Homebrew chdirs into the single top-level directory, so Formula uses:
#   bin.install "queria-cli"

set -euo pipefail

REPO="${QUERIA_BACKEND_REPO:-nandocoeg2/queria-backend}"
TAG="${1:-}"
OUT=""
if [[ $# -gt 0 ]]; then
  shift
fi
while [[ $# -gt 0 ]]; do
  case "$1" in
    --out)
      if [[ -z "${2:-}" ]]; then
        echo "usage: $0 cli-vX.Y.Z [--out path/to/queria-cli.rb]" >&2
        exit 2
      fi
      OUT="$2"
      shift 2
      ;;
    *)
      echo "unknown arg: $1" >&2
      echo "usage: $0 cli-vX.Y.Z [--out path/to/queria-cli.rb]" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$TAG" || "$TAG" != cli-v* ]]; then
  echo "usage: $0 cli-vX.Y.Z [--out path/to/queria-cli.rb]" >&2
  exit 2
fi

VERSION="${TAG#cli-v}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# Resolve homebrew-queria next to backend (queria/homebrew-queria):
#   main clone ROOT=…/backend → ../homebrew-queria
#   worktree ROOT=…/backend/.worktrees/<name> → ../../../homebrew-queria
# Prefer first existing tap dir; else default create path ROOT/../homebrew-queria.
DEFAULT_OUT=""
for candidate in \
  "$ROOT/../homebrew-queria" \
  "$ROOT/../../../homebrew-queria"
do
  if [[ -d "$candidate" ]]; then
    DEFAULT_OUT="$(cd "$candidate" && pwd)/Formula/queria-cli.rb"
    break
  fi
done
if [[ -z "$DEFAULT_OUT" ]]; then
  DEFAULT_OUT="$(cd "$ROOT/.." && pwd)/homebrew-queria/Formula/queria-cli.rb"
fi
OUT="${OUT:-$DEFAULT_OUT}"

# Token precedence: GH_TOKEN → GITHUB_TOKEN → HOMEBREW_GITHUB_API_TOKEN
TOKEN=""
if [[ -n "${GH_TOKEN:-}" ]]; then
  TOKEN="$GH_TOKEN"
elif [[ -n "${GITHUB_TOKEN:-}" ]]; then
  TOKEN="$GITHUB_TOKEN"
elif [[ -n "${HOMEBREW_GITHUB_API_TOKEN:-}" ]]; then
  TOKEN="$HOMEBREW_GITHUB_API_TOKEN"
fi

if [[ -n "$TOKEN" ]]; then
  echo "auth: using GitHub token (API asset download for private releases)" >&2
else
  echo "auth: no GH_TOKEN/GITHUB_TOKEN/HOMEBREW_GITHUB_API_TOKEN — public browser downloads only" >&2
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

# Private GitHub Releases: browser URLs under /releases/download/ return 404 even with
# Authorization. Prefer the Releases API asset endpoint when a token is present.
# API: GET /repos/{owner}/{repo}/releases/tags/{tag} → assets[].id
# then GET /repos/{owner}/{repo}/releases/assets/{id} with Accept: application/octet-stream
RELEASE_JSON=""
resolve_asset_id() {
  local asset="$1"
  if [[ -z "$TOKEN" ]]; then
    return 1
  fi
  if [[ -z "$RELEASE_JSON" ]]; then
    local meta_code
    set +e
    RELEASE_JSON="$(curl -sS -L -w '\n%{http_code}' \
      -H "Authorization: Bearer ${TOKEN}" \
      -H "Accept: application/vnd.github+json" \
      -H "X-GitHub-Api-Version: 2022-11-28" \
      "https://api.github.com/repos/${REPO}/releases/tags/${TAG}" 2>"$tmpdir/release.meta.err")"
    local meta_rc=$?
    set -e
    meta_code="$(printf '%s' "$RELEASE_JSON" | tail -n1)"
    RELEASE_JSON="$(printf '%s' "$RELEASE_JSON" | sed '$d')"
    if [[ $meta_rc -ne 0 || "$meta_code" != "200" || -z "$RELEASE_JSON" ]]; then
      echo "FAIL: cannot load release metadata for ${TAG} (HTTP ${meta_code:-?} curl_rc=${meta_rc})" >&2
      if [[ -s "$tmpdir/release.meta.err" ]]; then
        echo "curl: $(cat "$tmpdir/release.meta.err")" >&2
      fi
      RELEASE_JSON=""
      return 1
    fi
  fi
  if command -v python3 >/dev/null 2>&1; then
    ASSET_NAME="$asset" python3 -c 'import json,os,sys; name=os.environ["ASSET_NAME"]; data=json.load(sys.stdin); ids=[a["id"] for a in data.get("assets",[]) if a.get("name")==name]; print(ids[0] if ids else "")' <<<"$RELEASE_JSON"
  elif command -v jq >/dev/null 2>&1; then
    jq -r --arg n "$asset" '.assets[]? | select(.name==$n) | .id' <<<"$RELEASE_JSON" | head -n1
  else
    echo "FAIL: need python3 or jq to parse GitHub release metadata" >&2
    return 1
  fi
}

# Download asset and print sha256 of the **tarball**. Never invent hashes.
# Returns 0 and prints hash on stdout; errors go to stderr.
# Also writes asset id to $tmpdir/asset_id.<key> (survives command-substitution subshells).
fetch_sha() {
  local asset="$1"
  local dest="$tmpdir/$asset"
  local http_code=""
  local curl_rc=0
  local url=""
  local key
  key="$(printf '%s' "$asset" | tr '.-' '__')"
  # Capture body to file and HTTP status; do not fail the whole script on curl error alone.
  set +e
  if [[ -n "$TOKEN" ]]; then
    local asset_id
    asset_id="$(resolve_asset_id "$asset")"
    curl_rc=$?
    if [[ $curl_rc -eq 0 && -n "$asset_id" ]]; then
      # Persist API asset id for formula_url_block (not env: fetch_sha runs in $() subshell).
      printf '%s' "$asset_id" >"$tmpdir/asset_id.${key}"
      url="https://api.github.com/repos/${REPO}/releases/assets/${asset_id}"
      echo "fetch $url ($asset)" >&2
      http_code="$(curl -sS -L -w '%{http_code}' -o "$dest" \
        -H "Authorization: Bearer ${TOKEN}" \
        -H "Accept: application/octet-stream" \
        -H "X-GitHub-Api-Version: 2022-11-28" \
        "$url" 2>"$tmpdir/${asset}.curl.err")"
      curl_rc=$?
    else
      http_code="404"
      curl_rc=1
      echo "fetch api metadata miss for $asset" >&2
    fi
  else
    url="https://github.com/${REPO}/releases/download/${TAG}/${asset}"
    echo "fetch $url" >&2
    http_code="$(curl -sS -L -w '%{http_code}' -o "$dest" "$url" 2>"$tmpdir/${asset}.curl.err")"
    curl_rc=$?
  fi
  set -e
  if [[ $curl_rc -ne 0 || ! -s "$dest" || "$http_code" != "200" ]]; then
    local hint="is the Release published with this asset?"
    if [[ -z "$TOKEN" ]]; then
      hint="private repo/asset → set GH_TOKEN (or GITHUB_TOKEN / HOMEBREW_GITHUB_API_TOKEN); or asset not published for ${TAG}"
    elif [[ "$http_code" == "404" ]]; then
      hint="HTTP 404 — asset missing for ${TAG}, or token lacks repo read access"
    elif [[ "$http_code" == "401" || "$http_code" == "403" ]]; then
      hint="HTTP ${http_code} — token rejected or missing required scopes (repo read)"
    else
      hint="HTTP ${http_code:-?} curl_rc=${curl_rc}"
    fi
    echo "FAIL: cannot download ${asset} (${hint})" >&2
    if [[ -s "$tmpdir/${asset}.curl.err" ]]; then
      echo "curl: $(cat "$tmpdir/${asset}.curl.err")" >&2
    fi
    return 1
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$dest" | awk '{print $1}'
  else
    sha256sum "$dest" | awk '{print $1}'
  fi
}

# Formula url+headers block for one platform asset.
# Private backend: API asset URL + Authorization from HOMEBREW_GITHUB_API_TOKEN
# (browser /releases/download URLs always 404 when the repo is private).
formula_url_block() {
  local asset="$1"
  local sha="$2"
  local indent="${3:-6}"
  local pad key asset_id
  pad="$(printf "%${indent}s" "")"
  key="$(printf '%s' "$asset" | tr '.-' '__')"
  asset_id=""
  if [[ -f "$tmpdir/asset_id.${key}" ]]; then
    asset_id="$(cat "$tmpdir/asset_id.${key}")"
  fi
  if [[ -n "$asset_id" ]]; then
    # Use headers: (array). `header:` expects a single String and breaks if passed Array.
    cat <<EOF
${pad}url "https://api.github.com/repos/${REPO}/releases/assets/${asset_id}",
${pad}    headers: [
${pad}      "Accept: application/octet-stream",
${pad}      "Authorization: Bearer #{ENV.fetch("HOMEBREW_GITHUB_API_TOKEN", ENV.fetch("GH_TOKEN", ENV.fetch("GITHUB_TOKEN", "")))}",
${pad}    ]
${pad}sha256 "${sha}"
EOF
  else
    cat <<EOF
${pad}url "https://github.com/${REPO}/releases/download/${TAG}/${asset}"
${pad}sha256 "${sha}"
EOF
  fi
}

# Valid 64-char lowercase hex sha256
is_sha256() {
  local h="$1"
  [[ ${#h} -eq 64 && "$h" == "$(printf '%s' "$h" | tr -cd '0-9a-f')" ]]
}

# Ship gate (release-cli publish check): darwin arm + linux intel.
# Darwin intel is expected for full formula (workflow builds it non-optional).
need_assets="queria-cli-aarch64-apple-darwin.tar.gz queria-cli-x86_64-apple-darwin.tar.gz queria-cli-x86_64-unknown-linux-gnu.tar.gz"
optional_asset="queria-cli-aarch64-unknown-linux-gnu.tar.gz"

SHA_DARWIN_ARM=""
SHA_DARWIN_INTEL=""
SHA_LINUX_INTEL=""
SHA_LINUX_ARM=""

missing=""
for a in $need_assets; do
  set +e
  hash="$(fetch_sha "$a")"
  rc=$?
  set -e
  if [[ $rc -ne 0 ]] || ! is_sha256 "$hash"; then
    if [[ $rc -eq 0 ]]; then
      echo "FAIL: invalid sha256 for $a (got: ${hash:-empty})" >&2
    fi
    missing="${missing}${missing:+ }$a"
    continue
  fi
  case "$a" in
    queria-cli-aarch64-apple-darwin.tar.gz) SHA_DARWIN_ARM="$hash" ;;
    queria-cli-x86_64-apple-darwin.tar.gz) SHA_DARWIN_INTEL="$hash" ;;
    queria-cli-x86_64-unknown-linux-gnu.tar.gz) SHA_LINUX_INTEL="$hash" ;;
  esac
  echo "ok  $a  $hash" >&2
done

if [[ -n "$missing" ]]; then
  echo "" >&2
  echo "FAIL: required Homebrew formula assets missing or not downloadable for ${TAG}:" >&2
  for m in $missing; do
    echo "  - $m" >&2
  done
  echo "" >&2
  echo "Required minimum for ship gate: aarch64-apple-darwin + x86_64-unknown-linux-gnu." >&2
  echo "Also expected: x86_64-apple-darwin (macOS Intel)." >&2
  echo "Fix: publish a green Release queria-cli for ${TAG}, then re-run." >&2
  if [[ -z "$TOKEN" ]]; then
    echo "Private assets: export GH_TOKEN=… (or GITHUB_TOKEN / HOMEBREW_GITHUB_API_TOKEN) and re-run." >&2
  fi
  echo "Refusing to write a formula with invented or partial sha256 values." >&2
  exit 1
fi

LINUX_ARM_BLOCK=""
set +e
sha_arm="$(fetch_sha "$optional_asset" 2>/dev/null)"
rc_arm=$?
set -e
if [[ $rc_arm -eq 0 ]] && is_sha256 "$sha_arm"; then
  SHA_LINUX_ARM="$sha_arm"
  echo "ok  $optional_asset  $sha_arm" >&2
  LINUX_ARM_INNER="$(formula_url_block "$optional_asset" "$SHA_LINUX_ARM" 6)"
  LINUX_ARM_BLOCK=$(cat <<EOF
    on_arm do
${LINUX_ARM_INNER}
    end
EOF
)
else
  echo "warn: linux arm64 asset missing; formula will odie on linux arm" >&2
  LINUX_ARM_BLOCK=$(cat <<'EOF'
    on_arm do
      odie "queria-cli: no aarch64-unknown-linux-gnu asset for this release"
    end
EOF
)
fi

URL_DARWIN_ARM="$(formula_url_block queria-cli-aarch64-apple-darwin.tar.gz "$SHA_DARWIN_ARM" 6)"
URL_DARWIN_INTEL="$(formula_url_block queria-cli-x86_64-apple-darwin.tar.gz "$SHA_DARWIN_INTEL" 6)"
URL_LINUX_INTEL="$(formula_url_block queria-cli-x86_64-unknown-linux-gnu.tar.gz "$SHA_LINUX_INTEL" 6)"

PRIVATE_NOTE=""
if [[ -f "$tmpdir/asset_id.queria_cli_aarch64_apple_darwin_tar_gz" ]]; then
  PRIVATE_NOTE="# Private queria-backend: urls are Releases API asset endpoints.
# brew requires: export HOMEBREW_GITHUB_API_TOKEN=ghp_… (repo read)
# (plain github.com/.../releases/download/ URLs return 404 for private repos).
"
fi

mkdir -p "$(dirname "$OUT")"
cat >"$OUT" <<EOF
# frozen_string_literal: true

# Generated by queria-backend/scripts/generate_homebrew_formula.sh
# Release: ${TAG}  Repo: ${REPO}
# Do not hand-edit sha256; re-run the script after each CLI release.
# Archive layout: queria-cli-<triple>/queria-cli — brew chdirs into the single
# top-level dir, so install is bin.install "queria-cli" (not a nested path).
${PRIVATE_NOTE}
class QueriaCli < Formula
  desc "QuerIa CLI for laptop index-here + hub TUI (doctor/index/status)"
  homepage "https://github.com/${REPO}"
  version "${VERSION}"
  license "MIT"

  on_macos do
    on_arm do
${URL_DARWIN_ARM}
    end
    on_intel do
${URL_DARWIN_INTEL}
    end
  end

  on_linux do
${LINUX_ARM_BLOCK}
    on_intel do
${URL_LINUX_INTEL}
    end
  end

  def install
    bin.install "queria-cli"
  end

  test do
    assert_match "index-here", shell_output("#{bin}/queria-cli index-here --help")
  end
end
EOF

echo "wrote $OUT" >&2
if [[ -n "$PRIVATE_NOTE" ]]; then
  echo "next: commit+push homebrew-queria, then:" >&2
  echo "  export HOMEBREW_GITHUB_API_TOKEN=ghp_…   # read access to ${REPO}" >&2
  echo "  brew update && brew reinstall nandocoeg2/queria/queria-cli" >&2
else
  echo "next: commit+push homebrew-queria, then: brew update && brew reinstall nandocoeg2/queria/queria-cli" >&2
fi
