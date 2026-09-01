#!/usr/bin/env python3
"""voice-latency-report.py — turn-by-turn latency report for the voice pipeline.

Parses the `permagentd::voice` tracing lines emitted by
crates/goose-server/src/routes/voice.rs and
crates/goose-server/src/voice/ort_kokoro_backend.rs, and groups them into
CAPTURES. A capture starts at "Recording started". Enrollment takes, rejected
speakers, empty STT, abandoned starts, audible-but-interrupted replies, and
fully completed agent turns are classified separately rather than being folded
into one misleading "incomplete" bucket.

Metrics per turn (all measured server-side, from tracing timestamps):
  audio_s                — length of the captured utterance (client-reported).
  stt_ms                 — time to transcribe the utterance (Moonshine).
  speech_end_to_stream_ms — "recording stopped" until the reply stream is
                             ready: speaker gate + STT + correction + context,
                             recall, provider, and reply setup. It is not just
                             context/reply setup and it is not first-token time.
  ttft_ms                — time-to-first-token: LLM stream start to the first
                            assistant text chunk.
  first_sentence_tts_ms  — synthesis time for the FIRST spoken sentence (what
                            gates when audio can start).
  first_audio_ms         — wall-clock from "recording stopped" (speech end)
                            to the first audio byte sent to the client — the
                            number that matters for perceived latency.
  total_ms                — STT + reply-generation + TTS for the whole turn.
  n_sentences              — number of spoken sentences synthesized.

Latency percentiles use audible agent turns only. Enrollment and rejected
speakers are never allowed to distort them.

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
RE_STT = re.compile(r'TIMING STT: (\d+)ms\s+\|\s+transcript: "(.*)"')
RE_PIPELINE = re.compile(r"(?:total pre-stream|speech-end-to-stream)=(\d+)ms")
RE_TTFT = re.compile(r"TTFT: (\d+)ms after stream start")
RE_SENTENCE = re.compile(r"STREAM sentence (\d+): \d+chars TTS=(\d+)ms")
RE_FIRST_AUDIO = re.compile(r"TIMING first audio: (\d+)ms after speech-end")
RE_TOTAL = re.compile(
    r"TIMING Total: (\d+)ms \(STT=\d+ms Reply\+TTS=\d+ms, TTS_total=\d+ms, (\d+) sentences\)"
)
RE_SPEAKER = re.compile(
    r"speaker_print (early_)?(admit|reject|open|unavailable)(?: score=([-\d.]+))? gate_ms=(\d+)"
)
RE_ENROLL_START = re.compile(r"speaker_print enroll start")
RE_ENROLL_END = re.compile(r"speaker_print enroll(?:ed| skip| cleared)")

METRICS = [
    "stt_ms",
    "speech_end_to_stream_ms",
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
    kind: str = "agent"
    speech_end_to_stream_ms: Optional[int] = None
    ttft_ms: Optional[int] = None
    first_sentence_tts_ms: Optional[int] = None
    first_audio_ms: Optional[int] = None
    total_ms: Optional[int] = None
    n_sentences: int = 0
    speaker_outcome: Optional[str] = None
    speaker_score: Optional[float] = None
    speaker_gate_ms: Optional[int] = None
    stt_empty: bool = False
    _max_sentence_seen: int = field(default=0, repr=False)

    @property
    def complete(self) -> bool:
        return self.total_ms is not None

    @property
    def audible(self) -> bool:
        return self.first_audio_ms is not None

    @property
    def status(self) -> str:
        if self.kind == "enrollment":
            return "enrollment"
        if self.speaker_outcome in {"reject", "early_reject", "unavailable", "early_unavailable"}:
            return "rejected"
        if self.stt_empty:
            return "empty_stt"
        if self.first_audio_ms is not None and self.total_ms is None:
            return "interrupted"
        if self.first_audio_ms is not None:
            return "complete"
        if self.audio_s is None:
            return "abandoned"
        return "incomplete"


def parse_turns(path: str) -> list[Turn]:
    turns: list[Turn] = []
    cur: Optional[Turn] = None
    enrolling = False

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

            if RE_ENROLL_START.search(msg):
                enrolling = True
                continue
            if RE_ENROLL_END.search(msg):
                enrolling = False
                continue

            if (rs := RE_REC_START.search(msg)):
                close(cur)
                cur = Turn(
                    index=len(turns) + 1,
                    start=ts,
                    kind="enrollment" if enrolling else "agent",
                )
                turns.append(cur)
                continue
            if cur is None:
                continue  # stray line before any "Recording started"

            if (rs := RE_REC_STOP.search(msg)):
                cur.audio_s = float(rs.group(1))
            elif (rs := RE_STT.search(msg)):
                cur.stt_ms = int(rs.group(1))
                cur.stt_empty = rs.group(2) == ""
            elif (rs := RE_PIPELINE.search(msg)):
                cur.speech_end_to_stream_ms = int(rs.group(1))
            elif (rs := RE_SPEAKER.search(msg)):
                prefix = rs.group(1) or ""
                cur.speaker_outcome = prefix + rs.group(2)
                cur.speaker_score = float(rs.group(3)) if rs.group(3) is not None else None
                cur.speaker_gate_ms = int(rs.group(4))
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
    agent = [t for t in turns if t.kind == "agent"]
    audible = [t for t in agent if t.audible]
    complete = [t for t in agent if t.complete]
    incomplete = len(agent) - len(complete)
    summary = {
        "n_captures": len(turns),
        "n_turns": len(agent),
        "n_enrollment_captures": sum(t.kind == "enrollment" for t in turns),
        "n_audible": len(audible),
        "n_complete": len(complete),
        "n_incomplete": incomplete,
        "n_rejected": sum(t.status == "rejected" for t in agent),
        "n_empty_stt": sum(t.status == "empty_stt" for t in agent),
        "n_abandoned": sum(t.status == "abandoned" for t in agent),
        "n_interrupted": sum(t.status == "interrupted" for t in agent),
        "n_long_captures": sum(
            t.audio_s is not None and t.audio_s >= 10.0 for t in agent
        ),
        "n_30s_captures": sum(
            t.audio_s is not None and t.audio_s >= 30.0 for t in agent
        ),
        # 60 s (or cap) recordings that STT'd empty — kitchen music holding
        # Listening until maxTurnMs. Distinct from a barged-in incomplete.
        "n_empty_stt_cap": sum(
            1
            for t in agent
            if t.status == "empty_stt"
            and t.audio_s is not None
            and t.audio_s >= 59.5
            and t.stt_ms is not None
        ),
    }
    for metric in METRICS:
        vals = [getattr(t, metric) for t in audible if getattr(t, metric) is not None]
        summary[metric] = {
            "n": len(vals),
            "min": min(vals) if vals else None,
            "median": statistics.median(vals) if vals else None,
            "p90": percentile(vals, 0.9),
            "max": max(vals) if vals else None,
        }
    return summary


TURN_COLS = ["#", "start", "kind", "audio_s", "speaker", "gate_ms", "stt_ms",
             "speech_end_to_stream_ms", "ttft_ms", "1st_sentence_tts_ms",
             "1st_audio_ms", "total_ms", "sentences", "status"]
STAT_COLS = ["metric", "n", "min", "median", "p90", "max"]


def fmt(v, nd=0):
    if v is None:
        return "-"
    return f"{v:.{nd}f}" if isinstance(v, float) else str(v)


def turn_row(t: Turn) -> list[str]:
    speaker = t.speaker_outcome or "-"
    if t.speaker_score is not None:
        speaker += f"/{t.speaker_score:.3f}"
    return [str(t.index), t.start or "-", t.kind, fmt(t.audio_s, 1), speaker,
            fmt(t.speaker_gate_ms), fmt(t.stt_ms), fmt(t.speech_end_to_stream_ms),
            fmt(t.ttft_ms), fmt(t.first_sentence_tts_ms),
            fmt(t.first_audio_ms), fmt(t.total_ms), fmt(t.n_sentences),
            t.status]


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
    print(
        f"agent turns: {summary['n_turns']}   audible: {summary['n_audible']}   "
        f"fully complete: {summary['n_complete']}   "
        f"enrollment captures: {summary['n_enrollment_captures']}   "
        f"rejected: {summary['n_rejected']}   interrupted: {summary['n_interrupted']}"
    )
    print(
        f"empty STT: {summary['n_empty_stt']}   abandoned: {summary['n_abandoned']}   "
        f">=10s captures: {summary['n_long_captures']}   >=30s captures: {summary['n_30s_captures']}"
    )
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
            "turns": [
                {
                    **{k: v for k, v in asdict(t).items() if not k.startswith("_")},
                    "status": t.status,
                }
                for t in turns
            ],
            "summary": summary,
        }, indent=2))
    elif not turns:
        print(f"No voice turns found in {args.logfile}")
    else:
        print_report(turns, summary, args.markdown)


if __name__ == "__main__":
    main()
