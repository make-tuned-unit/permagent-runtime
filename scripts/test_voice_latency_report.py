#!/usr/bin/env python3
"""N0 tripwire: this morning's 60.2 s empty-STT turn must be counted as a cap.

2026-08-27 07:44 ADT (10:44Z) — kitchen music held Listening until maxTurnMs.
The report already marks the turn incomplete (no first-audio). N1 must also
expose `n_empty_stt_cap` so six of these cannot hide as generic incompletes.

Baseline to keep (12 complete turns after 08:10): median speech-end → first
audio 4083 ms. Do not raise that ceiling in later nodes.
"""
from __future__ import annotations

import importlib.util
import os
import sys

ROOT = os.path.dirname(os.path.abspath(__file__))
FIXTURE = os.path.join(ROOT, "testdata", "voice_20260827_empty_60s.txt")
BACKGROUND_FIXTURE = os.path.join(
    ROOT, "testdata", "voice_20260827_enrollment_and_background.txt"
)
REPORT = os.path.join(ROOT, "voice-latency-report.py")

# Recorded 2026-08-27 morning complete-turn median. Later nodes must stay ≤ this.
FIRST_AUDIO_CEILING_MS = 4083


def load_report():
    spec = importlib.util.spec_from_file_location("voice_latency_report", REPORT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main() -> int:
    report = load_report()
    turns = report.parse_turns(FIXTURE)
    if len(turns) != 1:
        print(f"FAIL: expected 1 turn in fixture, got {len(turns)}", file=sys.stderr)
        return 1
    t = turns[0]
    if t.complete:
        print("FAIL: 60.2s empty STT was classified complete", file=sys.stderr)
        return 1
    if t.audio_s != 60.2:
        print(f"FAIL: expected audio_s=60.2, got {t.audio_s}", file=sys.stderr)
        return 1
    if t.stt_ms != 587:
        print(f"FAIL: expected stt_ms=587, got {t.stt_ms}", file=sys.stderr)
        return 1

    summary = report.summarize(turns)
    got = summary.get("n_empty_stt_cap")
    if got != 1:
        print(
            f"FAIL: n_empty_stt_cap must be 1 for a 60s empty-STT cap, got {got!r}",
            file=sys.stderr,
        )
        print(
            f"(first-audio ceiling to keep: {FIRST_AUDIO_CEILING_MS} ms)",
            file=sys.stderr,
        )
        return 1

    # Tonight's enrollment recordings must not inflate turn counts or latency
    # percentiles. A false-accepted 33.1s background turn must be visible as a
    # long capture, and the new early learned-speaker reject must be classified
    # as rejected even though no Stop frame follows it.
    captures = report.parse_turns(BACKGROUND_FIXTURE)
    summary = report.summarize(captures)
    expected = {
        "n_captures": 5,
        "n_turns": 2,
        "n_enrollment_captures": 3,
        "n_audible": 1,
        "n_complete": 1,
        "n_rejected": 1,
        "n_long_captures": 1,
        "n_30s_captures": 1,
    }
    for key, want in expected.items():
        if summary.get(key) != want:
            print(
                f"FAIL: background fixture {key}={summary.get(key)!r}, expected {want}",
                file=sys.stderr,
            )
            return 1
    statuses = [t.status for t in captures]
    if statuses != ["enrollment", "enrollment", "enrollment", "complete", "rejected"]:
        print(f"FAIL: capture classifications were {statuses}", file=sys.stderr)
        return 1
    if summary["speech_end_to_stream_ms"]["median"] != 621:
        print("FAIL: speech_end_to_stream metric lost its real semantics", file=sys.stderr)
        return 1

    n6 = os.path.join(ROOT, "check_kitchen_voice_n6.py")
    import subprocess

    r = subprocess.run([sys.executable, n6, FIXTURE], capture_output=True, text=True)
    if r.returncode == 0:
        print("FAIL: N6 checker must reject this morning's 60s empty-STT fixture", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
