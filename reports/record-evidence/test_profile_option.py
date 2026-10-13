"""--profile {debug,release}: path resolution and fail-closed library discovery. No daemon, no signing."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import blank_native_daemon as blank
import isolated_daemon as isolated


class ProfileTests(unittest.TestCase):
    def setUp(self):
        self.addCleanup(isolated.set_profile, 'debug')
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.target = Path(self.tmp.name).resolve()
        for name in ('debug', 'release'):
            (self.target / name).mkdir()
        patcher = patch.object(isolated, 'TARGET', self.target)
        patcher.start()
        self.addCleanup(patcher.stop)
        isolated.set_profile('debug')

    def libs(self, profile):
        d = self.target / profile
        for name in ('libsherpa-onnx-c-api.dylib', 'libonnxruntime.1.dylib'):
            (d / name).write_bytes(b'x')

    def test_default_is_debug_and_release_resolves(self):
        self.assertEqual((isolated.PROFILE, isolated.DEBUG), ('debug', self.target / 'debug'))
        isolated.set_profile('release')
        self.assertEqual((isolated.PROFILE, isolated.DEBUG), ('release', self.target / 'release'))
        self.assertEqual(isolated.child_env()['DYLD_LIBRARY_PATH'], str(self.target / 'release'))

    def test_unknown_profile_rejected(self):
        for bad in ('Release', '../x', '', 'debug/..'):
            with self.assertRaisesRegex(RuntimeError, 'Unknown build profile'):
                isolated.set_profile(bad)
        self.assertEqual(isolated.PROFILE, 'debug')

    def test_release_libraries_fail_closed_without_borrowing_debug(self):
        self.libs('debug')
        isolated.set_profile('release')
        with self.assertRaisesRegex(RuntimeError, 'release dylibs are missing'):
            isolated.libraries()
        with self.assertRaisesRegex(RuntimeError, 'release dylibs are missing'):
            isolated.sign_libraries()

    def test_release_libraries_found_and_symlink_escape_rejected(self):
        self.libs('release')
        isolated.set_profile('release')
        self.assertTrue(all(p.parent == self.target / 'release' for p in isolated.libraries()))
        (self.target / 'release' / 'libonnxruntime.1.dylib').unlink()
        (self.target / 'release' / 'libonnxruntime.1.dylib').symlink_to(self.target / 'debug' / 'libsherpa-onnx-c-api.dylib')
        (self.target / 'debug' / 'libsherpa-onnx-c-api.dylib').write_bytes(b'x')
        with self.assertRaisesRegex(RuntimeError, 'symlink escapes owned release target'):
            isolated.libraries()

    def test_blank_runner_only_runs_release_binary_under_release_profile(self):
        self.libs('release')
        binary = self.target / 'release' / 'permagentd'
        binary.write_bytes(b'daemon')
        binary.chmod(0o700)
        sha = isolated.digest(binary)
        isolated.set_profile('release')
        with patch.object(blank.subprocess, 'run'):
            got = blank.reviewed_artifacts(sha)
        self.assertEqual(got['binary']['path'], str(binary))
        # A debug-only binary is never accepted for a release run.
        binary.unlink()
        (self.target / 'debug' / 'permagentd').write_bytes(b'daemon')
        with self.assertRaises(RuntimeError):
            blank.reviewed_artifacts(sha)

    def test_manifest_binary_must_match_active_profile(self):
        isolated.set_profile('release')
        self.assertEqual(str(isolated.DEBUG / 'permagentd'), str(self.target / 'release/permagentd'))

    def test_cli_flag_parsed_default_debug_and_release(self):
        for argv, expected in ((['x'], 'debug'), (['x', '--profile', 'release'], 'release')):
            with patch.object(sys, 'argv', argv + ['--sha256', '0' * 64]):
                with self.assertRaisesRegex(RuntimeError, 'Refusing launch without --execute'):
                    blank.main()
            self.assertEqual(isolated.PROFILE, expected)
        with patch.object(sys, 'argv', ['x', '--profile', 'bogus', '--sha256', '0' * 64]):
            with self.assertRaises(SystemExit):
                blank.main()


if __name__ == '__main__':
    unittest.main()
