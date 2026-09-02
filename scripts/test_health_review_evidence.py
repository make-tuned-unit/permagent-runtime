#!/usr/bin/env python3
import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "crates/goose-server/src/automation/health_review_evidence.py"
SPEC = importlib.util.spec_from_file_location("health_review_evidence", SCRIPT)
health = importlib.util.module_from_spec(SPEC)
assert SPEC.loader
SPEC.loader.exec_module(health)


class HealthEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.server = self.root / "server/2026-08-28"
        self.server.mkdir(parents=True)

    def tearDown(self):
        self.tmp.cleanup()

    def write(self, name, lines):
        path = self.server / name
        path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        return path

    def collect(self):
        return health.collect(
            self.root, 24, datetime(2026, 8, 28, 14, tzinfo=timezone.utc)
        )

    def test_all_rotations_window_filter_and_contamination_exclusion(self):
        self.write("a.log", [
            "2026-08-27T13:59:59Z ERROR old: crates/old.rs: prior only",
            "2026-08-27T14:00:00Z WARN one: crates/a.rs: first in window",
        ])
        self.write("b.log", [
            "2026-08-28T13:00:00Z ERROR two: crates/b.rs: database or disk is full allowed=false audited=false",
            "2026-08-28T13:01:00Z INFO auto: crates/x.rs: tool_call_json=health old warning",
            "2026-08-28T14:00:00Z ERROR future: crates/c.rs: end is exclusive",
        ])
        evidence = self.collect()
        self.assertEqual(evidence["coverage"]["files_considered"], 2)
        self.assertEqual(evidence["coverage"]["events_in_window"], 2)
        self.assertEqual(evidence["egress"]["denied_and_unaudited"], 1)
        self.assertEqual(evidence["egress"]["allowed_and_unaudited"], 0)

    def test_server_is_canonical_and_sidecar_mirror_is_not_double_counted(self):
        line = "2026-08-28T13:00:00Z WARN one: crates/a.rs: same warning"
        self.write("a.log", [line])
        (self.root / "daemon-sidecar.log").write_text("[err]   " + line + "\n")
        evidence = self.collect()
        self.assertEqual(evidence["coverage"]["canonical_source"], "server")
        self.assertTrue(evidence["coverage"]["sidecar_mirror_excluded"])
        self.assertEqual(evidence["counts"]["WARN"], 1)

    def test_voice_distribution_and_user_correction_are_evidence(self):
        self.write("voice.log", [
            '2026-08-28T13:00:00Z INFO permagentd::voice: crates/v.rs: TIMING STT: 40ms | transcript: "You did not check Grocery Savers"',
            "2026-08-28T13:00:03Z INFO permagentd::voice: crates/v.rs: TIMING first audio: 3000ms after speech-end",
            "2026-08-28T13:00:05Z INFO permagentd::voice: crates/v.rs: TIMING Total: 5000ms (STT=40ms Reply+TTS=4960ms, TTS_total=900ms, 2 sentences)",
            '2026-08-28T13:10:00Z INFO permagentd::voice: crates/v.rs: TIMING STT: 60ms | transcript: "Hello"',
            "2026-08-28T13:10:04Z INFO permagentd::voice: crates/v.rs: TIMING first audio: 4000ms after speech-end",
            "2026-08-28T13:10:08Z INFO permagentd::voice: crates/v.rs: TIMING Total: 8000ms (STT=60ms Reply+TTS=7940ms, TTS_total=1200ms, 3 sentences)",
        ])
        evidence = self.collect()
        self.assertEqual(evidence["voice"]["turns"], 2)
        self.assertEqual(evidence["voice"]["first_audio"]["median_ms"], 3500)
        messages = [event["message"] for event in evidence["events"]]
        self.assertTrue(any("Grocery Savers" in message for message in messages))

    def test_validator_rejects_uncited_finding_and_false_egress_claim(self):
        self.write("a.log", [
            "2026-08-28T13:00:00Z ERROR two: crates/b.rs: audit failed allowed=false audited=false"
        ])
        evidence = self.collect()
        report = (
            "# Health review\n"
            f'**Window:** {evidence["window"]["start"]} to {evidence["window"]["end"]} (end exclusive)\n'
            "## Findings\n### Egress\nCloud inference calls were not logged.\n"
            "## Latency\n## Clean\n"
        )
        errors = health.validate(evidence, report)
        self.assertTrue(any("no evidence ID" in error for error in errors))
        self.assertTrue(any("successful unaudited egress" in error for error in errors))

    def test_validator_accepts_exact_evidence_derived_report(self):
        self.write("a.log", [
            "2026-08-28T13:00:00Z ERROR two: crates/b.rs: audit failed allowed=false audited=false"
        ])
        evidence = self.collect()
        event_id = evidence["events"][0]["id"]
        report = (
            "# Health review\n"
            f'**Window:** {evidence["window"]["start"]} to {evidence["window"]["end"]} (end exclusive)\n'
            f"## Findings\n### Audit suppression\nEvidence [event:{event_id}] proves the request was suppressed.\n"
            "## Latency\nNo voice turns.\n## Clean\nNone asserted.\n"
        )
        self.assertEqual(health.validate(evidence, report), [])

    def test_credentials_are_redacted_out_of_evidence(self):
        self.write("a.log", [
            "2026-08-28T13:00:00Z ERROR net: crates/n.rs: retrying (2/5) "
            "authorization: Bearer sk-live-abcdef123456 api_key=topsecret999",
        ])
        evidence = self.collect()
        blob = json.dumps(evidence)
        self.assertNotIn("sk-live-abcdef123456", blob)
        self.assertNotIn("topsecret999", blob)
        self.assertIn("<redacted>", blob)
        # Redaction must not swallow the diagnostic content around it.
        self.assertIn("retrying (2/5)", blob)

    def test_every_relevant_category_is_selected_as_evidence(self):
        # Each of these is a category RELEVANT_RE claims to catch; an INFO line
        # is not otherwise selected, so selection here proves the pattern hit.
        self.write("a.log", [
            "2026-08-28T13:00:00Z INFO a: crates/a.rs: retrying (3/5) after upstream reset",
            "2026-08-28T13:00:01Z INFO b: crates/b.rs: runaway-loop guard tripped for agent",
            "2026-08-28T13:00:02Z INFO c: crates/c.rs: malformed tool response discarded",
            "2026-08-28T13:00:03Z INFO d: crates/d.rs: unknown decision kind 'ponder'",
            "2026-08-28T13:00:04Z INFO e: crates/e.rs: available memory 412MB, compressor active",
            "2026-08-28T13:00:05Z INFO f: crates/f.rs: nothing interesting happened here",
        ])
        evidence = self.collect()
        selected = {event["message"] for event in evidence["events"]}
        for expected in ("retrying (3/5)", "runaway-loop", "malformed",
                         "unknown decision kind", "available memory"):
            self.assertTrue(
                any(expected in message for message in selected),
                f"{expected} was not selected as evidence",
            )
        self.assertFalse(any("nothing interesting" in m for m in selected))

    def test_cli_collect_then_validate_round_trip(self):
        # The recipe shells out to these exact subcommands; nothing else in this
        # file exercises argparse or the --output/--report file I/O.
        self.write("a.log", [
            "2026-08-28T13:00:00Z ERROR two: crates/b.rs: audit failed allowed=false audited=false"
        ])
        evidence_path = self.root / "out/health-evidence-latest.json"
        collect = subprocess.run(
            [sys.executable, str(SCRIPT), "collect",
             "--log-dir", str(self.root), "--hours", "24",
             "--output", str(evidence_path), "--end", "2026-08-28T14:00:00Z"],
            capture_output=True, text=True,
        )
        self.assertEqual(collect.returncode, 0, collect.stderr)
        evidence = json.loads(evidence_path.read_text(encoding="utf-8"))
        self.assertEqual(evidence["coverage"]["events_in_window"], 1)

        report_path = self.root / "out/health-2026-08-28.md"
        report_path.write_text(
            "# Health review\n"
            f'**Window:** {evidence["window"]["start"]} to {evidence["window"]["end"]} (end exclusive)\n'
            f'## Findings\n### Audit suppression\n[event:{evidence["events"][0]["id"]}] suppressed.\n'
            "## Latency\nNo voice turns.\n## Clean\nNone asserted.\n",
            encoding="utf-8",
        )
        ok = subprocess.run(
            [sys.executable, str(SCRIPT), "validate",
             "--evidence", str(evidence_path), "--report", str(report_path)],
            capture_output=True, text=True,
        )
        self.assertEqual(ok.returncode, 0, ok.stderr)
        self.assertIn("health report validation: ok", ok.stdout)

        report_path.write_text("# Health review\n## Findings\n### Bogus\nNo ID.\n", encoding="utf-8")
        bad = subprocess.run(
            [sys.executable, str(SCRIPT), "validate",
             "--evidence", str(evidence_path), "--report", str(report_path)],
            capture_output=True, text=True,
        )
        self.assertEqual(bad.returncode, 1)
        self.assertIn("ERROR:", bad.stderr)


if __name__ == "__main__":
    unittest.main()
