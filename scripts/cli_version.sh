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
