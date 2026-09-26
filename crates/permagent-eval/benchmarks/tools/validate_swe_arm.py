#!/usr/bin/env python3
"""Validate the pinned SWE-arm slice: gold patch passes (twice), empty patch fails.

Two modes:

  --static   Registry-only (nothing pulled): for every instance, the pinned digest
             still resolves, is linux/arm64, and has the layout the dataset's
             eval_script assumes (WorkingDir /testbed, /opt/miniconda3 on PATH,
             label swe-bench.instance_id == the instance).

  --remote H Pull each image on the SSH host H (Docker there), run gold twice and
             an empty patch once, grade with swebench's own grader, remove the
             image, and write one JSON line per instance to --results. Refuses to
             pull when the host's free disk would drop under --min-free-gib.

The gold patch is read from the pinned dataset parquet at run time and never
written into the repository. Needs `requests pyyaml pandas pyarrow
huggingface_hub swebench datasets` (a current `datasets`; old ones break on new
pyarrow).

    python validate_swe_arm.py --static
    python validate_swe_arm.py --remote m1 --results /tmp/swe-arm-validation.jsonl [--only ID ...]
"""
from __future__ import annotations

import argparse
import base64
import json
import os
import shlex
import subprocess
import sys
import tempfile
import time

import requests
import yaml

HERE = os.path.dirname(os.path.abspath(__file__))
SLICE = os.path.join(HERE, "..", "swe-arm-30.yaml")
INSTANCES = os.path.join(HERE, "..", "swe-arm-30.instances.jsonl")
REMOTE_PRELUDE = (
    "export PATH=/opt/homebrew/bin:$HOME/.local/bin:$PATH; "
    "export DOCKER_HOST=unix://$HOME/.colima/default/docker.sock; "
)
ACCEPT = "application/vnd.docker.distribution.manifest.v2+json,application/vnd.oci.image.manifest.v1+json"


def load_slice():
    with open(SLICE) as f:
        doc = yaml.safe_load(f)
    rows = {}
    with open(INSTANCES) as f:
        for line in f:
            r = json.loads(line)
            rows[r["instance_id"]] = r
    return doc, rows


def static_check(entry) -> tuple[bool, str]:
    name = entry["image"].removeprefix("ghcr.io/")
    tok = requests.get("https://ghcr.io/token", params={"scope": f"repository:{name}:pull"}, timeout=30).json()
    h = {"Authorization": f"Bearer {tok['token']}", "Accept": ACCEPT}
    r = requests.get(f"https://ghcr.io/v2/{name}/manifests/{entry['image_digest']}", headers=h, timeout=30)
    if r.status_code != 200:
        return False, f"digest does not resolve ({r.status_code})"
    cfg = requests.get(f"https://ghcr.io/v2/{name}/blobs/{r.json()['config']['digest']}", headers=h, timeout=60).json()
    c = cfg.get("config", {})
    problems = []
    if cfg.get("architecture") != "arm64" or cfg.get("os") != "linux":
        problems.append(f"platform {cfg.get('os')}/{cfg.get('architecture')}")
    if c.get("WorkingDir") != "/testbed":
        problems.append(f"WorkingDir {c.get('WorkingDir')}")
    if not any(e.startswith("PATH=/opt/miniconda3/bin") for e in c.get("Env", [])):
        problems.append("no /opt/miniconda3 on PATH")
    if (c.get("Labels") or {}).get("swe-bench.instance_id") != entry["instance_id"]:
        problems.append("instance label mismatch")
    return (not problems), "; ".join(problems) or "ok"


def judge_script(patch: str, eval_script: str) -> str:
    """The same in-container script the Rust judge (src/swebench.rs) sends."""
    p64 = base64.encodebytes(patch.encode()).decode().rstrip("\n")
    e64 = base64.encodebytes(eval_script.encode()).decode().rstrip("\n")
    return (
        "set -u\nexec 2>&1\n"
        f"base64 -d > /tmp/permagent.patch <<'PERMAGENT_B64_EOF'\n{p64}\nPERMAGENT_B64_EOF\n"
        f"base64 -d > /tmp/permagent_eval.sh <<'PERMAGENT_B64_EOF'\n{e64}\nPERMAGENT_B64_EOF\n"
        "cd /testbed\n"
        "if [ ! -s /tmp/permagent.patch ]; then\n  echo '>>>>> Applied Patch (empty)'\n"
        "elif git apply --verbose /tmp/permagent.patch || patch --batch --fuzz=5 -p1 -i /tmp/permagent.patch; then\n"
        "  echo '>>>>> Applied Patch'\n"
        "else\n  echo '>>>>> Patch Apply Failed'\n  exit 3\nfi\n"
        "bash /tmp/permagent_eval.sh\n"
    )


def ssh(host, cmd, stdin=None, timeout=3600):
    return subprocess.run(["ssh", host, REMOTE_PRELUDE + cmd], input=stdin, capture_output=True,
                          text=True, timeout=timeout)


def free_gib(host) -> float:
    out = ssh(host, "df -k \"$HOME\" | tail -1 | awk '{print $4}'").stdout.strip()
    return int(out) / (1024 * 1024)


def grade(row, log_text) -> dict:
    from swebench.harness.grading import get_logs_eval, get_eval_tests_report, get_resolution_status
    from swebench.harness.constants import FAIL_TO_PASS, PASS_TO_PASS

    class Spec:  # the fields get_logs_eval reads
        log_parser = row["log_parser"]
        instance_id = row["instance_id"]
        repo = row["repo"]
    with tempfile.NamedTemporaryFile("w", suffix=".log", delete=False) as f:
        f.write(log_text)
    sm, ok = get_logs_eval(Spec, f.name)
    os.unlink(f.name)
    if not ok:
        return {"resolved": False, "log_ok": False}
    rep = get_eval_tests_report(sm, {FAIL_TO_PASS: row["FAIL_TO_PASS"], PASS_TO_PASS: row["PASS_TO_PASS"]})
    return {"resolved": get_resolution_status(rep) == "RESOLVED_FULL", "log_ok": True,
            "f2p_fail": rep[FAIL_TO_PASS]["failure"][:5], "p2p_fail": rep[PASS_TO_PASS]["failure"][:5]}


def remote_validate(host, entry, row, gold, min_free, run_timeout, keep_logs):
    iid, image = entry["instance_id"], f"{entry['image']}@{entry['image_digest']}"
    # Compressed layers plus the unpacked snapshot: ~4x compressed is a safe ceiling.
    need = entry["image_compressed_bytes"] * 4 / 2**30
    free = free_gib(host)
    if free - need < min_free:
        return {"instance_id": iid, "status": "skipped_disk", "free_gib": round(free, 1), "need_gib": round(need, 1)}
    res = {"instance_id": iid, "free_gib_before": round(free, 1)}
    t = time.time()
    p = ssh(host, f"docker pull -q {shlex.quote(image)}", timeout=3600)
    res["pull_secs"] = round(time.time() - t, 1)
    if p.returncode != 0:
        res["status"] = "pull_failed"
        res["error"] = p.stderr[-500:]
        return res
    try:
        runs = [("gold1", gold), ("gold2", gold), ("empty", "")]
        for label, patch in runs:
            name = f"permagent-validate-{iid.replace('__', '-')}-{label}"
            t = time.time()
            # No coreutils `timeout` on the Mac host: bound the run locally and
            # force-remove the named container if it overruns.
            try:
                stdout = ssh(host, f"docker run --rm -i --name {name} --memory 2g {shlex.quote(image)} bash -s",
                             stdin=judge_script(patch, row["eval_script"]), timeout=run_timeout).stdout
            except subprocess.TimeoutExpired:
                stdout = ">>>>> Tests Timed Out\n"
            ssh(host, f"docker rm -f {name} >/dev/null 2>&1 || true")
            secs = round(time.time() - t, 1)
            g = grade(row, stdout)
            res[label] = {"resolved": g["resolved"], "log_ok": g["log_ok"], "secs": secs,
                          "f2p_fail": g.get("f2p_fail", []), "p2p_fail": g.get("p2p_fail", [])}
            if keep_logs:
                with open(os.path.join(keep_logs, f"{iid}.{label}.log"), "w") as f:
                    f.write(stdout)
        ok = res["gold1"]["resolved"] and res["gold2"]["resolved"] and not res["empty"]["resolved"]
        res["status"] = "valid" if ok else "invalid"
    finally:
        ssh(host, f"docker rmi {shlex.quote(image)} >/dev/null 2>&1; colima ssh -- sudo fstrim /mnt/lima-colima >/dev/null 2>&1 || true")
        res["free_gib_after"] = round(free_gib(host), 1)
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--static", action="store_true")
    ap.add_argument("--remote")
    ap.add_argument("--results")
    ap.add_argument("--only", nargs="*")
    ap.add_argument("--min-free-gib", type=float, default=15.0)
    ap.add_argument("--run-timeout", type=int, default=1800)
    ap.add_argument("--keep-logs")
    args = ap.parse_args()
    doc, rows = load_slice()
    entries = [e for e in doc["instances"] if not args.only or e["instance_id"] in args.only]

    if args.static:
        bad = 0
        for e in entries:
            ok, why = static_check(e)
            bad += not ok
            print(f"{'OK ' if ok else 'BAD'} {e['instance_id']:<28} {why}")
        sys.exit(1 if bad else 0)

    if not (args.remote and args.results):
        ap.error("--remote needs --results")
    import pandas as pd
    from huggingface_hub import hf_hub_download
    ds = doc["dataset"]
    df = pd.read_parquet(hf_hub_download(ds["repo"], ds["file"], repo_type="dataset", revision=ds["revision"]))
    gold = dict(zip(df.instance_id, df.patch))
    with open(args.results, "a") as out:
        for e in entries:
            res = remote_validate(args.remote, e, rows[e["instance_id"]], gold[e["instance_id"]],
                                  args.min_free_gib, args.run_timeout, args.keep_logs)
            print(json.dumps(res), flush=True)
            out.write(json.dumps(res) + "\n")
            out.flush()


if __name__ == "__main__":
    main()
