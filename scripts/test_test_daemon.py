"""No-build regression tests for the real daemon test entry point."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("test-daemon.sh").resolve()


class DaemonTestRunnerTests(unittest.TestCase):
    def run_gate(self, *args, platform="Darwin", signature_fail=False, missing=False):
        with tempfile.TemporaryDirectory(prefix="permagent-test-gate-") as directory:
            root = Path(directory)
            bindir = root / "bin"
            bindir.mkdir()
            target = root / "target with spaces"
            for profile in ("debug", "release"):
                (target / profile).mkdir(parents=True)
                if not missing:
                    (target / profile / "libonnxruntime.1.27.0.dylib").touch()
            log = root / "calls"
            commands = {
                "uname": '#!/bin/bash\nprintf "%s\\n" "$FIXTURE_PLATFORM"\n',
                "cargo": '''#!/bin/bash
if [[ "$1" == metadata ]]; then
  printf '%s\\n' "$FIXTURE_METADATA"
  exit 0
fi
printf 'cargo' >> "$FIXTURE_LOG"
printf ' <%s>' "$@" >> "$FIXTURE_LOG"
printf '\\n' >> "$FIXTURE_LOG"
''',
                "codesign": '''#!/bin/bash
printf 'codesign' >> "$FIXTURE_LOG"
printf ' <%s>' "$@" >> "$FIXTURE_LOG"
printf '\\n' >> "$FIXTURE_LOG"
if [[ "$1" == --verify && "$FIXTURE_SIGNATURE_FAIL" == 1 ]]; then exit 1; fi
''',
            }
            for name, content in commands.items():
                path = bindir / name
                path.write_text(content)
                path.chmod(0o755)
            env = dict(os.environ)
            env.pop("TEST_DAEMON_PROFILE", None)
            env.update(
                PATH=f"{bindir}:{env['PATH']}",
                FIXTURE_PLATFORM=platform,
                FIXTURE_METADATA=json.dumps({"target_directory": str(target)}, separators=(",", ":")),
                FIXTURE_LOG=str(log),
                FIXTURE_SIGNATURE_FAIL="1" if signature_fail else "0",
            )
            result = subprocess.run(["/bin/bash", str(SCRIPT), *args], env=env,
                                    capture_output=True, text=True, timeout=10)
            return result, log.read_text().splitlines() if log.exists() else []

    def test_focused_scope_does_not_compile_integration_targets(self):
        result, calls = self.run_gate("--lib", "projection")
        self.assertEqual(result.returncode, 0, result.stderr)
        cargo = [line for line in calls if line.startswith("cargo")]
        self.assertEqual(len(cargo), 2)
        self.assertTrue(all("<--lib>" in line and "<--tests>" not in line for line in cargo))
        self.assertIn("<--no-run>", cargo[0])
        self.assertNotIn("<--no-run>", cargo[1])
        self.assertTrue(all("<projection>" in line for line in cargo))
        self.assertTrue(any("<--verify> <--strict>" in line for line in calls))

    def test_default_retains_lib_and_integration_execution(self):
        result, calls = self.run_gate()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(all("<--lib> <--tests>" in line for line in calls if line.startswith("cargo")))

    def test_release_scope_signs_resolved_profile_including_spaces(self):
        result, calls = self.run_gate("--lib", "--release", "projection")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(all("<--release>" in line for line in calls if line.startswith("cargo")))
        self.assertTrue(all("target with spaces/release/" in line for line in calls if line.startswith("codesign")))

    def test_signature_failure_prevents_test_execution(self):
        result, calls = self.run_gate("--lib", signature_fail=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(sum(line.startswith("cargo") for line in calls), 1)

    def test_missing_dylibs_prevents_test_execution(self):
        result, calls = self.run_gate("--lib", missing=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(sum(line.startswith("cargo") for line in calls), 1)

    def test_non_macos_executes_selected_scope_without_signing(self):
        result, calls = self.run_gate("--lib", "projection", platform="Linux")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ["cargo <test> <-p> <permagent-daemon> <--lib> <projection>"])

    def test_extra_arguments_fail_instead_of_silently_ignoring(self):
        result, calls = self.run_gate("--lib", "projection", "unexpected")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
