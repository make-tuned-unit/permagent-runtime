#!/usr/bin/env python3
"""N6 kitchen cook-through gates.

Unit tests cannot hear a kitchen. After rebuilding the iOS app AND
/Applications/Permagent.app (source edits do nothing until then):

  1. Music on, 30 s of not talking — Listening must not ride to 60 s.
  2. One short ask over the music — transcript is Jesse, not lyrics.
  3. Enrollment admit/reject lines in the sidecar log.

Then:

  python3 scripts/check_kitchen_voice_n6.py
  python3 scripts/check_kitchen_voice_n6.py --since 2026-08-27T18:00:00Z

Fails if this window still has a 60 s empty STT cap, or if complete-turn
median first-audio exceeds this morning's 4083 ms ceiling.

The 2026-08-27 morning fixture MUST fail this script (that is N0). Do not
mark N6 done on that log.
"""
from __future__ import annotations

import argparse
import importlib.util
import os
import re
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
REPORT = os.path.join(ROOT, "voice-latency-report.py")
DEFAULT_LOG = os.path.expanduser("~/.permagent/logs/daemon-sidecar.log")
FIRST_AUDIO_CEILING_MS = 4083
TS_RE = re.compile(r"^\[err\]\s+(\S+)\s+")


def load_report():
    spec = importlib.util.spec_from_file_location("voice_latency_report", REPORT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def line_ts(line: str) -> str | None:
    m = TS_RE.match(line)
    return m.group(1) if m else None


def count_speaker_print(path: str, since: str | None) -> dict[str, int]:
    counts = {"admit": 0, "reject": 0, "enroll": 0, "enrolled": 0}
    try:
        f = open(path, encoding="utf-8", errors="replace")
    except FileNotFoundError:
        return counts
    with f:
        for line in f:
            ts = line_ts(line)
            if since and ts and ts < since:
                continue
            if "speaker_print admit" in line:
                counts["admit"] += 1
            elif "speaker_print reject" in line:
                counts["reject"] += 1
            elif "speaker_print enrolled" in line:
                counts["enrolled"] += 1
            elif "speaker_print enroll" in line:
                counts["enroll"] += 1
    return counts


def main() -> int:
    ap = argparse.ArgumentParser(description="N6 kitchen-voice cook-through gates")
    ap.add_argument("logfile", nargs="?", default=DEFAULT_LOG)
    ap.add_argument(
        "--since",
        help="Only turns whose start timestamp is >= this (ISO-8601 prefix ok)",
    )
    ap.add_argument(
        "--allow-no-print",
        action="store_true",
        help="Do not fail when the window has no speaker_print admit/reject lines",
    )
    args = ap.parse_args()

    report = load_report()
    turns = report.parse_turns(args.logfile)
    if args.since:
        turns = [t for t in turns if t.start and t.start >= args.since]
    if not turns:
        print(
            f"FAIL: no voice turns in {args.logfile}"
            + (f" since {args.since}" if args.since else ""),
            file=sys.stderr,
        )
        print(
            "Rebuild Permagent.app + the iOS app, cook with music, then re-run.",
            file=sys.stderr,
        )
        return 1

    summary = report.summarize(turns)
    failed = False

    caps = summary.get("n_empty_stt_cap") or 0
    if caps:
        print(
            f"FAIL: n_empty_stt_cap={caps} (kitchen music still holding Listening)",
            file=sys.stderr,
        )
        failed = True

    median = (summary.get("first_audio_ms") or {}).get("median")
    if median is None:
        print(
            "FAIL: no complete turns with first-audio — cook-through needs one short ask",
            file=sys.stderr,
        )
        failed = True
    elif median > FIRST_AUDIO_CEILING_MS:
        print(
            f"FAIL: first-audio median {median:.0f} ms > ceiling {FIRST_AUDIO_CEILING_MS} ms",
            file=sys.stderr,
        )
        failed = True

    stt_p90 = (summary.get("stt_ms") or {}).get("p90")
    if stt_p90 is not None and stt_p90 > 300:
        print(f"FAIL: STT p90 {stt_p90:.0f} ms > 300 ms", file=sys.stderr)
        failed = True

    prints = count_speaker_print(args.logfile, args.since)
    if not args.allow_no_print and prints["admit"] + prints["reject"] == 0:
        print(
            "FAIL: no speaker_print admit/reject in this window — enroll, then talk",
            file=sys.stderr,
        )
        failed = True

    print(f"turns={summary['n_turns']} complete={summary['n_complete']} empty_stt_cap={caps}")
    print(f"first_audio_median_ms={median} ceiling_ms={FIRST_AUDIO_CEILING_MS}")
    print(f"stt_p90_ms={stt_p90}")
    print(
        "speaker_print "
        f"admit={prints['admit']} reject={prints['reject']} "
        f"enroll={prints['enroll']} enrolled={prints['enrolled']}"
    )

    if failed:
        print(
            "N6 not closed. Reopen N1 (music listen), N2 (lyrics), or N3 (reject-you).",
            file=sys.stderr,
        )
        return 1
    print("N6 log gates green. Confirm the transcript was Jesse, not lyrics.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
