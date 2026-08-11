#!/usr/bin/env python3
"""Harness efficiency benchmark — dispatch a standard task suite through the
REAL goal pipeline and harvest per-run cost evidence.

This automates the measurement performed by hand on 2026-08-10 (goals A/B/C/D):
every number comes from the worker's own transcript or the daemon's session
accounting, never from a model's self-report.

Method (one benchmark run = one task x one configuration):
  1. create a goal card on the target project with CHEAP user-authored
     completion checks (never a cold `cargo` build in a scratch worktree);
  2. dispatch it via the orchestrator through a chat session, PINNED to the
     worker under test (goal_advance worker=...);
  3. poll the card's execution receipt to terminal state;
  4. harvest:
       - claude_code workers ("cli-*" sessions): sum per-message usage from
         the worker's own transcript under ~/.claude/projects/<worktree-slug>/;
       - permagent harness workers: the daemon session row's accumulated
         token columns;
  5. append a JSON record to results/; render a markdown table with
     `--report`.

Honesty rules baked in:
  - n=1 per cell until you rerun; the report prints run counts, and nothing
    is averaged silently.
  - A run whose completion check failed is recorded as FAILED, not dropped —
    a cheap model that fails the task is not "cheaper".
  - Wall-clock is receipt dispatched_at -> terminal_at (includes queueing);
    model time is what the transcript shows. Both are recorded.

Usage:
  harness_bench.py list-tasks
  harness_bench.py run --task lookup --worker claude_code --project <slug> --label baseline
  harness_bench.py run --task multi_file --worker permagent --project <slug> --label kimi-k2.6
  harness_bench.py report

The per-role model behind the `permagent` worker comes from the live
PERMAGENT_ROLE_* config — record it in --label so a result is attributable
after the config changes (e.g. "permagent/kimi-k2.6", "permagent/minimax-m2").
"""

import argparse
import glob
import json
import os
import sqlite3
import sys
import time
import urllib.request
import uuid
from datetime import datetime, timezone

HOME = os.path.expanduser("~")
DAEMON = "http://127.0.0.1:3001"
APP_DB = f"{HOME}/.permagent/spectral/permagent.db"
RESULTS_DIR = os.path.join(os.path.dirname(os.path.abspath(__file__)), "results")

# ── The task suite: one per work class, blog post -> build from scratch ──────
# Each description is intentionally verbatim-stable so reruns are comparable.
TASKS = {
    "lookup": {
        "title": "Bench: locate {nonce} the similarity scorer",
        "description": (
            "Create the file docs/notes/bench-lookup-{nonce}.md containing: the repo-relative "
            "path of the file that defines the function `title_similarity`, the name of the "
            "constant holding its duplicate threshold, and one sentence on the algorithm. "
            "Find these by reading the codebase. Commit the new file. Touch nothing else."
        ),
        "checks": [{"type": "file_exists", "path": "docs/notes/bench-lookup-{nonce}.md"}],
        "class": "navigation / single-symbol lookup",
    },
    "multi_file": {
        "title": "Bench: thread a field {nonce} through the receipt",
        "description": (
            "Add an optional string field `bench_note_{nonce}` to the ExecutionReceipt struct "
            "and thread it through EVERY construction site including tests so the crate still "
            "compiles by inspection (serde must default it on deserialize). Do not run cargo "
            "build/check — verify by reading every construction site. Commit your work."
        ),
        "checks": [{
            "type": "command_exit_zero",
            "cmd": "grep -rq bench_note_{nonce} crates/goose/src/agents/platform_extensions/",
            "timeout_secs": 30,
        }],
        "class": "multi-file structural edit",
    },
    "writing": {
        "title": "Bench: write the {nonce} launch post",
        "description": (
            "Write docs/notes/bench-post-{nonce}.md: a ~600-word launch blog post for a "
            "fictional feature called 'Harbor Sync' (offline-first sync for small teams). "
            "Structure: hook, 3 concrete user scenarios, honest limitations section, CTA. "
            "No placeholder text. Commit the file. Touch nothing else."
        ),
        "checks": [{"type": "file_exists", "path": "docs/notes/bench-post-{nonce}.md"}],
        "class": "prose / marketing writing",
    },
    "scaffold": {
        "title": "Bench: scaffold {nonce} a link-check CLI",
        "description": (
            "From scratch, create tools/linkcheck-{nonce}/: a self-contained Python CLI "
            "(single file + README) that reads a markdown file and reports dead relative "
            "links (files that do not exist). Include 3 unit tests in the same directory "
            "using unittest, runnable with python3 -m unittest discover. Run the tests and "
            "include their output in your commit message. Commit everything."
        ),
        "checks": [{
            "type": "command_exit_zero",
            "cmd": "cd tools/linkcheck-{nonce} && python3 -m unittest discover -v",
            "timeout_secs": 120,
        }],
        "class": "small build from scratch (spec -> working code -> tested)",
    },
}


def token() -> str:
    with open(f"{HOME}/.permagent/secrets/daemon_token.json") as f:
        return json.load(f)["token"]


def api(path: str, body=None, method=None):
    req = urllib.request.Request(
        f"{DAEMON}{path}",
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Authorization": f"Bearer {token()}", "Content-Type": "application/json"},
        method=method or ("POST" if body is not None else "GET"),
    )
    return json.loads(urllib.request.urlopen(req, timeout=60).read().decode())


def project_id(slug: str) -> str:
    for p in api("/api/projects"):
        if p.get("slug") == slug:
            return p["id"]
    sys.exit(f"no project with slug {slug!r}")


def sq(query: str, args=()):
    con = sqlite3.connect(f"file:{APP_DB}?mode=ro", uri=True)
    try:
        return con.execute(query, args).fetchall()
    finally:
        con.close()


def dispatch_via_chat(session_id: str, card_id: str, worker: str):
    text = (
        f"Goal card {card_id}: goal_advance to ready, then goal_advance action dispatch "
        f"with worker='{worker}'. Report the worker session id. Do nothing else."
    )
    api(f"/sessions/{session_id}/reply", {
        "request_id": str(uuid.uuid4()),
        "user_message": {
            "role": "user", "created": int(time.time()),
            "content": [{"type": "text", "text": text}],
            "metadata": {"userVisible": True, "agentVisible": True},
        },
    })


def wait_terminal(card_id: str, timeout_s: int = 2400) -> dict:
    deadline = time.time() + timeout_s
    while time.time() < deadline:
        rows = sq("SELECT metadata_json FROM cards WHERE id=?", (card_id,))
        if rows:
            meta = json.loads(rows[0][0] or "{}")
            receipt = meta.get("execution_receipt") or {}
            if receipt.get("state") in ("completed", "failed", "timed_out", "blocked"):
                return meta
        time.sleep(20)
    sys.exit(f"goal {card_id} did not reach a terminal state in {timeout_s}s")


def harvest_cli(worker_session: str) -> dict:
    pat = f"{HOME}/.claude/projects/*{worker_session}*/*.jsonl"
    files = sorted(glob.glob(pat), key=os.path.getmtime)
    if not files:
        return {"harvest": "cli-transcript-missing"}
    out = cc = cr = inp = turns = tools = 0
    for line in open(files[-1]):
        try:
            d = json.loads(line)
        except json.JSONDecodeError:
            continue
        if d.get("type") != "assistant":
            continue
        m = d.get("message") or {}
        u = m.get("usage") or {}
        out += u.get("output_tokens", 0)
        cc += u.get("cache_creation_input_tokens", 0)
        cr += u.get("cache_read_input_tokens", 0)
        inp += u.get("input_tokens", 0)
        turns += 1
        tools += sum(1 for c in m.get("content", [])
                     if isinstance(c, dict) and c.get("type") == "tool_use")
    return {"harvest": "cli-transcript", "turns": turns, "tool_calls": tools,
            "output_tokens": out, "input_tokens": inp,
            "cache_creation": cc, "cache_read": cr,
            "billed_input": inp + cc + cr}


def harvest_internal(worker_session: str) -> dict:
    # sessions has no message_count column (schema drift caught 2026-08-10 on
    # the first haiku cell) — count from the messages table instead.
    rows = sq(
        "SELECT accumulated_input_tokens, accumulated_output_tokens, "
        "accumulated_cache_read_tokens, accumulated_cache_write_tokens "
        "FROM sessions WHERE id=?", (worker_session,))
    if not rows:
        return {"harvest": "session-row-missing"}
    inp, out, cr, cw = (v or 0 for v in rows[0])
    try:
        msgs = sq("SELECT COUNT(*) FROM messages WHERE session_id=?", (worker_session,))[0][0]
    except Exception:
        msgs = None
    return {"harvest": "daemon-session", "messages": msgs,
            "output_tokens": out, "input_tokens": inp,
            "cache_creation": cw, "cache_read": cr,
            "billed_input": inp + cw + cr}


def cmd_run(args):
    task = TASKS[args.task]
    nonce = datetime.now(timezone.utc).strftime("%m%d%H%M")
    fill = lambda s: s.replace("{nonce}", nonce)
    pid = project_id(args.project)

    card = api(f"/api/projects/{pid}/cards", {
        "title": fill(task["title"]),
        "description": fill(task["description"]),
        "cardType": "goal",
        "metadataJson": {"completion_checks": [
            {k: (fill(v) if isinstance(v, str) else v) for k, v in c.items()}
            for c in task["checks"]]},
    })
    print(f"card {card['id']} created; dispatching to {args.worker} …")
    dispatch_via_chat(args.session, card["id"], args.worker)

    meta = wait_terminal(card["id"])
    receipt = meta.get("execution_receipt") or {}
    ws = meta.get("worker_session_id") or ""
    usage = harvest_cli(ws) if ws.startswith("cli-") else harvest_internal(ws)

    verdict = (meta.get("dispatch_evidence") or {}).get("verdict") or {}
    record = {
        "at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "task": args.task, "task_class": task["class"], "nonce": nonce,
        "label": args.label, "worker": meta.get("worker_key"),
        "card_id": card["id"], "worker_session": ws,
        "state": receipt.get("state"),
        "check_status": verdict.get("status"),
        "dispatched_at": receipt.get("dispatched_at"),
        "terminal_at": receipt.get("terminal_at"),
        "first_output_at": receipt.get("first_output_at"),
        "usage": usage,
    }
    os.makedirs(RESULTS_DIR, exist_ok=True)
    path = os.path.join(RESULTS_DIR, f"{args.task}-{args.label}-{nonce}.json")
    with open(path, "w") as f:
        json.dump(record, f, indent=2)
    print(json.dumps(record, indent=2))
    print(f"\nrecorded -> {path}")


def cmd_report(_args):
    rows = []
    for path in sorted(glob.glob(os.path.join(RESULTS_DIR, "*.json"))):
        with open(path) as f:
            rows.append(json.load(f))
    if not rows:
        print("no results recorded yet")
        return
    print("| task | label | worker | state | check | tools | turns | out tok | billed in | run |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for r in rows:
        u = r.get("usage", {})
        print(f"| {r['task']} | {r['label']} | {r.get('worker')} | {r.get('state')} "
              f"| {r.get('check_status')} | {u.get('tool_calls', u.get('messages', '—'))} "
              f"| {u.get('turns', '—')} | {u.get('output_tokens', '—')} "
              f"| {u.get('billed_input', '—')} | {r['nonce']} |")
    print(f"\n{len(rows)} run(s). n=1 cells are n=1 — rerun before believing a delta.")


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("list-tasks")
    run = sub.add_parser("run")
    run.add_argument("--task", choices=TASKS, required=True)
    run.add_argument("--worker", required=True,
                     help="roster key to pin: claude_code, codex, permagent")
    run.add_argument("--project", required=True, help="project slug")
    run.add_argument("--label", required=True,
                     help="config label, e.g. baseline / permagent-kimi-k2.6")
    run.add_argument("--session", required=True,
                     help="chat session id used to drive goal_advance")
    sub.add_parser("report")
    args = ap.parse_args()

    if args.cmd == "list-tasks":
        for k, t in TASKS.items():
            print(f"{k:11s} {t['class']}")
    elif args.cmd == "run":
        cmd_run(args)
    else:
        cmd_report(args)


if __name__ == "__main__":
    main()
