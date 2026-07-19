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


def mcp(edge: str, token: str, method: str, params: dict | None = None, rid: int = 1) -> dict:
    body: dict[str, Any] = {"jsonrpc": "2.0", "id": rid, "method": method}
    if params is not None:
        body["params"] = params
    code, raw = http_request(
        "POST",
        f"{edge}/mcp",
        token=token,
        body=body,
        timeout=180,
    )
    if code != 200:
        return {"_http_status": code, "_raw": raw[:500]}
    try:
        return parse_json_body(raw)
    except Exception as e:
        return {"_parse_error": str(e), "_raw": raw[:500]}


def tools_call(edge: str, token: str, name: str, arguments: dict, rid: int = 10) -> dict:
    return mcp(
        edge,
        token,
        "tools/call",
        {"name": name, "arguments": arguments},
        rid=rid,
    )


def tool_payload(resp: dict) -> dict:
    """Normalize MCP tools/call response to {isError, structured, texts}."""
    if resp.get("_http_status") and resp["_http_status"] != 200:
        return {
            "isError": True,
            "texts": [f"http {resp['_http_status']}: {resp.get('_raw', '')}"],
            "structured": None,
        }
    if resp.get("_parse_error"):
        return {"isError": True, "texts": [resp["_parse_error"]], "structured": None}
    if resp.get("error") is not None:
        err = resp["error"]
        msg = err.get("message") if isinstance(err, dict) else str(err)
        return {"isError": True, "texts": [msg or "error"], "structured": None}
    r = resp.get("result") or {}
    content = r.get("content") or []
    texts = [c.get("text", "") for c in content if isinstance(c, dict)]
    if r.get("isError"):
        return {"isError": True, "texts": texts, "structured": r.get("structuredContent")}
    sc = r.get("structuredContent")
    parsed = None
    for t in texts:
        try:
            parsed = json.loads(t)
            break
        except Exception:
            pass
    if sc is None and isinstance(parsed, dict):
        sc = parsed
    return {"isError": False, "structured": sc, "texts": texts}


def items_from(pr: dict) -> list:
    s = pr.get("structured") or {}
    if isinstance(s, dict):
        its = s.get("items") or (s.get("result") or {}).get("items") or []
        if its:
            return its
    for t in pr.get("texts") or []:
        try:
            j = json.loads(t)
            its = j.get("items") or []
            if its:
                return its
        except Exception:
            continue
    return []


def payload_blob(pr: dict) -> str:
    parts = list(pr.get("texts") or [])
    if pr.get("structured") is not None:
        parts.append(json.dumps(pr["structured"]))
    return "\n".join(parts)


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

    # --- E7 initialize + tools/list ---
    init = mcp(
        edge,
        token,
        "initialize",
        {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "e2e-agent-path-edge", "version": "0.1"},
        },
        rid=1,
    )
    if "result" not in init:
        fail("E7", f"initialize failed: {init}")
    listed = mcp(edge, token, "tools/list", {}, rid=2)
    tools = (listed.get("result") or {}).get("tools") or []
    names = {t.get("name") for t in tools if isinstance(t, dict)}
    for required in ("retrieve_context", "list_projects", "index_memory"):
        if required not in names:
            fail("E7", f"missing tool {required} in {names}")
    ok("E7", f"tools={len(names)}")

    # --- E8 list_projects ---
    lp = tool_payload(
        tools_call(edge, token, "list_projects", {}, rid=11)
    )
    if lp["isError"]:
        fail("E8", str(lp.get("texts")))
    sc = lp.get("structured") or {}
    projs = sc.get("projects") or []
    mcp_slugs = {
        x.get("slug") for x in projs if isinstance(x, dict) and x.get("slug")
    }
    if slug not in mcp_slugs:
        fail("E8", f"smoke slug missing in MCP list: {mcp_slugs!r}")
    if mcp_slugs != {slug}:
        fail("E8", f"unexpected extra projects (token should be smoke-only): {mcp_slugs!r}")
    ok("E8")

    # --- E9 retrieve_context ---
    rv = tool_payload(
        tools_call(
            edge,
            token,
            "retrieve_context",
            {
                "project_id": project_id,
                "query": "project overview conventions",
                "include_scratch": True,
                "include_global": False,
                "limit": 5,
            },
            rid=12,
        )
    )
    if rv["isError"]:
        fail("E9", str(rv.get("texts")))
    ok("E9", f"items={len(items_from(rv))}")

    # --- E10 index_memory ---
    marker = f"{MARKER_PREFIX}{int(time.time())}-{uuid.uuid4().hex[:8]}"
    body_text = f"{marker} agent-path edge e2e scratch note"
    ix = tool_payload(
        tools_call(
            edge,
            token,
            "index_memory",
            {
                "project_id": project_id,
                "body": body_text,
                "title": f"e2e {marker}",
                "category": "e2e",
                "tags": ["e2e-agent"],
            },
            rid=13,
        )
    )
    if ix["isError"]:
        fail("E10", str(ix.get("texts")))
    ok("E10", f"marker={marker}")

    # --- E11 retrieve scratch for marker ---
    found = False
    last_note = ""
    for attempt in range(RETRIES):
        rr = tool_payload(
            tools_call(
                edge,
                token,
                "retrieve_context",
                {
                    "project_id": project_id,
                    "query": marker,
                    "include_scratch": True,
                    "include_global": False,
                    "limit": 10,
                },
                rid=20 + attempt,
            )
        )
        if rr["isError"]:
            last_note = str(rr.get("texts"))
            time.sleep(RETRY_SLEEP_SEC)
            continue
        blob = payload_blob(rr)
        if marker in blob:
            found = True
            break
        # also scan item bodies
        for it in items_from(rr):
            if marker in json.dumps(it):
                found = True
                break
        if found:
            break
        last_note = f"attempt {attempt+1}: items={len(items_from(rr))}"
        time.sleep(RETRY_SLEEP_SEC)
    if not found:
        fail("E11", f"marker not found after retries: {last_note}")
    ok("E11")

    # --- E12 hook script smoke ---
    if args.skip_hooks:
        skip("E12", "--skip-hooks")
    else:
        if not hook_script_body:
            fail("E12", "no hook script from E1")
        with tempfile.TemporaryDirectory() as td:
            path = os.path.join(td, "queria-retrieve-hook.sh")
            with open(path, "w", encoding="utf-8") as f:
                f.write(hook_script_body)
            os.chmod(path, 0o755)
            syn = subprocess.run(
                ["bash", "-n", path], capture_output=True, text=True
            )
            if syn.returncode != 0:
                fail("E12", f"bash -n: {syn.stderr[:300]}")
            env = os.environ.copy()
            env["QUERIA_AGENT_TOKEN"] = token
            env["QUERIA_EDGE_URL"] = edge
            env["QUERIA_PROJECT_SLUG"] = slug
            stdin = json.dumps(
                {
                    "hook_event_name": "UserPromptSubmit",
                    "prompt": "ok",
                }
            )
            run = subprocess.run(
                ["bash", path],
                input=stdin,
                capture_output=True,
                text=True,
                env=env,
                timeout=30,
            )
            if run.returncode != 0:
                fail(
                    "E12",
                    f"exit={run.returncode} stderr={run.stderr[:300]}",
                )
        ok("E12")

    print("RESULT: PASS")
    sys.exit(0)


if __name__ == "__main__":
    main()
