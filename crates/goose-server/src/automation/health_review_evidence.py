#!/usr/bin/env python3
"""Deterministic evidence collection and report validation for Health Review.

The scheduled agent is good at interpreting evidence but must not improvise the
log window, sample one arbitrary rotation, or turn an old report into current
evidence. This companion keeps those mechanical invariants outside the prompt.
It uses only the Python standard library so the bundled starter works on a
stock macOS installation.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import statistics
import sys
from collections import Counter
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCHEMA_VERSION = 1
# Groups beyond this are not reported, so their representative events must not
# be emitted either: an evidence event nothing can cite is only noise.
MAX_WARNING_GROUPS = 100
LINE_RE = re.compile(
    r"^(?:\[err\]\s+)?(?P<ts>\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d+)?Z)"
    r"\s+(?P<level>TRACE|DEBUG|INFO|WARN|ERROR)\s+(?P<body>.*)$"
)
SOURCE_RE = re.compile(r"(?P<source>crates/[A-Za-z0-9_./-]+\.rs)(?::(?P<line>\d+))?:\s*")
TARGET_RE = re.compile(r"(?P<target>[A-Za-z0-9_:.-]+):\s+")
# The auth scheme must be part of the kept prefix, not the redacted value:
# `authorization: Bearer <token>` otherwise redacts the word "Bearer" and
# leaves the token itself in the evidence.
SECRET_RE = re.compile(
    r"(?i)((?:api[_-]?key|access[_-]?token|authorization)\s*[=:]\s*"
    r"(?:bearer\s+|basic\s+|token\s+)?|bearer\s+)"
    r"[^\s,}\]]+"
)
UUID_RE = re.compile(r"\b[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\b", re.I)
NUMBER_RE = re.compile(r"\b\d+(?:\.\d+)?\b")

RELEVANT_RE = re.compile(
    r"(?i)retrying\s*\(|runaway-loop|malformed|unknown decision kind|"
    r"TIMING (?:STT|first audio|Total)|STREAM sentence 1:|spoken budget|"
    r"Client sent Close|PRONUNCIATION unresolved|speaker_print|"
    r"database or disk is full|salvaged index fields|memory pressure|"
    r"available memory|compressor"
)
FRICTION_RE = re.compile(
    r"(?i)\b(?:no|stop|wrong|didn't|did not|you forgot|you missed|"
    r"too much|not what|you didn't|actually)\b"
)
STT_RE = re.compile(r'TIMING STT: (?P<ms>\d+)ms \| transcript: "(?P<text>.*)"')
FIRST_AUDIO_RE = re.compile(r"TIMING first audio: (?P<ms>\d+)ms after speech-end")
TOTAL_RE = re.compile(
    r"TIMING Total: (?P<total>\d+)ms \(STT=(?P<stt>\d+)ms "
    r"Reply\+TTS=(?P<reply>\d+)ms, TTS_total=(?P<tts>\d+)ms, "
    r"(?P<sentences>\d+) sentences\)"
)


def iso(dt: datetime) -> str:
    return dt.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def parse_iso(raw: str) -> datetime:
    return datetime.fromisoformat(raw.replace("Z", "+00:00")).astimezone(timezone.utc)


def expand_path(raw: str) -> Path:
    return Path(os.path.expandvars(os.path.expanduser(raw))).resolve()


def redact(text: str) -> str:
    return SECRET_RE.sub(lambda m: f"{m.group(1)}<redacted>", text)


def parse_line(raw: str, path: Path) -> dict | None:
    match = LINE_RE.match(raw.rstrip("\n"))
    if not match or "tool_call_json=" in raw:
        return None
    body = match.group("body")
    source_match = SOURCE_RE.search(body)
    target_match = TARGET_RE.search(body)
    source = source_match.group("source") if source_match else None
    source_line = int(source_match.group("line")) if source_match and source_match.group("line") else None
    if source_match:
        message = body[source_match.end() :]
    elif target_match:
        message = body[target_match.end() :]
    else:
        message = body
    target = target_match.group("target") if target_match else "unknown"
    event = {
        "timestamp": iso(parse_iso(match.group("ts"))),
        "level": match.group("level"),
        "target": target,
        "source": source,
        "source_line": source_line,
        "message": redact(message.strip()),
        "log_file": str(path),
    }
    stable = "\0".join((event["timestamp"], target, event["message"]))
    event["id"] = "evt_" + hashlib.sha256(stable.encode()).hexdigest()[:12]
    return event


def signature(event: dict) -> str:
    text = UUID_RE.sub("<uuid>", event["message"])
    text = NUMBER_RE.sub("<n>", text)
    return f'{event["level"]}|{event["target"]}|{text[:300]}'


def log_files(log_dir: Path) -> tuple[list[Path], str]:
    server = sorted(p for p in (log_dir / "server").glob("**/*.log") if p.is_file())
    if server:
        # daemon-sidecar mirrors the same process with slightly different
        # formatting and timestamp precision. The structured server rotations
        # are canonical; mixing both doubles counts.
        return server, "server"
    sidecars = [
        p for p in (log_dir / "daemon-sidecar.log", log_dir / "daemon-sidecar.log.old")
        if p.is_file()
    ]
    return sidecars, "daemon-sidecar"


def percentile_summary(values: list[int]) -> dict | None:
    if not values:
        return None
    ordered = sorted(values)
    return {
        "count": len(ordered),
        "median_ms": int(statistics.median(ordered)),
        "worst_ms": ordered[-1],
        "best_ms": ordered[0],
    }


def collect(log_dir: Path, hours: float, end: datetime) -> dict:
    start = end - timedelta(hours=hours)
    paths, canonical = log_files(log_dir)
    events: list[dict] = []
    scanned: list[dict] = []
    for path in paths:
        accepted = 0
        try:
            with path.open("r", encoding="utf-8", errors="replace") as handle:
                for raw in handle:
                    event = parse_line(raw, path)
                    if event is None:
                        continue
                    timestamp = parse_iso(event["timestamp"])
                    if start <= timestamp < end:
                        events.append(event)
                        accepted += 1
        except OSError as exc:
            scanned.append({"path": str(path), "events": 0, "error": str(exc)})
            continue
        scanned.append({"path": str(path), "events": accepted})
    # A copied/rotated file can duplicate a canonical line exactly. IDs include
    # timestamp + content, so this removes those without merging real repeats.
    events = sorted({e["id"]: e for e in events}.values(), key=lambda e: e["timestamp"])

    grouped: dict[str, list[dict]] = {}
    for event in events:
        if event["level"] in ("WARN", "ERROR"):
            grouped.setdefault(signature(event), []).append(event)
    warning_groups = []
    ordered_groups = sorted(grouped.items(), key=lambda item: (-len(item[1]), item[0]))
    for _, members in ordered_groups[:MAX_WARNING_GROUPS]:
        rep = members[0]
        warning_groups.append({
            "level": rep["level"],
            "target": rep["target"],
            "count": len(members),
            "first": members[0]["timestamp"],
            "last": members[-1]["timestamp"],
            "representative_event": rep["id"],
            "message_signature": NUMBER_RE.sub("<n>", rep["message"])[:300],
        })

    by_id = {event["id"]: event for event in events}
    selected: dict[str, dict] = {}
    for group in warning_groups:
        event_id = group["representative_event"]
        selected[event_id] = by_id[event_id]
    for event in events:
        if RELEVANT_RE.search(event["message"]):
            selected[event["id"]] = event
        stt = STT_RE.search(event["message"])
        if stt and FRICTION_RE.search(stt.group("text")):
            selected[event["id"]] = event

    turns: list[dict] = []
    current: dict | None = None
    for event in events:
        if match := STT_RE.search(event["message"]):
            if current:
                turns.append(current)
            current = {
                "stt_event": event["id"],
                "timestamp": event["timestamp"],
                "stt_ms": int(match.group("ms")),
                # Keep private transcript text out of the aggregate. A user
                # correction is separately selected as minimal friction proof.
                "transcript_chars": len(match.group("text")),
            }
            selected[event["id"]] = event
        elif current and (match := FIRST_AUDIO_RE.search(event["message"])):
            current["first_audio_ms"] = int(match.group("ms"))
            current["first_audio_event"] = event["id"]
            selected[event["id"]] = event
        elif current and (match := TOTAL_RE.search(event["message"])):
            current.update({
                "total_ms": int(match.group("total")),
                "tts_ms": int(match.group("tts")),
                "sentences": int(match.group("sentences")),
                "total_event": event["id"],
            })
            selected[event["id"]] = event
            turns.append(current)
            current = None
    if current:
        turns.append(current)

    first_audio = [t["first_audio_ms"] for t in turns if "first_audio_ms" in t]
    totals = [t["total_ms"] for t in turns if "total_ms" in t]
    stt_values = [t["stt_ms"] for t in turns]
    tts_values = [t["tts_ms"] for t in turns if "tts_ms" in t]

    levels = Counter(e["level"] for e in events)
    egress = {
        "denied_and_unaudited": sum(
            "allowed=false" in e["message"] and "audited=false" in e["message"]
            for e in events
        ),
        "allowed_and_unaudited": sum(
            "allowed=true" in e["message"] and "audited=false" in e["message"]
            for e in events
        ),
    }
    return {
        "schema_version": SCHEMA_VERSION,
        "window": {
            "start": iso(start),
            "end": iso(end),
            "hours": hours,
            "end_exclusive": True,
        },
        "coverage": {
            "canonical_source": canonical,
            "files_considered": len(paths),
            "files_scanned": scanned,
            "events_in_window": len(events),
            "earliest_event": events[0]["timestamp"] if events else None,
            "latest_event": events[-1]["timestamp"] if events else None,
            "window_filter_enforced": True,
            "sidecar_mirror_excluded": canonical == "server",
        },
        "counts": dict(sorted(levels.items())),
        "warning_error_groups": warning_groups,
        "voice": {
            "turns": len(turns),
            "completed_turns": len(totals),
            "first_audio": percentile_summary(first_audio),
            "total": percentile_summary(totals),
            "stt": percentile_summary(stt_values),
            "tts": percentile_summary(tts_values),
            "turn_details": turns,
        },
        "egress": egress,
        "events": sorted(selected.values(), key=lambda e: e["timestamp"]),
        "rules": {
            "findings_must_reference_event_ids": True,
            "previous_reports_are_context_not_current_evidence": True,
            "allowed_false_means_the_egress_was_suppressed": True,
        },
    }


def validate(evidence: dict, report: str) -> list[str]:
    errors: list[str] = []
    window = evidence["window"]
    exact_window = f'**Window:** {window["start"]} to {window["end"]} (end exclusive)'
    if exact_window not in report:
        errors.append(f"missing exact window line: {exact_window}")
    if "## Findings" not in report or "## Latency" not in report or "## Clean" not in report:
        errors.append("report must contain Findings, Latency, and Clean sections")

    valid_ids = {event["id"] for event in evidence["events"]}
    cited_ids = set(re.findall(r"\[event:(evt_[0-9a-f]{12})\]", report))
    unknown = cited_ids - valid_ids
    if unknown:
        errors.append("unknown or out-of-window event IDs: " + ", ".join(sorted(unknown)))

    findings = report.split("## Findings", 1)[-1].split("\n## ", 1)[0]
    headings = list(re.finditer(r"(?m)^###\s+.+$", findings))
    if headings:
        for index, heading in enumerate(headings):
            end = headings[index + 1].start() if index + 1 < len(headings) else len(findings)
            block = findings[heading.start() : end]
            if not re.search(r"\[event:evt_[0-9a-f]{12}\]", block):
                errors.append(f"finding has no evidence ID: {heading.group(0)}")
    elif "No confirmed findings" not in findings:
        errors.append("Findings must use ### headings with an evidence ID, or say 'No confirmed findings.'")

    voice = evidence["voice"]
    if voice["turns"]:
        required = [f'- Voice turns: {voice["turns"]}']
        if voice["first_audio"]:
            required.extend([
                f'- First-audio median: {voice["first_audio"]["median_ms"]}ms',
                f'- First-audio worst: {voice["first_audio"]["worst_ms"]}ms',
            ])
        for line in required:
            if line not in report:
                errors.append(f"latency section missing evidence-derived line: {line}")

    lowered = report.lower()
    for forbidden in ("~/.permagent/audit.db", "egress_calls"):
        if forbidden in lowered:
            errors.append(f"known invalid implementation detail in report: {forbidden}")
    if evidence["egress"]["allowed_and_unaudited"] == 0:
        bad_claims = ("cloud inference calls were not logged", "unlogged cloud calls succeeded")
        if any(claim in lowered for claim in bad_claims):
            errors.append("report claims successful unaudited egress, but evidence contains none")
    if "salvaged index fields instead of storing raw" in lowered and "stored as raw unstructured" in lowered:
        errors.append("report contradicts the Librarian salvage evidence")
    return errors


def write_json(path: Path, payload: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_suffix(path.suffix + ".tmp")
    tmp.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    tmp.replace(path)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    collect_parser = sub.add_parser("collect")
    collect_parser.add_argument("--log-dir", required=True)
    collect_parser.add_argument("--hours", type=float, required=True)
    collect_parser.add_argument("--output", required=True)
    collect_parser.add_argument("--end", help="fixed UTC ISO timestamp (tests/replay)")
    validate_parser = sub.add_parser("validate")
    validate_parser.add_argument("--evidence", required=True)
    validate_parser.add_argument("--report", required=True)
    args = parser.parse_args(argv)

    if args.command == "collect":
        if not (0 < args.hours <= 24 * 31):
            parser.error("--hours must be in (0, 744]")
        end = parse_iso(args.end) if args.end else datetime.now(timezone.utc)
        payload = collect(expand_path(args.log_dir), args.hours, end)
        output = expand_path(args.output)
        write_json(output, payload)
        print(output)
        return 0

    evidence_path = expand_path(args.evidence)
    report_path = expand_path(args.report)
    evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
    errors = validate(evidence, report_path.read_text(encoding="utf-8"))
    if errors:
        for error in errors:
            print(f"ERROR: {error}", file=sys.stderr)
        return 1
    print("health report validation: ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
