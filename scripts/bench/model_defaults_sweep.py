#!/usr/bin/env python3
"""Task-major sweep of the Permagent coding harness across model candidates.

Mirrors crates/permagent-eval's contract exactly (same argv, same env, same
isolated PERMAGENT_PATH_ROOT + cost_ledger read, same oracle overlay), but runs
TASK-MAJOR with a measured-spend stop so an early stop leaves a complete N-way
comparison on fewer tasks rather than a ragged one on all of them.

Never sets PERMAGENT_DISABLE_KEYRING: the signed bundled CLI reads the keychain
without prompting, and no secret ever passes through this process.
"""
import argparse, json, os, re, shutil, sqlite3, subprocess, sys, tempfile, time

# The SIGNED bundled CLI. A freshly built dev binary gets an ad-hoc,
# content-hashed code identity, so macOS re-prompts for the keychain on
# every rebuild; the bundled one has a stable Developer ID and an ACL that
# sticks. Override with PERMAGENT_BIN.
BIN = os.environ.get("PERMAGENT_BIN",
                     "/Applications/Permagent.app/Contents/MacOS/permagent")
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
TASKS = os.path.join(REPO, "crates", "permagent-eval", "tasks")
TURNS = os.path.join(REPO, "crates", "goose-server", "tests", "fixtures",
                     "chat_bench_turns.json")
SCRATCH = os.environ.get("MODEL_DEFAULTS_SCRATCH", os.path.join(tempfile.gettempdir(),
                                                               "model-defaults-bench"))

RUNS = os.path.join(SCRATCH, "runs")

# Cheapest first, so a row that has to be abandoned loses the dearest runs last.
# Priced per the pilot's shape (~720k input / ~7k output tokens per task) at each
# provider's canonical rates; used only to decide whether a ROW fits the cap.
CANDIDATES = [
    ("dschat",    "custom_deepseek", "deepseek-chat",             0.33),
    ("haiku",     "anthropic",       "claude-haiku-4-5-20251001", 0.30),
    ("kimi25",    "moonshot",        "kimi-k2.5",                 0.45),
    ("gpt54mini", "openai",          "gpt-5.4-mini",              0.54),
    ("sonnet5",   "anthropic",       "claude-sonnet-5",           0.90),
    ("glm53",     "zai",             "glm-5.3",                   1.05),
]
PACK_ROLES = ("EDIT", "HARD", "MECHANICAL", "LOCAL")

RECIPE = os.environ.get("MODEL_DEFAULTS_RECIPE",
                        os.path.join(SCRATCH, "coding-recipe.yaml"))

RATE_LIMIT_RE = re.compile(r"rate[ _-]?limit|429|too many requests", re.I)
# The CLI renders each tool invocation as "  \u25b8 <tool>" (optionally
# "\u25b8 [subagent:<id>] <tool>"); see print_tool_header and
# render_subagent_tool_call in crates/goose-cli/src/session/output.rs. The glyph
# is the one marker both the direct and the subagent path share.
TOOL_RE = re.compile(r"^\s*\u25b8\s*(?:\[subagent:[^\]]*\]\s*)?([a-z_]+)", re.M)


def load_tasks():
    out = []
    for name in sorted(os.listdir(TASKS)):
        if not name.startswith("defaults-"):
            continue
        d = os.path.join(TASKS, name)
        spec = {}
        for line in open(os.path.join(d, "task.yaml")):
            m = re.match(r"^([a-z_]+):\s*(.*)$", line)
            if m and m.group(1) != "prompt":
                spec[m.group(1)] = m.group(2).strip()
        prompt = open(os.path.join(d, "task.yaml")).read().split("prompt: |", 1)[1]
        prompt = "\n".join(l[2:] if l.startswith("  ") else l for l in prompt.strip("\n").split("\n"))
        lang = re.search(r"lang=(\w+)", open(os.path.join(d, "task.yaml")).read())
        out.append(dict(id=name, dir=d, prompt=prompt.strip(),
                        max_turns=int(spec.get("max_turns", 25)),
                        harness_timeout=int(spec.get("harness_timeout_secs", 600)),
                        oracle_timeout=int(spec.get("oracle_timeout_secs", 180)),
                        lang=lang.group(1) if lang else "?"))
    return out


def task_recipe(task, run_dir):
    """The shipped coding recipe with this task's prompt embedded.

    `--recipe` and `-t/--text` are mutually exclusive in the CLI (see the
    conflicts_with rules on InputOptions), so a headless recipe run takes its
    user prompt from the recipe's own `prompt:` field. The body is the SHIPPED
    recipe verbatim (`permagent run --recipe permagent-coding --render-recipe`),
    title included — `is_coding_harness_recipe` matches on the title, and that is
    what gates repo-map injection, so keeping it identical keeps the harness the
    real one.
    """
    body = open(RECIPE).read().rstrip("\n")
    lines = ["prompt: |"] + ["  " + l for l in task["prompt"].split("\n")]
    path = os.path.join(run_dir, "recipe.yaml")
    open(path, "w").write(body + "\n" + "\n".join(lines) + "\n")
    return path


def ledger(data_root):
    db = os.path.join(data_root, "spectral", "permagent.db")
    if not os.path.exists(db):
        return dict(usd=None, rows=0, input=None, output=None, cache_read=None, estimated=False)
    try:
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        r = con.execute(
            "SELECT COUNT(*), SUM(cost_usd), SUM(input_tokens), SUM(output_tokens),"
            " SUM(cache_read_tokens), SUM(cache_write_tokens), MAX(is_estimated)"
            " FROM cost_ledger").fetchone()
        # `deepseek-chat` is an alias: the ledger bills it as `deepseek-v4-flash`.
        # Report rows by the model actually billed, not the id we asked for.
        billed = con.execute(
            "SELECT provider, model, COUNT(*), SUM(cost_usd), SUM(input_tokens),"
            " SUM(output_tokens), SUM(cache_read_tokens) FROM cost_ledger"
            " GROUP BY provider, model ORDER BY COUNT(*) DESC").fetchall()
        con.close()
    except sqlite3.Error as e:
        return dict(usd=None, rows=0, input=None, output=None, cache_read=None,
                    cache_write=None, estimated=False, billed=[], error=str(e))
    return dict(rows=r[0] or 0, usd=r[1], input=r[2], output=r[3], cache_read=r[4],
                cache_write=r[5], estimated=bool(r[6]),
                billed=[dict(provider=b[0], model=b[1], calls=b[2], usd=b[3],
                             input=b[4], output=b[5], cache_read=b[6]) for b in billed])


def run_one(task, label, provider, model, keep=False):
    stamp = f"{label}-{task['id']}-{int(time.time()*1000)}"
    run_dir = os.path.join(RUNS, stamp)
    work = os.path.join(run_dir, "workspace")
    data = os.path.join(run_dir, "data")
    os.makedirs(data, exist_ok=True)
    shutil.copytree(os.path.join(task["dir"], "workspace"), work)

    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("PERMAGENT_PACK_", "PERMAGENT_CHEAP_", "PERMAGENT_BUDGET_"))}
    env.pop("PERMAGENT_DISABLE_KEYRING", None)
    env.pop("CARGO_TARGET_DIR", None)
    env["PERMAGENT_PATH_ROOT"] = data
    env["GOOSE_MODE"] = "auto"
    for role in PACK_ROLES:
        env[f"PERMAGENT_PACK_{role}_PROVIDER"] = provider
        env[f"PERMAGENT_PACK_{role}_MODEL"] = model

    recipe = task_recipe(task, run_dir)
    argv = [BIN, "run", "--recipe", recipe,
            "--provider", provider, "--model", model,
            "--output-format", "text", "--max-turns", str(task["max_turns"])]
    t0 = time.time()
    timed_out = False
    try:
        p = subprocess.run(argv, cwd=work, env=env, capture_output=True, text=True,
                           timeout=task["harness_timeout"])
        rc, log = p.returncode, (p.stdout or "") + (p.stderr or "")
    except subprocess.TimeoutExpired as e:
        rc, timed_out = None, True
        log = ((e.stdout or b"").decode("utf8", "replace") if isinstance(e.stdout, bytes) else (e.stdout or "")) + \
              ((e.stderr or b"").decode("utf8", "replace") if isinstance(e.stderr, bytes) else (e.stderr or ""))
    secs = round(time.time() - t0, 1)
    os.makedirs(os.path.join(run_dir, "logs"), exist_ok=True)
    open(os.path.join(run_dir, "logs", "harness.log"), "w").write(log)

    # Tamper-proof grading: overlay the pristine oracle, then run it.
    shutil.copytree(os.path.join(task["dir"], "oracle"), work, dirs_exist_ok=True)
    oenv = {k: v for k, v in env.items() if k != "CARGO_TARGET_DIR"}
    try:
        o = subprocess.run(["bash", "oracle_check.sh"], cwd=work, env=oenv,
                           capture_output=True, text=True, timeout=task["oracle_timeout"])
        solved, otail = o.returncode == 0, (o.stdout or "")[-500:]
    except subprocess.TimeoutExpired:
        solved, otail = False, "ORACLE TIMEOUT"

    led = ledger(data)
    tools = TOOL_RE.findall(log)
    result = dict(task=task["id"], lang=task["lang"], candidate=label, provider=provider,
                  model=model, solved=solved, secs=secs, exit=rc, timed_out=timed_out,
                  tool_calls=len(tools), tool_names=tools[:60],
                  rate_limit_events=len(RATE_LIMIT_RE.findall(log)),
                  log_chars=len(log), oracle_tail=otail, **{f"ledger_{k}": v for k, v in led.items()})
    if not keep:
        shutil.rmtree(run_dir, ignore_errors=True)
    return result


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--budget-usd", type=float, default=10.0)
    ap.add_argument("--out", default=os.path.join(SCRATCH, "results.json"))
    ap.add_argument("--tasks", default="")
    ap.add_argument("--candidates", default="")
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--max-turns", type=int, default=0,
                    help="override every task's turn ceiling (0 = use task.yaml)")
    args = ap.parse_args()

    tasks = load_tasks()
    if args.max_turns:
        for t in tasks:
            t["max_turns"] = args.max_turns
    if args.tasks:
        # Honour the ORDER given, not alphabetical: the sweep is task-major with a
        # spend stop, so the first rows must be the ones worth having if it stops.
        by_id = {t["id"]: t for t in tasks}
        tasks = [by_id[w] for w in args.tasks.split(",") if w in by_id]
    cands = CANDIDATES
    if args.candidates:
        want = set(args.candidates.split(","))
        cands = [c for c in cands if c[0] in want]

    print(f"{len(tasks)} tasks x {len(cands)} candidates = {len(tasks)*len(cands)} runs, "
          f"cap ${args.budget_usd:.2f}")
    for t in tasks:
        print(f"  {t['id']:52} {t['lang']} max_turns={t['max_turns']}")
    print("  a complete row is projected at $%.2f" % sum(c[3] for c in cands))
    if args.dry_run:
        return

    os.makedirs(RUNS, exist_ok=True)
    results, spent = [], 0.0
    stopped = None
    seen = {}  # label -> [measured costs], so the row estimate improves as we go
    for t in tasks:
        # Stop at a ROW boundary, never mid-row: a half-measured row is not a
        # comparison, and the whole point of going task-major is that whatever we
        # finish is a complete N-way result.
        row_est = sum(
            (sum(seen[c[0]]) / len(seen[c[0]])) if seen.get(c[0]) else c[3]
            for c in cands
        )
        if spent + row_est > args.budget_usd:
            stopped = (f"BUDGET STOP before {t['id']}: spent ${spent:.2f}, next row "
                       f"projected ${row_est:.2f}, cap ${args.budget_usd:.2f}")
            print(stopped)
            break
        for label, prov, model, _est in cands:
            r = run_one(t, label, prov, model, keep=args.keep)
            if r["ledger_usd"]:
                spent += r["ledger_usd"]
                seen.setdefault(label, []).append(r["ledger_usd"])
            results.append(r)
            print(f"{t['id'][:44]:44} {label:10} solved={str(r['solved']):5} "
                  f"{r['secs']:6.1f}s tools={r['tool_calls']:3} "
                  f"${(r['ledger_usd'] or 0):.4f} calls={r['ledger_rows']} "
                  f"rl={r['rate_limit_events']} total=${spent:.2f}", flush=True)
            json.dump(dict(results=results, spent=spent, stopped=stopped),
                      open(args.out, "w"), indent=1)

    json.dump(dict(results=results, spent=spent, stopped=stopped),
              open(args.out, "w"), indent=1)
    print(f"\nTOTAL MEASURED SPEND ${spent:.4f} over {len(results)} runs -> {args.out}")


if __name__ == "__main__":
    main()
