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

VERSION="${TAG#cli-}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEFAULT_OUT="$(cd "$ROOT/.." && pwd)/homebrew-queria/Formula/queria-cli.rb"
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

AUTH=()
if [[ -n "$TOKEN" ]]; then
  AUTH=(-H "Authorization: Bearer ${TOKEN}" -H "Accept: application/octet-stream")
  echo "auth: using GitHub token for private/public asset download" >&2
else
  echo "auth: no GH_TOKEN/GITHUB_TOKEN/HOMEBREW_GITHUB_API_TOKEN — public downloads only" >&2
fi

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

# Download asset and print sha256 of the **tarball**. Never invent hashes.
# Returns 0 and prints hash on stdout; errors go to stderr.
fetch_sha() {
  local asset="$1"
  local url="https://github.com/${REPO}/releases/download/${TAG}/${asset}"
  local dest="$tmpdir/$asset"
  local http_code
  echo "fetch $url" >&2
  # Capture body to file and HTTP status; do not fail the whole script on curl error alone.
  set +e
  http_code="$(curl -sS -L -w '%{http_code}' -o "$dest" "${AUTH[@]+"${AUTH[@]}"}" "$url" 2>"$tmpdir/${asset}.curl.err")"
  local curl_rc=$?
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
  LINUX_ARM_BLOCK=$(cat <<EOF
    on_arm do
      url "https://github.com/${REPO}/releases/download/${TAG}/queria-cli-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "${SHA_LINUX_ARM}"
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

mkdir -p "$(dirname "$OUT")"
cat >"$OUT" <<EOF
# frozen_string_literal: true

# Generated by queria-backend/scripts/generate_homebrew_formula.sh
# Release: ${TAG}  Repo: ${REPO}
# Do not hand-edit sha256; re-run the script after each CLI release.
# Archive layout: queria-cli-<triple>/queria-cli — brew chdirs into the single
# top-level dir, so install is bin.install "queria-cli" (not a nested path).

class QueriaCli < Formula
  desc "QuerIa CLI for laptop index-here (Needs review ingest)"
  homepage "https://github.com/${REPO}"
  version "${VERSION}"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/${REPO}/releases/download/${TAG}/queria-cli-aarch64-apple-darwin.tar.gz"
      sha256 "${SHA_DARWIN_ARM}"
    end
    on_intel do
      url "https://github.com/${REPO}/releases/download/${TAG}/queria-cli-x86_64-apple-darwin.tar.gz"
      sha256 "${SHA_DARWIN_INTEL}"
    end
  end

  on_linux do
${LINUX_ARM_BLOCK}
    on_intel do
      url "https://github.com/${REPO}/releases/download/${TAG}/queria-cli-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "${SHA_LINUX_INTEL}"
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
echo "next: commit+push homebrew-queria, then: brew update && brew reinstall nandocoeg2/queria/queria-cli" >&2
