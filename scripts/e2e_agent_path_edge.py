#!/usr/bin/env python3
"""Agent-path E2E against QuerIa edge (pre-minted smoke token). Spec: agent-path-edge-e2e-design."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from typing import Any

MARKER_PREFIX = "e2e-agent-"
RETRIES = 5
RETRY_SLEEP_SEC = 2


def redact(s: str) -> str:
    return re.sub(r"qria_[A-Za-z0-9_-]+", "qria_***", s)


def fail(step: str, msg: str) -> None:
    print(f"{step} FAIL: {redact(msg)}", file=sys.stderr)
    print("RESULT: FAIL", file=sys.stderr)
    sys.exit(1)


def ok(step: str, note: str = "") -> None:
    suffix = f" ({note})" if note else ""
    print(f"{step} PASS{suffix}")


def skip(step: str, reason: str) -> None:
    print(f"{step} SKIP: {reason}")


def http_request(
    method: str,
    url: str,
    *,
    token: str | None = None,
    body: dict | None = None,
    timeout: float = 60.0,
) -> tuple[int, str]:
    data = None if body is None else json.dumps(body).encode()
    headers = {"Accept": "application/json, text/event-stream"}
    if body is not None:
        headers["Content-Type"] = "application/json"
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.getcode(), resp.read().decode()
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode(errors="replace")
    except Exception as e:
        return 0, str(e)


def parse_json_body(raw: str) -> Any:
    raw = raw.strip()
    if not raw:
        return None
    if "data:" in raw and not raw.startswith("{"):
        data_lines = [ln[5:].strip() for ln in raw.splitlines() if ln.startswith("data:")]
        if data_lines:
            return json.loads(data_lines[-1])
    return json.loads(raw)


def main() -> None:
    p = argparse.ArgumentParser(description="QuerIa agent-path edge E2E")
    p.add_argument(
        "--edge",
        default=os.environ.get("QUERIA_EDGE_URL", "http://168.110.214.130:17674"),
    )
    p.add_argument("--skip-hooks", action="store_true")
    args = p.parse_args()
    edge = args.edge.rstrip("/")
    token = os.environ.get("QUERIA_AGENT_TOKEN", "").strip()
    slug = os.environ.get("QUERIA_SMOKE_PROJECT_SLUG", "queria-smoke").strip()
    if not token.startswith("qria_"):
        fail("E0", "QUERIA_AGENT_TOKEN missing or not qria_*")

    # --- E0 health ---
    code, body = http_request("GET", f"{edge}/healthz", timeout=15)
    if code != 200 or "OK" not in body.upper() and "ok" not in body.lower() and body.strip() != "OK":
        # accept literal OK body (prod) case-insensitive
        if code != 200 or "ok" not in body.lower():
            fail("E0", f"healthz status={code} body={body[:200]}")
    ok("E0")

    # --- E1 hook-script ---
    code, body = http_request("GET", f"{edge}/api/v1/setup/hook-script", timeout=30)
    if code != 200 or not body.startswith("#!/usr/bin/env bash"):
        fail("E1", f"hook-script status={code} head={body[:80]!r}")
    hook_script_body = body
    ok("E1")

    # --- E2 hooks-snippet droid ---
    code, body = http_request(
        "GET", f"{edge}/api/v1/setup/hooks-snippet?client=droid", timeout=30
    )
    if code != 200:
        fail("E2", f"status={code}")
    if "SessionStart" not in body or "UserPromptSubmit" not in body:
        fail("E2", "missing SessionStart/UserPromptSubmit")
    ok("E2")

    # --- E3 no auth projects ---
    code, body = http_request("GET", f"{edge}/api/v1/agent/projects", timeout=30)
    if code != 401 or "agent_token_required" not in body:
        fail("E3", f"expected 401 agent_token_required got {code} {body[:200]}")
    ok("E3")

    # --- E4 bearer projects ---
    code, body = http_request(
        "GET", f"{edge}/api/v1/agent/projects", token=token, timeout=30
    )
    if code != 200:
        fail("E4", f"status={code} body={body[:200]}")
    try:
        projects = parse_json_body(body).get("projects") or []
    except Exception as e:
        fail("E4", f"json: {e}")
    slugs = {pr.get("slug") for pr in projects if isinstance(pr, dict)}
    if slug not in slugs:
        fail("E4", f"smoke slug {slug!r} not in {slugs}")
    project_id = next(pr["id"] for pr in projects if pr.get("slug") == slug)
    ok("E4", f"project_id={project_id}")

    # --- E5 bad bearer ---
    code, body = http_request(
        "POST",
        f"{edge}/api/v1/agent/retrieve-context",
        token="qria_not_a_real_token_for_e5",
        body={"project_slug": slug, "query": "ping", "limit": 3},
        timeout=30,
    )
    if code != 401:
        fail("E5", f"expected 401 got {code}")
    ok("E5")

    # --- E6 valid agent retrieve ---
    code, body = http_request(
        "POST",
        f"{edge}/api/v1/agent/retrieve-context",
        token=token,
        body={
            "project_slug": slug,
            "query": "project overview conventions",
            "limit": 5,
            "include_scratch": True,
            "include_global": False,
        },
        timeout=90,
    )
    if code != 200:
        fail("E6", f"status={code} body={body[:300]}")
    try:
        data = parse_json_body(body)
    except Exception as e:
        fail("E6", f"json: {e}")
    if not isinstance(data.get("items"), list):
        fail("E6", "missing items array")
    if "retrieval" not in data or "project_id" not in data:
        fail("E6", "missing retrieval/project_id")
    ok("E6", f"items={len(data['items'])}")

    # Task 2 appends E7–E11; Task 3 E12
    print("RESULT: PARTIAL (E0-E6 only; continue plan Task 2)")
    # Temporary exit 0 so Task 1 self-check can hit edge; Task 2 removes PARTIAL.
    sys.exit(0)


if __name__ == "__main__":
    main()
