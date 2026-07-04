#!/usr/bin/env bash
set -euo pipefail

PROJECT_SLUG="${1:-fjulian-me}"
QUERY="${2:-Astro markdown content flow}"

run_with_secrets() {
  rtk infisical run --env=dev -- "$@"
}

run_with_secrets cargo run -p queria-cli -- database migrate
run_with_secrets cargo run -p queria-cli -- embeddings status --project "${PROJECT_SLUG}"
run_with_secrets cargo run -p queria-cli -- retrieval probe \
  --project "${PROJECT_SLUG}" \
  --query "${QUERY}" \
  --limit 5
