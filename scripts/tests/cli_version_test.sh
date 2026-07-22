#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
# shellcheck source=../cli_version.sh
source "$ROOT/scripts/cli_version.sh"

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

cat >"$tmpdir/ok.toml" <<'TOML'
[package]
name = "queria-cli"
version = "0.3.3"
edition = "2021"
TOML

cat >"$tmpdir/bad.toml" <<'TOML'
[package]
name = "queria-cli"
edition = "2021"
TOML

got="$(cli_version_from_cargo_toml "$tmpdir/ok.toml")"
test "$got" = "0.3.3"
test "$(cli_tag_for_version "$got")" = "cli-v0.3.3"

if cli_version_from_cargo_toml "$tmpdir/bad.toml" 2>/dev/null; then
  echo "expected failure on missing version" >&2
  exit 1
fi

# reject garbage
cat >"$tmpdir/junk.toml" <<'TOML'
version = "not-a-semver!"
TOML
if cli_version_from_cargo_toml "$tmpdir/junk.toml" 2>/dev/null; then
  echo "expected failure on invalid version" >&2
  exit 1
fi

echo "cli_version_test: ok"
