#!/usr/bin/env python3
"""Chat-turn bench driven through the SIGNED bundled CLI.

Why the CLI and not the daemon's /reply: a freshly built dev binary gets an
ad-hoc, content-hashed code identity, so macOS puts up a keychain dialog the
first time it reads a secret, and an unattended run hangs on it. The bundled CLI
has a stable Developer ID and an existing ACL. The trade-off is stated in the
report: this is the CLI session prompt, not /reply's, so it lacks the daemon's
ambient-context and recall injection. Everything downstream of that — the
provider stack, the ~17k-token capability inventory, the tool schemas, the
cache-control layout — is identical, because both go through Agent::reply.

Turns run in one sequence per candidate so the provider-side prefix cache warms
the way it does in a real conversation: turn 1 is the cold number, turns 2..N the
warm ones. Each turn is its own process, so no in-process state carries over.
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

# Haiku first: Jesse's decision is already made, so the measurement that MUST
# land is the one that confirms or refuses it. The alternatives follow
# cheapest-first, so a spend stop drops the dearest comparison, not the answer.
CANDIDATES = [
    ("haiku",     "anthropic",       "claude-haiku-4-5-20251001"),
    ("dschat",    "custom_deepseek", "deepseek-chat"),
    ("kimi25",    "moonshot",        "kimi-k2.5"),
    ("gpt54mini", "openai",          "gpt-5.4-mini"),
    ("glm53",     "zai",             "glm-5.3"),
    ("sonnet5",   "anthropic",       "claude-sonnet-5"),
]
# A sentence boundary a reader would recognise: terminal punctuation followed by
# whitespace or end of string, at least 12 characters in so "Hi." or a bare
# "OK." does not count as a sentence.
SENTENCE_RE = re.compile(r"[.!?](?:\s|$)")


def seed_config(data_root):
    """Give the isolated root the operator's EXTENSION set, and nothing else.

    An empty root loads no extensions, which would understate the chat prompt:
    the ~17k-token capability inventory is static and present either way, but the
    tool SCHEMAS are not, and they are a real part of what a chat turn pays for.
    So the operator's `extensions:` block is copied in. No secret is copied —
    provider keys live in the OS keychain, not in this file — and the
    provider/model come from the CLI flags, which outrank the file.
    """
    src = os.path.expanduser("~/.permagent/config.yaml")
    if not os.path.exists(src):
        return False
    keep, depth_ok = [], False
    for line in open(src):
        if line.startswith("extensions:"):
            depth_ok = True
            keep.append(line)
            continue
        if depth_ok:
            # The block ends at the next column-0 key.
            if line.strip() and not line[0].isspace():
                depth_ok = False
                continue
            keep.append(line)
    if not keep:
        return False
    os.makedirs(data_root, exist_ok=True)
    with open(os.path.join(data_root, "config.yaml"), "w") as fh:
        fh.writelines(keep)
        # Refuse tool EXECUTION: headless runs decline to auto-approve, so a tool
        # request is still emitted and measurable but nothing acts on the machine.
        fh.write("GOOSE_MODE: approve\n")
    return True


def base_env(data_root):
    env = {k: v for k, v in os.environ.items()
           if not k.startswith(("PERMAGENT_PACK_", "PERMAGENT_CHEAP_", "PERMAGENT_BUDGET_"))}
    env.pop("PERMAGENT_DISABLE_KEYRING", None)
    env.pop("CARGO_TARGET_DIR", None)
    env["PERMAGENT_PATH_ROOT"] = data_root
    env["GOOSE_MODE"] = "approve"
    return env


def ledger(data_root):
    db = os.path.join(data_root, "spectral", "permagent.db")
    if not os.path.exists(db):
        return {}
    try:
        con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
        r = con.execute(
            "SELECT COUNT(*), SUM(cost_usd), SUM(input_tokens), SUM(output_tokens),"
            " SUM(cache_read_tokens) FROM cost_ledger").fetchone()
        billed = con.execute(
            "SELECT provider, model FROM cost_ledger LIMIT 1").fetchone()
        con.close()
    except sqlite3.Error:
        return {}
    return dict(calls=r[0] or 0, usd=r[1], input=r[2], output=r[3], cache_read=r[4],
                billed_provider=(billed or [None, None])[0],
                billed_model=(billed or [None, None])[1])


def startup_cost_ms(provider, model, data_root):
    """How much of a turn's latency is the CLI starting up rather than the model.

    Measured by asking for a model id that cannot exist: the process still loads
    extensions and builds the whole system prompt, then the provider refuses. No
    tokens are billed. Reported as a labelled constant and subtracted nowhere —
    the reader is told what it is and can subtract it themselves.
    """
    env = base_env(data_root)
    t0 = time.time()
    try:
        subprocess.run([BIN, "run", "--no-session", "--provider", provider,
                        "--model", model + "-does-not-exist-bench",
                        "-t", "x"], env=env, capture_output=True, text=True, timeout=180)
    except subprocess.TimeoutExpired:
        return None
    return int((time.time() - t0) * 1000)


def run_turn(turn, label, provider, model, data_root, idx):
    env = base_env(data_root)
    argv = [BIN, "run", "--no-session", "--provider", provider, "--model", model,
            "--output-format", "stream-json", "--max-turns", "3", "-t", turn["prompt"]]
    t0 = time.time()
    ttft = first_sentence = None
    text = []
    thought = False
    tools = []
    err = None
    try:
        p = subprocess.Popen(argv, env=env, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, bufsize=1)
        for line in p.stdout:
            line = line.strip()
            if not line.startswith("{"):
                continue
            try:
                ev = json.loads(line)
            except json.JSONDecodeError:
                continue
            msg = ev.get("message") or {}
            for c in msg.get("content", []) or []:
                kind = c.get("type")
                if kind in ("thinking", "redactedThinking", "redacted_thinking"):
                    thought = True
                elif kind == "text" and (c.get("text") or "").strip():
                    if ttft is None:
                        ttft = int((time.time() - t0) * 1000)
                    text.append(c["text"])
                    if first_sentence is None:
                        joined = "".join(text)
                        m = SENTENCE_RE.search(joined, 12)
                        if m:
                            first_sentence = int((time.time() - t0) * 1000)
                elif kind in ("toolRequest", "tool_request", "toolUse", "tool_use"):
                    name = (((c.get("toolCall") or {}).get("value") or {}).get("name")
                            or c.get("name"))
                    if name:
                        tools.append(str(name).split("__")[-1])
        p.wait(timeout=240)
        err = (p.stderr.read() or "")[-300:] if p.returncode else None
    except Exception as e:                                  # noqa: BLE001
        err = f"{type(e).__name__}: {e}"
    total = int((time.time() - t0) * 1000)
    body = "".join(text).strip()
    expect = turn.get("expect_tool")
    return dict(id=turn["id"], kind=turn.get("kind"), candidate=label, index=idx,
                ttft_ms=ttft, first_sentence_ms=first_sentence, total_ms=total,
                thought=thought, tools=tools,
                tool_correct=(None if not expect else (expect in tools)),
                silent_tool_call=bool(tools) and not body,
                reply=body[:1500], chars=len(body), error=err)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--budget-usd", type=float, default=5.0)
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--candidates", default="")
    ap.add_argument("--out", default=os.path.join(SCRATCH, "results.json"))
    args = ap.parse_args()

    fixture = json.load(open(TURNS))
    turns = fixture["turns"][: args.limit] if args.limit else fixture["turns"]
    cands = CANDIDATES
    if args.candidates:
        want = set(args.candidates.split(","))
        cands = [c for c in cands if c[0] in want]
    print(f"{len(turns)} turns x {len(cands)} candidates, cap ${args.budget_usd:.2f}")

    os.makedirs(RUNS, exist_ok=True)
    out = dict(turns=len(turns), results=[], per_candidate={}, spent=0.0, stopped=None)
    spent = 0.0
    for label, prov, model in cands:
        if spent > args.budget_usd:
            out["stopped"] = f"BUDGET STOP before {label}: spent ${spent:.2f}"
            print(out["stopped"]); break
        data_root = os.path.join(RUNS, label)
        shutil.rmtree(data_root, ignore_errors=True)
        os.makedirs(data_root, exist_ok=True)
        seeded = seed_config(data_root)
        start_ms = startup_cost_ms(prov, model, os.path.join(RUNS, label + "-startup"))
        rows = []
        for i, t in enumerate(turns):
            r = run_turn(t, label, prov, model, data_root, i)
            rows.append(r)
            out["results"].append(r)
            print(f"{label:10} {t['id']} {str(t.get('kind'))[:6]:6} "
                  f"ttft={str(r['ttft_ms']):>6} total={r['total_ms']:>6} "
                  f"tools={','.join(r['tools'])[:24]:24} "
                  f"{'THINKS' if r['thought'] else '      '} "
                  f"{'ERR' if r['error'] else ''}", flush=True)
            json.dump(out, open(args.out, "w"), indent=1)
        led = ledger(data_root)
        if led.get("usd"):
            spent += led["usd"]
        out["per_candidate"][label] = dict(provider=prov, model=model,
                                           startup_ms=start_ms, ledger=led,
                                           extensions_seeded=seeded)
        out["spent"] = spent
        print(f"== {label}: ${led.get('usd') or 0:.4f} over {led.get('calls')} calls, "
              f"billed as {led.get('billed_model')}, startup {start_ms} ms, "
              f"running total ${spent:.2f}", flush=True)
        json.dump(out, open(args.out, "w"), indent=1)
    json.dump(out, open(args.out, "w"), indent=1)
    print(f"\nTOTAL MEASURED SPEND ${spent:.4f} -> {args.out}")


if __name__ == "__main__":
    main()
