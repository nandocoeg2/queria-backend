#!/usr/bin/env bash
# QuerIa auto-retrieve hook (T4 + R6 + H1).
# Events: SessionStart, UserPromptSubmit (Droid / Claude Code compatible stdin JSON).
# Fail-open: any network/auth failure exits 0 with empty inject so agent work continues.
#
# Env:
#   QUERIA_AGENT_TOKEN   (required) raw qria_… token
#   QUERIA_EDGE_URL      (required) e.g. http://127.0.0.1:17674
#   QUERIA_PROJECT_ID    preferred project UUID
#   QUERIA_PROJECT_SLUG  alt resolution
# Optional:
#   QUERIA_HOOK_LIMIT (default 5)
#   QUERIA_HOOK_COOLDOWN_SEC (default 30)
#   QUERIA_HOOK_MAX_CHARS (default 3500)
#   QUERIA_HOOK_STATE_DIR (default ${XDG_CACHE_HOME:-$HOME/.cache}/queria-hooks)
#   QUERIA_HOOK_SAME_QUERY_SEC (default 300)

set -u

COOLDOWN_SEC="${QUERIA_HOOK_COOLDOWN_SEC:-30}"
SAME_QUERY_SEC="${QUERIA_HOOK_SAME_QUERY_SEC:-300}"
MAX_CHARS="${QUERIA_HOOK_MAX_CHARS:-3500}"
LIMIT="${QUERIA_HOOK_LIMIT:-5}"
STATE_DIR="${QUERIA_HOOK_STATE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/queria-hooks}"
EDGE_URL="${QUERIA_EDGE_URL:-}"
TOKEN="${QUERIA_AGENT_TOKEN:-}"
PROJECT_ID="${QUERIA_PROJECT_ID:-}"
PROJECT_SLUG="${QUERIA_PROJECT_SLUG:-}"

warn() { printf '%s\n' "$*" >&2; }

# Always fail-open for the agent.
fail_open() {
  warn "queria-hook: $*"
  exit 0
}

if ! command -v jq >/dev/null 2>&1; then
  fail_open "jq not found; skip auto-retrieve"
fi
if ! command -v curl >/dev/null 2>&1; then
  fail_open "curl not found; skip auto-retrieve"
fi

INPUT="$(cat || true)"
if [[ -z "${INPUT// }" ]]; then
  fail_open "empty stdin"
fi

EVENT="$(printf '%s' "$INPUT" | jq -r '.hook_event_name // .hookEventName // empty' 2>/dev/null || true)"
PROMPT="$(printf '%s' "$INPUT" | jq -r '.prompt // empty' 2>/dev/null || true)"
CWD="$(printf '%s' "$INPUT" | jq -r '.cwd // empty' 2>/dev/null || true)"
SOURCE="$(printf '%s' "$INPUT" | jq -r '.source // empty' 2>/dev/null || true)"

# Only run useful SessionStart sources when present
if [[ "$EVENT" == "SessionStart" && -n "$SOURCE" && "$SOURCE" != "startup" && "$SOURCE" != "resume" && "$SOURCE" != "clear" ]]; then
  # compact/other: still allow soft inject; no skip
  :
fi

if [[ -z "$TOKEN" || -z "$EDGE_URL" ]]; then
  fail_open "QUERIA_AGENT_TOKEN or QUERIA_EDGE_URL unset; skip"
fi

# Query selection
QUERY=""
if [[ "$EVENT" == "UserPromptSubmit" || -n "$PROMPT" ]]; then
  QUERY="$PROMPT"
elif [[ "$EVENT" == "SessionStart" || -z "$EVENT" ]]; then
  base="$(basename "${CWD:-$PWD}" 2>/dev/null || echo project)"
  if [[ -n "$CWD" && -f "$CWD/README.md" ]]; then
    head_snip="$(head -n 12 "$CWD/README.md" 2>/dev/null | tr '\n' ' ' | head -c 200 || true)"
    QUERY="project ${base} overview architecture conventions setup ${head_snip}"
  else
    QUERY="project ${base} overview architecture conventions setup deploy"
  fi
else
  # Unknown event: soft skip
  exit 0
fi

# Normalize whitespace
QUERY="$(printf '%s' "$QUERY" | tr -s '[:space:]' ' ' | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
if [[ ${#QUERY} -gt 480 ]]; then
  QUERY="${QUERY:0:480}"
fi

# R5: skip trivial prompts
trivial_re='^(ok|okay|thanks|thank you|thx|yes|yep|no|nope|lanjut|continue|lgtm|sure|k|kk|\.+)$'
if [[ -n "$PROMPT" ]]; then
  low="$(printf '%s' "$QUERY" | tr '[:upper:]' '[:lower:]')"
  if [[ ${#low} -lt 8 ]] || [[ "$low" =~ $trivial_re ]]; then
    exit 0
  fi
fi

if [[ -z "$QUERY" ]]; then
  exit 0
fi

mkdir -p "$STATE_DIR" 2>/dev/null || true
STATE_FILE="$STATE_DIR/last.json"
NOW="$(date +%s)"

query_hash="$(printf '%s' "$QUERY" | shasum -a 256 2>/dev/null | awk '{print $1}')"
if [[ -z "$query_hash" ]]; then
  query_hash="$(printf '%s' "$QUERY" | cksum | awk '{print $1}')"
fi

if [[ -f "$STATE_FILE" ]]; then
  last_ts="$(jq -r '.ts // 0' "$STATE_FILE" 2>/dev/null || echo 0)"
  last_hash="$(jq -r '.query_hash // empty' "$STATE_FILE" 2>/dev/null || true)"
  if [[ "$last_ts" =~ ^[0-9]+$ ]]; then
    delta=$((NOW - last_ts))
    if [[ "$delta" -lt "$COOLDOWN_SEC" ]]; then
      exit 0
    fi
    if [[ -n "$last_hash" && "$last_hash" == "$query_hash" && "$delta" -lt "$SAME_QUERY_SEC" ]]; then
      exit 0
    fi
  fi
fi

# Build request body
if [[ -n "$PROJECT_ID" ]]; then
  BODY="$(jq -nc --arg q "$QUERY" --arg id "$PROJECT_ID" --argjson lim "$LIMIT" \
    '{project_id:$id, query:$q, limit:$lim, include_scratch:true, include_global:true}')"
elif [[ -n "$PROJECT_SLUG" ]]; then
  BODY="$(jq -nc --arg q "$QUERY" --arg slug "$PROJECT_SLUG" --argjson lim "$LIMIT" \
    '{project_slug:$slug, query:$q, limit:$lim, include_scratch:true, include_global:true}')"
else
  # Bootstrap: pick first allowed project
  projects_json="$(curl -sS -m 8 \
    -H "Authorization: Bearer ${TOKEN}" \
    "${EDGE_URL%/}/api/v1/agent/projects" 2>/dev/null || true)"
  first_id="$(printf '%s' "$projects_json" | jq -r '.projects[0].id // empty' 2>/dev/null || true)"
  if [[ -z "$first_id" ]]; then
    fail_open "no project id/slug and list projects empty"
  fi
  BODY="$(jq -nc --arg q "$QUERY" --arg id "$first_id" --argjson lim "$LIMIT" \
    '{project_id:$id, query:$q, limit:$lim, include_scratch:true, include_global:true}')"
fi

HTTP_BODY="$(mktemp 2>/dev/null || echo /tmp/queria-hook-$$.body)"
HTTP_CODE="$(curl -sS -m 12 \
  -o "$HTTP_BODY" -w '%{http_code}' \
  -X POST "${EDGE_URL%/}/api/v1/agent/retrieve-context" \
  -H "Authorization: Bearer ${TOKEN}" \
  -H 'Content-Type: application/json' \
  -d "$BODY" 2>/dev/null || echo "000")"

if [[ "$HTTP_CODE" != "200" ]]; then
  rm -f "$HTTP_BODY" 2>/dev/null || true
  fail_open "retrieve HTTP ${HTTP_CODE}"
fi

# Persist throttle state (no secrets)
jq -nc --argjson ts "$NOW" --arg qh "$query_hash" '{ts:$ts, query_hash:$qh}' >"$STATE_FILE" 2>/dev/null || true

# Format condensed markdown inject (stdout → SessionStart / UserPromptSubmit context)
COUNT="$(jq -r '.items | length // 0' "$HTTP_BODY" 2>/dev/null || echo 0)"
MODE="$(jq -r '.retrieval.mode // "unknown"' "$HTTP_BODY" 2>/dev/null || echo unknown)"

{
  echo "## QuerIa context (auto)"
  echo ""
  echo "Auto-retrieved ${COUNT} item(s); mode=${MODE}. Prefer trusted lane over scratch. Call MCP retrieve_context for deeper queries."
  echo ""
  if [[ "$COUNT" == "0" || "$COUNT" == "null" ]]; then
    echo "_No citations (empty index or embeddings pending)._"
  else
    jq -r '
      .items[]? |
      "### \(.title // "untitled") [\(.lane // "?")/\(.status // "?")] score=\(.score // 0)\n" +
      (if .citation.source_path then "- path: `\(.citation.source_path)`\n" else "" end) +
      (if .citation.source_uri then "- uri: `\(.citation.source_uri)`\n" else "" end) +
      "\n" +
      ((.body // "") | gsub("\r"; "") | split("\n") | .[0:12] | join("\n")) +
      "\n"
    ' "$HTTP_BODY" 2>/dev/null || true
  fi
} | head -c "$MAX_CHARS"

rm -f "$HTTP_BODY" 2>/dev/null || true
exit 0
