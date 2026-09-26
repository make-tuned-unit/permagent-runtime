#!/usr/bin/env python3
"""Reproduce the pinned SWE-arm-30 selection (crates/permagent-eval/benchmarks/swe-arm-30.yaml).

Deterministic: the same pinned dataset revisions + the same registry state give the
same 30 + 6 IDs. Needs `huggingface_hub`, `pandas`, `pyarrow`, `requests`, `pyyaml`
(e.g. `uv venv .venv-bench && uv pip install huggingface_hub pandas pyarrow requests pyyaml`).

    python select_swe_arm30.py --out ../swe-arm-30.yaml --instances ../swe-arm-30.instances.jsonl

Nothing is pulled: arm64 availability is checked with registry manifest GETs only.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from concurrent.futures import ThreadPoolExecutor

import pandas as pd
import requests
import yaml
from huggingface_hub import hf_hub_download

# Source of truth for task rows: the SWE-bench org copy ships the per-instance
# `eval_script` and `log_parser` the judge replays, pinned here.
VERIFIED = ("SWE-bench/SWE-bench_Verified", "78f471bf655a3137b2e8a75af1501690ec009ec3")
# The copy named in huggingface-benchmarks.yaml; cross-checked row by row.
VERIFIED_PRINCETON = ("princeton-nlp/SWE-bench_Verified", "c104f840cc67f8b6eec6f759ebc8b2693d585d4a")
MINI = ("MariusHobbhahn/swe-bench-verified-mini", "b316c349947c29963fce3f4a65967c9807a4b673")
PARQUET = "data/test-00000-of-00001.parquet"

EXCLUDED_REPOS = {"scikit-learn/scikit-learn", "matplotlib/matplotlib", "pydata/xarray"}
EASY, MEDIUM = "<15 min fix", "15 min - 1 hour"
# (repo, bucket) -> how many core tasks; 12 easy + 18 medium, no repo above 8.
QUOTA = {
    ("django/django", EASY): 3, ("django/django", MEDIUM): 5,
    ("sphinx-doc/sphinx", EASY): 2, ("sphinx-doc/sphinx", MEDIUM): 4,
    ("sympy/sympy", EASY): 2, ("sympy/sympy", MEDIUM): 4,
    ("pytest-dev/pytest", EASY): 2, ("pytest-dev/pytest", MEDIUM): 2,
    ("astropy/astropy", EASY): 1, ("astropy/astropy", MEDIUM): 1,
    ("pylint-dev/pylint", EASY): 1, ("pylint-dev/pylint", MEDIUM): 1,
    ("psf/requests", EASY): 1, ("psf/requests", MEDIUM): 1,
}
# One spare per stratum most likely to need a replacement.
SPARES = [
    ("django/django", MEDIUM), ("sphinx-doc/sphinx", EASY), ("sympy/sympy", MEDIUM),
    ("pytest-dev/pytest", MEDIUM), ("psf/requests", EASY), ("astropy/astropy", MEDIUM),
]
IMAGE = "ghcr.io/epoch-research/swe-bench.eval.arm64.{iid}"
ACCEPT = ",".join([
    "application/vnd.oci.image.index.v1+json",
    "application/vnd.docker.distribution.manifest.v2+json",
    "application/vnd.oci.image.manifest.v1+json",
    "application/vnd.docker.distribution.manifest.list.v2+json",
])


def load(repo_rev):
    repo, rev = repo_rev
    path = hf_hub_download(repo, PARQUET, repo_type="dataset", revision=rev)
    return pd.read_parquet(path)


def as_list(v):
    return json.loads(v) if isinstance(v, str) else list(v)


def probe(iid: str) -> dict:
    """Manifest + config GET for the arm64 image (no layers are downloaded)."""
    name = f"epoch-research/swe-bench.eval.arm64.{iid}"
    tok = requests.get("https://ghcr.io/token", params={"scope": f"repository:{name}:pull"}, timeout=30).json()
    if "token" not in tok:
        return {"available": False}
    h = {"Authorization": f"Bearer {tok['token']}", "Accept": ACCEPT}
    r = requests.get(f"https://ghcr.io/v2/{name}/manifests/latest", headers=h, timeout=30)
    if r.status_code != 200:
        return {"available": False}
    m = r.json()
    cfg = requests.get(f"https://ghcr.io/v2/{name}/blobs/{m['config']['digest']}", headers=h, timeout=60).json()
    return {
        "available": cfg.get("architecture") == "arm64",
        "digest": r.headers["Docker-Content-Digest"],
        "compressed_bytes": sum(l["size"] for l in m.get("layers", [])),
        "workdir": cfg.get("config", {}).get("WorkingDir"),
    }


def stable_key(iid: str) -> str:
    return hashlib.sha256(iid.encode()).hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", required=True)
    ap.add_argument("--instances", required=True)
    args = ap.parse_args()

    v, vp, mini = load(VERIFIED), load(VERIFIED_PRINCETON), load(MINI)
    merged = v.merge(vp, on="instance_id", suffixes=("", "_p"))
    # Rows whose test lists differ between the two copies are ambiguous: skip them.
    disagree = {
        r.instance_id for r in merged.itertuples()
        if sorted(as_list(r.FAIL_TO_PASS)) != sorted(as_list(r.FAIL_TO_PASS_p))
        or sorted(as_list(r.PASS_TO_PASS)) != sorted(as_list(r.PASS_TO_PASS_p))
    }
    pool = v[~v.repo.isin(EXCLUDED_REPOS) & v.difficulty.isin([EASY, MEDIUM]) & ~v.instance_id.isin(disagree)].copy()
    pool["in_mini"] = pool.instance_id.isin(set(mini.instance_id))
    pool["key"] = pool.instance_id.map(stable_key)
    # Mini members first (distribution-matched), then a stable hash order.
    pool = pool.sort_values(["in_mini", "key"], ascending=[False, True])

    with ThreadPoolExecutor(16) as ex:
        images = dict(zip(pool.instance_id, ex.map(probe, pool.instance_id)))
    pool = pool[pool.instance_id.map(lambda i: images[i]["available"])]

    chosen, spares = [], []
    for (repo, bucket), n in QUOTA.items():
        rows = pool[(pool.repo == repo) & (pool.difficulty == bucket)]
        chosen += list(rows.instance_id[:n])
    for repo, bucket in SPARES:
        rows = pool[(pool.repo == repo) & (pool.difficulty == bucket) & ~pool.instance_id.isin(chosen + spares)]
        spares.append(rows.instance_id.iloc[0])

    def entry(iid, role):
        r = pool[pool.instance_id == iid].iloc[0]
        img = images[iid]
        return {
            "instance_id": iid, "role": role, "repo": r.repo, "difficulty": r.difficulty,
            "version": str(r.version), "base_commit": r.base_commit, "in_verified_mini": bool(r.in_mini),
            "log_parser": r.log_parser,
            "image": IMAGE.format(iid=iid), "image_digest": img["digest"],
            "image_compressed_bytes": int(img["compressed_bytes"]),
            "fail_to_pass": len(as_list(r.FAIL_TO_PASS)), "pass_to_pass": len(as_list(r.PASS_TO_PASS)),
        }

    doc = {
        "id": "swe-arm-30",
        # Relative to this YAML. Holds the judge's inputs (no gold patches).
        "instances_file": os.path.basename(args.instances),
        "dataset": {"repo": VERIFIED[0], "revision": VERIFIED[1], "file": PARQUET},
        "cross_checked_against": {"repo": VERIFIED_PRINCETON[0], "revision": VERIFIED_PRINCETON[1]},
        "subset_source": {"repo": MINI[0], "revision": MINI[1]},
        "excluded_repos": sorted(EXCLUDED_REPOS),
        "instances": [entry(i, "core") for i in chosen] + [entry(i, "spare") for i in spares],
    }
    with open(args.out, "w") as f:
        yaml.safe_dump(doc, f, sort_keys=False, width=120)

    # The judge's inputs. The gold `patch` is deliberately NOT written: it would sit
    # on disk next to a harness that can read files.
    keep = ["instance_id", "repo", "base_commit", "environment_setup_commit", "version", "difficulty",
            "problem_statement", "hints_text", "created_at", "test_patch", "eval_script", "log_parser",
            "FAIL_TO_PASS", "PASS_TO_PASS"]
    with open(args.instances, "w") as f:
        for iid in chosen + spares:
            r = pool[pool.instance_id == iid].iloc[0]
            row = {k: (as_list(r[k]) if k in ("FAIL_TO_PASS", "PASS_TO_PASS") else r[k]) for k in keep}
            row["image"] = f"{IMAGE.format(iid=iid)}@{images[iid]['digest']}"
            f.write(json.dumps(row, sort_keys=True) + "\n")
    print(f"{len(chosen)} core + {len(spares)} spares; {len(disagree)} ambiguous rows skipped: {sorted(disagree)}")


if __name__ == "__main__":
    main()
