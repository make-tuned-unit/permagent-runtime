#!/usr/bin/env python3
"""Public-suite benchmark runner: Aider polyglot exercises through the
Permagent coding harness, headless, with per-task JSON evidence.

Why this exists: `harness_bench.py` measures our OWN task suite, which nobody
else runs. This one drives a RECOGNISED public suite (the Aider polyglot
benchmark — 225 Exercism exercises across six languages, frozen since
2024-12-22) so a number from it can be put next to somebody else's number
with the differences stated out loud.

The three phases are separate commands on purpose, because reproducibility
lives in the boundaries between them:

  prepare  deterministic task selection (sorted names + seeded RNG), then
           materialise each task into its own throwaway workspace with the
           reference solution (.meta/) DELETED. Writes a manifest recording
           the suite repo's git SHA, the seed, and the exact task list, so
           the same 10 tasks come back on any machine.
  run      one task at a time (this Mac is memory-tight), each through
           `permagent run --recipe permagent-coding` in that task's
           workspace. Captures stdout/stderr, wall time, exit code.
  grade    NEVER grades in the workspace the agent touched. Copies the
           agent's solution file(s) into a PRISTINE copy of the exercise and
           runs the original tests there. That makes weakening or deleting a
           test worthless — and the tamper is recorded as its own field
           rather than silently absorbed.
  report   markdown table + the honesty clauses.

Honesty rules, same as harness_bench.py:
  - a failed task is a FAILED row, not a dropped one;
  - n is printed on every table, and a pilot is labelled PILOT;
  - nothing is taken from a model's self-report — pass/fail comes from the
    suite's own unittest run, cost/tokens from the session accounting.

Usage:
  polyglot_bench.py prepare --suite <clone> --workdir <dir> --lang python \\
      --n 10 --seed 20260825
  polyglot_bench.py run --workdir <dir> --label deepseek-chat \\
      --provider custom_deepseek --model deepseek-chat --max-turns 40
  polyglot_bench.py grade --workdir <dir> --label deepseek-chat
  polyglot_bench.py report --workdir <dir>
"""

import argparse
import hashlib
import json
import os
import random
import shutil
import sqlite3
import subprocess
import sys
import time
from datetime import datetime, timezone

# Languages whose test runner needs nothing but a stock toolchain. Python's
# Exercism tests are all plain `unittest` (verified: 34/34 on suite SHA
# 7e0611e7), so the runner is dependency-free and fully offline. The other
# five need go / a JDK / npm install / cargo fetch per exercise; they are
# declared here so the gap is visible rather than implied.
LANG_RUNNERS = {
    "python": {
        "test_cmd": [sys.executable, "-m", "unittest", "discover", "-p", "*_test.py"],
        "needs": "python3 only (tests are stdlib unittest)",
    },
}
LANGS_NOT_WIRED = ["cpp", "go", "java", "javascript", "rust"]

STAMP = "%Y-%m-%dT%H:%M:%SZ"


def now():
    return datetime.now(timezone.utc).strftime(STAMP)


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def git_sha(repo):
    return subprocess.run(
        ["git", "-C", repo, "rev-parse", "HEAD"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()


def exercise_dir(suite, lang, name):
    return os.path.join(suite, lang, "exercises", "practice", name)


def read_meta(suite, lang, name):
    with open(os.path.join(exercise_dir(suite, lang, name), ".meta", "config.json")) as fh:
        cfg = json.load(fh)
    files = cfg.get("files", {})
    return {
        "solution": files.get("solution", []),
        "test": files.get("test", []),
        "example": files.get("example", []),
        "blurb": cfg.get("blurb", ""),
    }


# ── prepare ─────────────────────────────────────────────────────────────────

def cmd_prepare(args):
    suite, workdir, lang = args.suite, args.workdir, args.lang
    if lang not in LANG_RUNNERS:
        sys.exit(f"lang '{lang}' has no wired test runner (wired: {list(LANG_RUNNERS)}; "
                 f"present in the suite but not wired here: {LANGS_NOT_WIRED})")

    practice = os.path.join(suite, lang, "exercises", "practice")
    if not os.path.isdir(practice):
        sys.exit(f"not a polyglot-benchmark clone: {practice} missing")

    # Deterministic selection: sort first so filesystem order cannot leak in,
    # then a seeded sample. Same seed + same suite SHA => same task list.
    population = sorted(d for d in os.listdir(practice)
                        if os.path.isdir(os.path.join(practice, d)))
    n = min(args.n, len(population))
    chosen = sorted(random.Random(args.seed).sample(population, n))

    os.makedirs(workdir, exist_ok=True)
    tasks = []
    for name in chosen:
        meta = read_meta(suite, lang, name)
        src = exercise_dir(suite, lang, name)

        # Pristine copy: the grader's ground truth, never shown to the agent.
        pristine = os.path.join(workdir, "pristine", name)
        shutil.rmtree(pristine, ignore_errors=True)
        shutil.copytree(src, pristine)

        # Agent workspace: same thing with the reference solution removed.
        ws = os.path.join(workdir, "workspaces", name)
        shutil.rmtree(ws, ignore_errors=True)
        shutil.copytree(src, ws)
        shutil.rmtree(os.path.join(ws, ".meta"), ignore_errors=True)

        tasks.append({
            "name": name,
            "lang": lang,
            "blurb": meta["blurb"],
            "solution_files": meta["solution"],
            "test_files": meta["test"],
            "instructions_files": sorted(
                os.path.relpath(os.path.join(r, f), ws)
                for r, _, fs in os.walk(os.path.join(ws, ".docs"))
                for f in fs
            ) if os.path.isdir(os.path.join(ws, ".docs")) else [],
            # Hashed BEFORE the agent runs so tampering is detectable after.
            "test_sha256": {
                t: sha256_file(os.path.join(ws, t)) for t in meta["test"]
                if os.path.exists(os.path.join(ws, t))
            },
        })

    manifest = {
        "created_at": now(),
        "suite": "aider-polyglot",
        "suite_repo": "https://github.com/Aider-AI/polyglot-benchmark",
        "suite_sha": git_sha(suite),
        "lang": lang,
        "seed": args.seed,
        "n_requested": args.n,
        "n_selected": len(chosen),
        "population_size": len(population),
        "selection": "sorted(random.Random(seed).sample(sorted(names), n))",
        "test_cmd": LANG_RUNNERS[lang]["test_cmd"],
        "harness_sha": subprocess.run(
            ["git", "-C", os.path.dirname(os.path.dirname(os.path.dirname(
                os.path.abspath(__file__)))), "rev-parse", "HEAD"],
            capture_output=True, text=True).stdout.strip() or "unknown",
        "tasks": tasks,
    }
    with open(os.path.join(workdir, "manifest.json"), "w") as fh:
        json.dump(manifest, fh, indent=2)
    print(f"prepared {len(chosen)}/{len(population)} {lang} tasks "
          f"(seed={args.seed}, suite={manifest['suite_sha'][:8]}) -> {workdir}")
    for t in tasks:
        print(f"  {t['name']}")


# ── run ─────────────────────────────────────────────────────────────────────

PROMPT = """Implement the exercise in this directory so that its tests pass.

Read {docs} for the specification. Write your implementation in {solution}.

Rules:
- Do NOT modify, weaken, delete, or skip any test file ({tests}). The tests are
  the specification; changing them does not count as solving anything.
- Do not add new dependencies. The standard library is available.
- Run the tests with: {test_cmd}
- Stop when the tests pass, or when you have explained why they cannot.
"""


def load_manifest(workdir):
    with open(os.path.join(workdir, "manifest.json")) as fh:
        return json.load(fh)


def cmd_run(args):
    man = load_manifest(args.workdir)
    outdir = os.path.join(args.workdir, "runs", args.label)
    os.makedirs(outdir, exist_ok=True)
    test_cmd_str = " ".join(man["test_cmd"])

    for task in man["tasks"]:
        rec_path = os.path.join(outdir, f"{task['name']}.json")
        if os.path.exists(rec_path) and not args.force:
            print(f"skip {task['name']} (already run; --force to redo)")
            continue

        ws = os.path.join(args.workdir, "workspaces", task["name"])
        # Each label gets its own copy of the workspace, so two models never
        # inherit each other's edits.
        ws_label = os.path.join(args.workdir, "runs", args.label, "ws", task["name"])
        shutil.rmtree(ws_label, ignore_errors=True)
        os.makedirs(os.path.dirname(ws_label), exist_ok=True)
        shutil.copytree(ws, ws_label)

        prompt = PROMPT.format(
            docs=", ".join(task["instructions_files"]) or ".docs/instructions.md",
            solution=", ".join(task["solution_files"]),
            tests=", ".join(task["test_files"]),
            test_cmd=test_cmd_str,
        )

        # A fixed session NAME is the whole harvest strategy: the CLI writes
        # its own accounting into ~/.permagent/spectral/permagent.db, and the
        # name is the only handle a caller can set on a fresh run
        # (`--session-id` is refused unless `--resume` is passed). Tokens and
        # cost then come from the runtime's ledger rather than from anything
        # the model says about itself.
        sname = f"pubbench-{args.label}-{task['name']}-{int(time.time())}"
        # `--recipe` and `--text` are mutually exclusive, so the task statement
        # has to travel INSIDE the recipe. The recipe body is copied verbatim
        # from the shipped `permagent-coding.yaml` and only a `prompt:` block is
        # appended — the derived file is kept next to the record and its sha256
        # is stored, so a reader can prove the harness instructions were not
        # quietly edited to help the model.
        recipe_path = os.path.join(outdir, "recipes", f"{task['name']}.yaml")
        os.makedirs(os.path.dirname(recipe_path), exist_ok=True)
        with open(args.recipe_file) as fh:
            base_recipe = fh.read()
        with open(recipe_path, "w") as fh:
            fh.write(base_recipe.rstrip("\n") + "\nprompt: |\n" +
                     "".join(f"  {line}\n" for line in prompt.splitlines()))

        cmd = [args.binary, "run", "--recipe", recipe_path,
               "--name", sname, "--quiet"]
        if args.provider:
            cmd += ["--provider", args.provider]
        if args.model:
            cmd += ["--model", args.model]
        if args.max_turns:
            cmd += ["--max-turns", str(args.max_turns)]

        print(f"[{now()}] run {task['name']} ({args.label}) ...", flush=True)
        t0 = time.time()
        try:
            proc = subprocess.run(cmd, cwd=ws_label, capture_output=True,
                                  text=True, timeout=args.timeout, stdin=subprocess.DEVNULL)
            rc, out, err, timed_out = proc.returncode, proc.stdout, proc.stderr, False
        except subprocess.TimeoutExpired as e:
            rc, timed_out = -1, True
            out = _txt(e.stdout)
            err = _txt(e.stderr)
        wall = time.time() - t0

        rec = {
            "task": task["name"],
            "label": args.label,
            "session_name": sname,
            "provider_requested": args.provider,
            "model_requested": args.model,
            "started_at": datetime.fromtimestamp(t0, timezone.utc).strftime(STAMP),
            "wall_seconds": round(wall, 1),
            "exit_code": rc,
            "timed_out": timed_out,
            "max_turns": args.max_turns,
            "recipe_file": os.path.relpath(args.recipe_file, os.path.dirname(
                os.path.dirname(os.path.dirname(os.path.abspath(__file__))))),
            "recipe_sha256": sha256_file(recipe_path),
            "base_recipe_sha256": sha256_file(args.recipe_file),
            "stdout_tail": out[-8000:],
            "stderr_tail": err[-8000:],
            "stream_evidence": stream_evidence(out, err),
            "ledger": harvest_ledger(sname),
        }
        with open(rec_path, "w") as fh:
            json.dump(rec, fh, indent=2)
        led = rec["ledger"]
        print(f"    rc={rc} wall={wall:.0f}s cost=${led['cost_usd'] or 0:.4f} "
              f"tool_calls={led['tool_calls']} msgs={led['messages']} "
              f"rate_limit_retries={rec['stream_evidence']['rate_limit_retries']}")


def _txt(v):
    if v is None:
        return ""
    return v.decode("utf-8", "replace") if isinstance(v, bytes) else v


DB = os.path.expanduser("~/.permagent/spectral/permagent.db")


def resolve_session_id(con, session_name):
    row = con.execute("SELECT id FROM sessions WHERE name=? ORDER BY created_at DESC LIMIT 1",
                      (session_name,)).fetchone()
    return row[0] if row else None


def harvest_ledger(session_name):
    """Read the runtime's own accounting for this session.

    `cost_ledger` is the per-call, append-only truth: it carries the provider
    and model that actually served each call, so a run is attributable after
    the config changes, and a subagent's calls (the recipe's cross-model
    reviewer) are separable from the main loop's rather than being quietly
    folded into one number.

    Every field is None when the row is absent. A missing measurement must not
    look like a measured zero.
    """
    empty = {"cost_usd": None, "input_tokens": None, "output_tokens": None,
             "cache_read_tokens": None, "cache_write_tokens": None,
             "calls": None, "messages": None, "tool_calls": None,
             "models_used": [], "subagent_cost_usd": None, "session_id": None,
             "estimated_price_calls": None, "db_found": False}
    if not os.path.exists(DB):
        return empty
    try:
        con = sqlite3.connect(f"file:{DB}?mode=ro", uri=True)
    except sqlite3.Error:
        return empty
    try:
        session_id = resolve_session_id(con, session_name)
        if session_id is None:
            con.close()
            return empty
        row = con.execute(
            "SELECT COALESCE(SUM(cost_usd),0), COALESCE(SUM(input_tokens),0), "
            "COALESCE(SUM(output_tokens),0), COALESCE(SUM(cache_read_tokens),0), "
            "COALESCE(SUM(cache_write_tokens),0), COUNT(*), "
            "COALESCE(SUM(is_estimated),0) FROM cost_ledger "
            "WHERE session_id=? OR parent_session_id=?", (session_id, session_id)).fetchone()
        if not row or row[5] == 0:
            con.close()
            return empty
        models = [f"{p}/{m}" for p, m in con.execute(
            "SELECT DISTINCT provider, model FROM cost_ledger "
            "WHERE session_id=? OR parent_session_id=?", (session_id, session_id))]
        sub = con.execute(
            "SELECT COALESCE(SUM(cost_usd),0) FROM cost_ledger "
            "WHERE (session_id=? OR parent_session_id=?) AND subagent_id IS NOT NULL",
            (session_id, session_id)).fetchone()[0]
        msgs = con.execute("SELECT COUNT(*) FROM messages WHERE session_id=?",
                           (session_id,)).fetchone()[0]
        # Tool calls are counted from the stored messages, not from a log line:
        # every assistant turn's content_json carries its toolRequest entries.
        tools = 0
        for (cj,) in con.execute(
                "SELECT content_json FROM messages WHERE session_id=? AND role='assistant'",
                (session_id,)):
            try:
                for part in json.loads(cj or "[]"):
                    if isinstance(part, dict) and (
                            "toolRequest" in part or part.get("type") == "toolRequest"):
                        tools += 1
            except (json.JSONDecodeError, TypeError):
                pass
        con.close()
    except sqlite3.Error:
        return empty
    return {"cost_usd": round(row[0], 6), "input_tokens": row[1], "output_tokens": row[2],
            "session_id": session_id,
            "cache_read_tokens": row[3], "cache_write_tokens": row[4], "calls": row[5],
            "messages": msgs, "tool_calls": tools, "models_used": sorted(models),
            "subagent_cost_usd": round(sub, 6), "estimated_price_calls": row[6],
            "db_found": True}


def stream_evidence(out, err):
    """What the run's own stdout/stderr proves.

    Only the rate-limit counter is load-bearing. On `main` today a 429 has no
    structured home — `providers/retry.rs` logs `Request failed, retrying` and
    the error's Debug rendering carries `RateLimitExceeded`, so that pair is
    the only honest signal available. Counted as such, and named after the log
    line it comes from rather than dressed up as a metric.
    """
    blob = (out or "") + "\n" + (err or "")
    return {
        "rate_limit_retries": sum(
            1 for line in blob.splitlines()
            if "Request failed, retrying" in line and "RateLimitExceeded" in line),
        "retry_lines": sum(1 for line in blob.splitlines()
                           if "Request failed, retrying" in line),
        "stdout_bytes": len(out or ""),
        "stderr_bytes": len(err or ""),
    }


# ── grade ───────────────────────────────────────────────────────────────────

def cmd_grade(args):
    man = load_manifest(args.workdir)
    outdir = os.path.join(args.workdir, "runs", args.label)
    for task in man["tasks"]:
        rec_path = os.path.join(outdir, f"{task['name']}.json")
        if not os.path.exists(rec_path):
            print(f"skip {task['name']} (no run record)")
            continue
        with open(rec_path) as fh:
            rec = json.load(fh)

        ws_label = os.path.join(outdir, "ws", task["name"])
        # Ground truth: a PRISTINE exercise + only the agent's solution files.
        # Whatever the agent did to the tests is irrelevant to the verdict —
        # and recorded separately below.
        grade_dir = os.path.join(outdir, "grade", task["name"])
        shutil.rmtree(grade_dir, ignore_errors=True)
        shutil.copytree(os.path.join(args.workdir, "pristine", task["name"]), grade_dir)
        shutil.rmtree(os.path.join(grade_dir, ".meta"), ignore_errors=True)

        missing = []
        for sol in task["solution_files"]:
            src = os.path.join(ws_label, sol)
            if os.path.exists(src):
                shutil.copy2(src, os.path.join(grade_dir, sol))
            else:
                missing.append(sol)

        tampered = {}
        for t, want in task["test_sha256"].items():
            got_path = os.path.join(ws_label, t)
            got = sha256_file(got_path) if os.path.exists(got_path) else None
            if got != want:
                tampered[t] = "deleted" if got is None else "modified"

        try:
            proc = subprocess.run(man["test_cmd"], cwd=grade_dir, capture_output=True,
                                  text=True, timeout=args.test_timeout)
            passed = proc.returncode == 0
            tail = (proc.stdout + proc.stderr)[-4000:]
            test_timeout = False
        except subprocess.TimeoutExpired:
            passed, tail, test_timeout = False, "TEST TIMEOUT", True

        rec["grade"] = {
            "passed": passed,
            "missing_solution_files": missing,
            "test_files_tampered": tampered,
            "test_timeout": test_timeout,
            "test_output_tail": tail,
            "graded_at": now(),
            "method": "agent's solution file(s) copied into a pristine copy of "
                      "the exercise; suite's own tests run there",
        }
        with open(rec_path, "w") as fh:
            json.dump(rec, fh, indent=2)
        flag = "PASS" if passed else "FAIL"
        if tampered:
            flag += f" (TEST TAMPER: {tampered})"
        print(f"  {task['name']:<24} {flag}")


# ── report ──────────────────────────────────────────────────────────────────

def cmd_report(args):
    man = load_manifest(args.workdir)
    runs_root = os.path.join(args.workdir, "runs")
    labels = sorted(d for d in os.listdir(runs_root)
                    if os.path.isdir(os.path.join(runs_root, d))) if os.path.isdir(runs_root) else []

    lines = []
    A = lines.append
    A(f"# Aider polyglot ({man['lang']}) through the Permagent coding harness")
    A("")
    A(f"- suite: `{man['suite_repo']}` @ `{man['suite_sha']}`")
    A(f"- harness commit: `{man['harness_sha']}`")
    n_tasks = len(man["tasks"])
    A(f"- selection: `{man['selection']}`, seed `{man['seed']}`, "
      f"{n_tasks} of {man['population_size']} {man['lang']} exercises")
    A(f"- grading: {man['test_cmd']} in a pristine copy; agent's solution files only")
    A(f"- prepared: {man['created_at']}")
    A("")
    if n_tasks < 30:
        A(f"> **PILOT — n={n_tasks}.** This is a smoke test of the "
          f"measurement path, not a benchmark result. A single task is 10% of "
          f"this number; the confidence interval is wider than any difference "
          f"shown. Do not quote it as a score.")
        A("")

    A("| label | model(s) actually billed | pass@1 | passed/n | cost | median wall | "
      "total wall | tool calls | test tamper | timeouts | rate-limit retries |")
    A("|---|---|---|---|---|---|---|---|---|---|---|")
    for label in labels:
        recs = []
        d = os.path.join(runs_root, label)
        for f in sorted(os.listdir(d)):
            if f.endswith(".json"):
                with open(os.path.join(d, f)) as fh:
                    recs.append(json.load(fh))
        if not recs:
            continue
        graded = [r for r in recs if "grade" in r]
        n = len(graded)
        p = sum(1 for r in graded if r["grade"]["passed"])
        walls = sorted(r["wall_seconds"] for r in recs)
        med = walls[len(walls) // 2] if walls else 0
        tot = sum(walls)
        tamp = sum(1 for r in graded if r["grade"]["test_files_tampered"])
        tmo = sum(1 for r in recs if r.get("timed_out"))
        rl = sum(r["stream_evidence"]["rate_limit_retries"] for r in recs)
        cost = sum(r["ledger"]["cost_usd"] or 0 for r in recs)
        priced = sum(1 for r in recs if r["ledger"]["cost_usd"] is not None)
        tools = sum(r["ledger"]["tool_calls"] or 0 for r in recs)
        models = sorted({m for r in recs for m in r["ledger"]["models_used"]}) \
            or [recs[0].get("model_requested") or "?"]
        rate = f"{100.0 * p / n:.0f}%" if n else "—"
        cost_cell = f"${cost:.2f}" + ("" if priced == len(recs) else f" ({priced}/{len(recs)} priced)")
        A(f"| {label} | {', '.join(f'`{m}`' for m in models)} | {rate} | {p}/{n} | "
          f"{cost_cell} | {med:.0f}s | {tot/60:.0f}m | {tools} | {tamp} | {tmo} | {rl} |")
    A("")
    A("## Per task")
    A("")
    A("| task | " + " | ".join(labels) + " |")
    A("|---|" + "---|" * len(labels))
    for task in man["tasks"]:
        cells = []
        for label in labels:
            p = os.path.join(runs_root, label, f"{task['name']}.json")
            if not os.path.exists(p):
                cells.append("—")
                continue
            with open(p) as fh:
                r = json.load(fh)
            g = r.get("grade")
            if not g:
                cells.append("ungraded")
            else:
                c = "PASS" if g["passed"] else "FAIL"
                if g["test_files_tampered"]:
                    c += " ⚠tamper"
                if r.get("timed_out"):
                    c += " ⏱"
                cells.append(f"{c} ({r['wall_seconds']:.0f}s)")
        A(f"| `{task['name']}` | " + " | ".join(cells) + " |")
    A("")
    A("## Not directly comparable to the published Aider polyglot leaderboard")
    A("")
    A("- **Different task set.** The leaderboard is all 225 exercises across six "
      "languages. This is a seeded subset of "
      f"{n_tasks} of the {man['population_size']} {man['lang']} exercises only.")
    A("- **Different protocol.** Aider's benchmark allows two attempts, the second "
      "with the test output fed back. The Permagent harness runs its own agentic "
      "loop with a verify tool and an unbounded-until-max-turns retry budget. "
      "More iterations are available here than the leaderboard allows.")
    A("- **Different scaffold.** Aider edits via search/replace blocks with no "
      "shell. This harness has a shell, a repo map, structured search, and a "
      "cross-model reviewer. The number measures the harness, not the model.")
    A("- **Contamination is unbounded.** These are public Exercism exercises with "
      "public solutions, frozen since 2024-12-22. Every model tested here was "
      "trained after that. Treat the absolute number as an upper bound.")

    out = "\n".join(lines) + "\n"
    if args.out:
        with open(args.out, "w") as fh:
            fh.write(out)
        print(f"wrote {args.out}")
    else:
        print(out)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("prepare")
    p.add_argument("--suite", required=True, help="clone of Aider-AI/polyglot-benchmark")
    p.add_argument("--workdir", required=True)
    p.add_argument("--lang", default="python", choices=sorted(LANG_RUNNERS))
    p.add_argument("--n", type=int, default=10)
    p.add_argument("--seed", type=int, default=20260825)
    p.set_defaults(func=cmd_prepare)

    p = sub.add_parser("run")
    p.add_argument("--workdir", required=True)
    p.add_argument("--label", required=True)
    p.add_argument("--binary", default="permagent")
    p.add_argument("--recipe-file", default=os.path.join(
        os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__)))),
        "crates", "goose-cli", "src", "recipes", "builtin", "permagent-coding.yaml"),
        help="the harness recipe under test; copied verbatim, prompt appended")
    p.add_argument("--provider")
    p.add_argument("--model")
    p.add_argument("--max-turns", type=int, default=40)
    p.add_argument("--timeout", type=int, default=1200, help="per-task seconds")
    p.add_argument("--force", action="store_true")
    p.set_defaults(func=cmd_run)

    p = sub.add_parser("grade")
    p.add_argument("--workdir", required=True)
    p.add_argument("--label", required=True)
    p.add_argument("--test-timeout", type=int, default=120)
    p.set_defaults(func=cmd_grade)

    p = sub.add_parser("report")
    p.add_argument("--workdir", required=True)
    p.add_argument("--out")
    p.set_defaults(func=cmd_report)

    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
