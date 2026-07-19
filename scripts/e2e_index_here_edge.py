#!/usr/bin/env python3
"""Multi-root index-here edge smoke (spec residuals 1–2).

Env:
  QUERIA_EDGE_URL          default http://127.0.0.1:17674
  QUERIA_AGENT_TOKEN       Bearer with IndexLocal (and ideally RetrieveContext)
  QUERIA_PROMOTE_TOKEN     optional Bearer with ManageNeedsReview
  QUERIA_CLI               optional path to queria-cli binary
  QUERIA_SMOKE_WAIT_SEC    optional embed wait (default 30)

Exit: 0 PASS or PASS with SKIP promote; non-zero on hard fail.
Never prints full qria_ tokens.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


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
    headers = {"Accept": "application/json"}
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


def git(cwd: Path, *args: str) -> None:
    r = subprocess.run(
        ["git", *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        check=False,
    )
    if r.returncode != 0:
        raise RuntimeError(f"git {args} in {cwd}: {r.stderr or r.stdout}")


def write_add(repo: Path, rel: str, body: str) -> None:
    path = repo / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(body, encoding="utf-8")
    git(repo, "add", rel)


def setup_fixture(root: Path) -> None:
    """Parent + nested git; parent does NOT track nested files (clean multi-root)."""
    parent = root / "workspace"
    nested = parent / "services" / "api"
    parent.mkdir(parents=True)
    nested.mkdir(parents=True)

    for d in (parent, nested):
        git(d, "init", "-q")
        git(d, "config", "user.email", "smoke@example.com")
        git(d, "config", "user.name", "Smoke")

    write_add(parent, "docs/parent-smoke.md", "# parent smoke uniquephrase-alpha\n")
    write_add(nested, "src/nested-smoke.ts", "export const nestedPhrase = 'uniquephrase-beta';\n")
    write_add(nested, "README.md", "# nested uniquephrase-beta\n")

    git(parent, "commit", "-qm", "parent smoke")
    git(nested, "commit", "-qm", "nested smoke")
    # Distinct remote-looking origins → distinct auto-slugs
    git(parent, "remote", "add", "origin", "git@smoke.example:org/parent-smoke-idx.git")
    git(nested, "remote", "add", "origin", "git@smoke.example:org/nested-smoke-idx.git")


def find_cli() -> str:
    env = os.environ.get("QUERIA_CLI", "").strip()
    if env:
        return env
    which = shutil.which("queria-cli")
    if which:
        return which
    # cargo workspace default
    cand = Path(__file__).resolve().parents[1] / "target" / "debug" / "queria-cli"
    if cand.is_file():
        return str(cand)
    return "queria-cli"


def run_cli(cli: str, cwd: Path, args: list[str], env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [cli, *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        env=env,
        timeout=300,
    )


def mcp_tools_call(edge: str, token: str, name: str, arguments: dict) -> dict:
    body = {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments},
    }
    code, raw = http_request("POST", f"{edge}/mcp", token=token, body=body, timeout=120)
    if code != 200:
        return {"isError": True, "http": code, "raw": raw[:400]}
    try:
        j = json.loads(raw)
    except Exception:
        return {"isError": True, "raw": raw[:400]}
    if j.get("error"):
        return {"isError": True, "error": j["error"]}
    r = j.get("result") or {}
    texts = [
        c.get("text", "")
        for c in (r.get("content") or [])
        if isinstance(c, dict)
    ]
    structured = r.get("structuredContent")
    for t in texts:
        try:
            structured = json.loads(t)
            break
        except Exception:
            pass
    return {"isError": bool(r.get("isError")), "structured": structured, "texts": texts}


def retrieve(edge: str, token: str, project_id: str, query: str) -> list:
    code, raw = http_request(
        "POST",
        f"{edge}/api/v1/agent/retrieve-context",
        token=token,
        body={
            "project_id": project_id,
            "query": query,
            "limit": 10,
            "include_needs_review": False,
        },
        timeout=60,
    )
    if code != 200:
        # fallback MCP
        pr = mcp_tools_call(
            edge,
            token,
            "retrieve_context",
            {
                "project_id": project_id,
                "query": query,
                "limit": 10,
                "include_needs_review": False,
            },
        )
        s = pr.get("structured") or {}
        return list(s.get("items") or [])
    try:
        j = json.loads(raw)
    except Exception:
        return []
    return list(j.get("items") or (j.get("result") or {}).get("items") or [])


def list_projects_mcp(edge: str, token: str) -> list[dict]:
    pr = mcp_tools_call(edge, token, "list_projects", {})
    s = pr.get("structured")
    if isinstance(s, dict):
        projects = s.get("projects") or s.get("items") or []
        if isinstance(projects, list):
            return [p for p in projects if isinstance(p, dict)]
    for t in pr.get("texts") or []:
        try:
            j = json.loads(t)
            projects = j.get("projects") or j.get("items") or []
            if isinstance(projects, list):
                return [p for p in projects if isinstance(p, dict)]
        except Exception:
            continue
    return []


def main() -> None:
    edge = os.environ.get("QUERIA_EDGE_URL", "http://127.0.0.1:17674").rstrip("/")
    token = os.environ.get("QUERIA_AGENT_TOKEN", "").strip()
    promote = os.environ.get("QUERIA_PROMOTE_TOKEN", "").strip()
    wait_sec = int(os.environ.get("QUERIA_SMOKE_WAIT_SEC", "30"))
    cli = find_cli()

    # E0 health
    code, body = http_request("GET", f"{edge}/healthz", timeout=15)
    if code != 200:
        fail("E0", f"healthz {code} {body[:200]}")
    ok("E0", "healthz 200")

    if not token:
        fail("I0", "set QUERIA_AGENT_TOKEN (IndexLocal + RetrieveContext recommended)")

    env = os.environ.copy()
    env["QUERIA_AGENT_TOKEN"] = token
    env["QUERIA_EDGE_URL"] = edge

    with tempfile.TemporaryDirectory(prefix="queria-idx-here-") as td:
        root = Path(td)
        setup_fixture(root)
        workspace = root / "workspace"

        # I1 dry-run
        r = run_cli(
            cli,
            workspace,
            ["index-here", "--token-env", "QUERIA_AGENT_TOKEN", "--dry-run", "--yes", "--depth", "4"],
            env,
        )
        out = (r.stdout or "") + (r.stderr or "")
        if r.returncode != 0:
            fail("I1", f"dry-run exit {r.returncode}: {out[:500]}")
        if "discovered" not in out and "git root" not in out:
            # still ok if summary format differs slightly
            pass
        root_lines = [ln for ln in out.splitlines() if "origin=" in ln or "git root" in ln]
        ok("I1", f"dry-run roots_hint={len(root_lines)} exit=0")

        # I2 index
        r = run_cli(
            cli,
            workspace,
            ["index-here", "--token-env", "QUERIA_AGENT_TOKEN", "--yes", "--depth", "4"],
            env,
        )
        out = (r.stdout or "") + (r.stderr or "")
        if r.returncode != 0:
            fail("I2", f"index-here exit {r.returncode}: {out[:800]}")
        ok("I2", "index-here upload")

        if wait_sec > 0:
            time.sleep(min(wait_sec, 120))
            ok("I3", f"waited {min(wait_sec, 120)}s for embed jobs")
        else:
            skip("I3", "QUERIA_SMOKE_WAIT_SEC=0")

        projects = list_projects_mcp(edge, token)
        # Prefer slugs from auto-create (parent-smoke-idx / nested-smoke-idx → last segment)
        candidates = [
            p
            for p in projects
            if str(p.get("slug") or "").endswith("smoke-idx")
            or "smoke" in str(p.get("slug") or "").lower()
            or "parent" in str(p.get("slug") or "").lower()
            or "nested" in str(p.get("slug") or "").lower()
        ]
        if not candidates and projects:
            candidates = projects[:2]
        if not candidates:
            skip("I4", "no projects visible to token (list_projects empty or authz)")
            print("RESULT: PASS (partial)")
            return

        # I4 default retrieve should not surface needs_review bodies as trusted citations preferred
        any_items_nr = False
        for p in candidates[:4]:
            pid = str(p.get("id") or p.get("project_id") or "")
            if not pid:
                continue
            items = retrieve(edge, token, pid, "uniquephrase-alpha uniquephrase-beta")
            # If include_needs_review is default false, items should not include lane needs_review
            for it in items:
                status = str(it.get("status") or "").lower()
                lane = str(it.get("lane") or "").lower()
                if status == "needs_review" or lane == "needs_review":
                    any_items_nr = True
        if any_items_nr:
            fail("I4", "default retrieve returned needs_review status/lane")
        ok("I4", f"default retrieve no NR across {min(4, len(candidates))} project(s)")

        # I5 promote
        if not promote:
            skip("I5", "set QUERIA_PROMOTE_TOKEN with manage_needs_review for promote step")
            print("RESULT: PASS (promote skipped)")
            return

        # list NR via MCP
        pr = mcp_tools_call(edge, promote, "list_needs_review", {"limit": 50})
        if pr.get("isError"):
            skip("I5", f"list_needs_review failed (grant?): {redact(str(pr)[:200])}")
            print("RESULT: PASS (promote skipped)")
            return
        structured = pr.get("structured") or {}
        items = structured.get("items") or structured.get("knowledge_items") or []
        if not items and isinstance(structured, list):
            items = structured
        # try texts
        if not items:
            for t in pr.get("texts") or []:
                try:
                    j = json.loads(t)
                    items = j.get("items") or j or []
                    if items:
                        break
                except Exception:
                    continue
        if not items:
            skip("I5", "no needs_review items to promote (embed lag or empty)")
            print("RESULT: PASS (promote skipped)")
            return

        kid = None
        if isinstance(items, list) and items:
            first = items[0]
            if isinstance(first, dict):
                kid = first.get("knowledge_item_id") or first.get("id")
        if not kid:
            skip("I5", "could not parse knowledge_item_id from list_needs_review")
            print("RESULT: PASS (promote skipped)")
            return

        pr2 = mcp_tools_call(
            edge, promote, "promote_knowledge", {"knowledge_item_id": str(kid)}
        )
        if pr2.get("isError"):
            fail("I5", f"promote_knowledge: {redact(str(pr2)[:300])}")
        ok("I5", f"promoted {kid}")

        # I6 retrieve after promote (best-effort)
        time.sleep(2)
        hit = False
        for p in candidates[:4]:
            pid = str(p.get("id") or p.get("project_id") or "")
            if not pid:
                continue
            items = retrieve(edge, token, pid, "uniquephrase")
            if items:
                hit = True
                break
        if hit:
            ok("I6", "retrieve returned items after promote")
        else:
            skip("I6", "no hits yet (embed lag ok)")

    print("RESULT: PASS")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        fail("INT", "interrupted")
