#!/usr/bin/env python3
"""voice-latency-report.py — turn-by-turn latency report for the voice pipeline.

Parses the `permagentd::voice` tracing lines emitted by
crates/goose-server/src/routes/voice.rs and
crates/goose-server/src/voice/ort_kokoro_backend.rs, and groups them into
TURNS. A turn starts at "Recording started" and ends at "TIMING Total" (or
the next "Recording started", for a turn that never finished — e.g. it was
barged-in on or the client disconnected mid-reply).

Metrics per turn (all measured server-side, from tracing timestamps):
  audio_s                — length of the captured utterance (client-reported).
  stt_ms                 — time to transcribe the utterance (Whisper/STT).
  pre_stream_ms          — "recording stopped" to the first LLM token being
                            available to stream: STT + context/recall + reply
                            setup.
  ttft_ms                — time-to-first-token: LLM stream start to the first
                            assistant text chunk.
  first_sentence_tts_ms  — synthesis time for the FIRST spoken sentence (what
                            gates when audio can start).
  first_audio_ms         — wall-clock from "recording stopped" (speech end)
                            to the first audio byte sent to the client — the
                            number that matters for perceived latency.
  total_ms                — STT + reply-generation + TTS for the whole turn.
  n_sentences              — number of spoken sentences synthesized.

A turn that never reaches "TIMING first audio" (a quick tap, an empty
transcript, a disconnect before any reply) is listed but excluded from the
percentile statistics, and counted separately as an "incomplete turn".

Usage: python3 scripts/voice-latency-report.py [logfile] [--markdown] [--json]
Default logfile: ~/.permagent/logs/daemon-sidecar.log
"""
import argparse
import json
import math
import os
import re
import statistics
import sys
from dataclasses import asdict, dataclass, field
from typing import Optional

DEFAULT_LOG = os.path.expanduser("~/.permagent/logs/daemon-sidecar.log")

LINE_RE = re.compile(r"^\[err\]\s+(\S+)\s+INFO permagentd::voice:\s?(.*)$")
RE_REC_START = re.compile(r"Recording started, sample_rate=(\d+)")
RE_REC_STOP = re.compile(r"Recording stopped, \d+ samples \(([\d.]+)s audio\)")
RE_STT = re.compile(r"TIMING STT: (\d+)ms")
RE_PIPELINE = re.compile(r"total pre-stream=(\d+)ms")
RE_TTFT = re.compile(r"TTFT: (\d+)ms after stream start")
RE_SENTENCE = re.compile(r"STREAM sentence (\d+): \d+chars TTS=(\d+)ms")
RE_FIRST_AUDIO = re.compile(r"TIMING first audio: (\d+)ms after speech-end")
RE_TOTAL = re.compile(
    r"TIMING Total: (\d+)ms \(STT=\d+ms Reply\+TTS=\d+ms, TTS_total=\d+ms, (\d+) sentences\)"
)

METRICS = [
    "stt_ms",
    "pre_stream_ms",
    "ttft_ms",
    "first_sentence_tts_ms",
    "first_audio_ms",
    "total_ms",
]


@dataclass
class Turn:
    index: int
    start: Optional[str] = None
    audio_s: Optional[float] = None
    stt_ms: Optional[int] = None
    pre_stream_ms: Optional[int] = None
    ttft_ms: Optional[int] = None
    first_sentence_tts_ms: Optional[int] = None
    first_audio_ms: Optional[int] = None
    total_ms: Optional[int] = None
    n_sentences: int = 0
    _max_sentence_seen: int = field(default=0, repr=False)

    @property
    def complete(self) -> bool:
        return self.first_audio_ms is not None


def parse_turns(path: str) -> list[Turn]:
    turns: list[Turn] = []
    cur: Optional[Turn] = None

    def close(t: Optional[Turn]):
        if t is not None and t.n_sentences == 0:
            t.n_sentences = t._max_sentence_seen

    try:
        f = open(path, encoding="utf-8", errors="replace")
    except FileNotFoundError:
        print(f"No log file at {path}", file=sys.stderr)
        return []

    with f:
        for line in f:
            m = LINE_RE.match(line.rstrip("\n"))
            if not m:
                continue
            ts, msg = m.group(1), m.group(2)

            if (rs := RE_REC_START.search(msg)):
                close(cur)
                cur = Turn(index=len(turns) + 1, start=ts)
                turns.append(cur)
                continue
            if cur is None:
                continue  # stray line before any "Recording started"

            if (rs := RE_REC_STOP.search(msg)):
                cur.audio_s = float(rs.group(1))
            elif (rs := RE_STT.search(msg)):
                cur.stt_ms = int(rs.group(1))
            elif (rs := RE_PIPELINE.search(msg)):
                cur.pre_stream_ms = int(rs.group(1))
            elif (rs := RE_TTFT.search(msg)):
                cur.ttft_ms = int(rs.group(1))
            elif (rs := RE_SENTENCE.search(msg)):
                n, tts_ms = int(rs.group(1)), int(rs.group(2))
                cur._max_sentence_seen = max(cur._max_sentence_seen, n)
                if cur.first_sentence_tts_ms is None:
                    cur.first_sentence_tts_ms = tts_ms
            elif (rs := RE_FIRST_AUDIO.search(msg)):
                cur.first_audio_ms = int(rs.group(1))
            elif (rs := RE_TOTAL.search(msg)):
                cur.total_ms = int(rs.group(1))
                cur.n_sentences = int(rs.group(2))
                close(cur)
                cur = None

    close(cur)
    return turns


def percentile(values: list[float], p: float) -> Optional[float]:
    if not values:
        return None
    s = sorted(values)
    if len(s) == 1:
        return s[0]
    k = (len(s) - 1) * p
    lo, hi = math.floor(k), math.ceil(k)
    if lo == hi:
        return s[int(k)]
    return s[lo] * (hi - k) + s[hi] * (k - lo)


def summarize(turns: list[Turn]) -> dict:
    complete = [t for t in turns if t.complete]
    incomplete = len(turns) - len(complete)
    summary = {"n_turns": len(turns), "n_complete": len(complete), "n_incomplete": incomplete}
    for metric in METRICS:
        vals = [getattr(t, metric) for t in complete if getattr(t, metric) is not None]
        summary[metric] = {
            "n": len(vals),
            "min": min(vals) if vals else None,
            "median": statistics.median(vals) if vals else None,
            "p90": percentile(vals, 0.9),
            "max": max(vals) if vals else None,
        }
    return summary


TURN_COLS = ["#", "start", "audio_s", "stt_ms", "pre_stream_ms", "ttft_ms",
             "1st_sentence_tts_ms", "1st_audio_ms", "total_ms", "sentences", "status"]
STAT_COLS = ["metric", "n", "min", "median", "p90", "max"]


def fmt(v, nd=0):
    if v is None:
        return "-"
    return f"{v:.{nd}f}" if isinstance(v, float) else str(v)


def turn_row(t: Turn) -> list[str]:
    return [str(t.index), t.start or "-", fmt(t.audio_s, 1), fmt(t.stt_ms),
            fmt(t.pre_stream_ms), fmt(t.ttft_ms), fmt(t.first_sentence_tts_ms),
            fmt(t.first_audio_ms), fmt(t.total_ms), fmt(t.n_sentences),
            "ok" if t.complete else "incomplete"]


def stat_row(metric: str, s: dict) -> list[str]:
    return [metric, str(s["n"]), fmt(s["min"], 1), fmt(s["median"], 1), fmt(s["p90"], 1), fmt(s["max"], 1)]


def print_table(header: list[str], rows: list[list[str]], markdown: bool):
    if markdown:
        print("| " + " | ".join(header) + " |")
        print("|" + "|".join("---" for _ in header) + "|")
        for r in rows:
            print("| " + " | ".join(r) + " |")
    else:
        widths = [max(len(h), *(len(r[i]) for r in rows)) if rows else len(h) for i, h in enumerate(header)]
        print("  ".join(h.ljust(w) for h, w in zip(header, widths)))
        for r in rows:
            print("  ".join(c.ljust(w) for c, w in zip(r, widths)))


def print_report(turns: list[Turn], summary: dict, markdown: bool):
    print_table(TURN_COLS, [turn_row(t) for t in turns], markdown)
    print()
    print(f"complete turns: {summary['n_complete']}   incomplete turns: {summary['n_incomplete']}")
    print()
    print_table(STAT_COLS, [stat_row(m, summary[m]) for m in METRICS], markdown)


def main():
    ap = argparse.ArgumentParser(description="Voice pipeline latency report")
    ap.add_argument("logfile", nargs="?", default=DEFAULT_LOG)
    ap.add_argument("--markdown", action="store_true")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    turns = parse_turns(args.logfile)
    summary = summarize(turns)

    if args.json:
        print(json.dumps({
            "turns": [{k: v for k, v in asdict(t).items() if not k.startswith("_")} for t in turns],
            "summary": summary,
        }, indent=2))
    elif not turns:
        print(f"No voice turns found in {args.logfile}")
    else:
        print_report(turns, summary, args.markdown)


if __name__ == "__main__":
    main()
