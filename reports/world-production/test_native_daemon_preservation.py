import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from native_daemon_preservation import DaemonPreservation, digest
import run_guarded_native as runner
import run_record_native_tests as native_tests


class PreservationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / 'debug').mkdir()
        self.path = self.root / 'debug/permagentd'
        self.path.write_bytes(b'new source-built daemon')
        self.path.chmod(0o755)
        self.original = digest(self.path)

    def test_tauri_in_place_copy_is_restored_with_independent_backup(self):
        guard = DaemonPreservation(self.root)
        guard.prepare()
        self.assertNotEqual(self.path.stat().st_ino, guard.backup.stat().st_ino)
        self.path.write_bytes(b'old installed sidecar')
        guard.restore(group_stopped=True)
        self.assertEqual(digest(self.path), self.original)
        self.assertTrue(guard.evidence['restored'])
        self.assertEqual(digest(guard.backup), self.original)
        self.assertTrue(self.path.stat().st_mode & 0o100)

    def test_unchanged_and_missing_destination(self):
        guard = DaemonPreservation(self.root)
        guard.prepare()
        inode = self.path.stat().st_ino
        guard.restore(group_stopped=True)
        self.assertEqual(guard.evidence['status'], 'unchanged')
        self.assertEqual(self.path.stat().st_ino, inode)
        self.path.unlink()
        guard.restore(group_stopped=True)
        self.assertEqual(digest(self.path), self.original)

    def test_live_group_and_corrupt_backup_refuse_restore(self):
        guard = DaemonPreservation(self.root)
        guard.prepare()
        self.path.write_bytes(b'old sidecar')
        with self.assertRaisesRegex(RuntimeError, 'group'):
            guard.restore(group_stopped=False)
        self.assertEqual(self.path.read_bytes(), b'old sidecar')
        guard.backup.write_bytes(b'corrupt')
        with self.assertRaisesRegex(RuntimeError, 'hash changed'):
            guard.restore(group_stopped=True)
        self.assertEqual(self.path.read_bytes(), b'old sidecar')

    def test_symlinks_rejected_before_build_and_restore(self):
        guard = DaemonPreservation(self.root)
        guard.prepare()
        other = self.root / 'other'
        other.write_bytes(b'untouched')
        self.path.unlink()
        self.path.symlink_to(other)
        with self.assertRaisesRegex(RuntimeError, 'canonical'):
            guard.restore(group_stopped=True)
        with self.assertRaisesRegex(RuntimeError, 'canonical'):
            DaemonPreservation(self.root).prepare()
        self.assertEqual(other.read_bytes(), b'untouched')

    def test_no_preexisting_artifact_is_explicit(self):
        self.path.unlink()
        guard = DaemonPreservation(self.root)
        guard.prepare()
        guard.restore(group_stopped=True)
        self.assertEqual(guard.evidence['status'], 'no_preexisting_daemon')
        self.assertFalse(self.path.exists())

    def test_runner_restores_on_failed_build_and_launch_error(self):
        for module, mode in [(module, mode) for module in (runner, native_tests)
                             for mode in ('success', 'failure', 'launch_error', 'interrupt')]:
            with self.subTest(mode=mode):
                reports = self.root / (module.__name__ + mode)
                reports.mkdir()
                (reports / 'native-review-config.json').write_text('{}')
                def spawn(*args, **kwargs):
                    self.path.write_bytes(b'old installed sidecar')
                    if mode == 'launch_error':
                        raise OSError('simulated launch failure')
                    if mode == 'interrupt':
                        raise KeyboardInterrupt('simulated interruption')
                    process = unittest.mock.Mock()
                    process.poll.return_value = 0 if mode == 'success' else 17
                    process.pid = 12345
                    return process
                with patch.object(runner, 'TARGET', self.root), patch.object(runner, 'REPORTS', reports), \
                     patch.object(runner, 'preflight'), patch.object(runner, 'free_gib', return_value=30), \
                     patch.object(runner, 'wait_owned_group', return_value=True), \
                     patch.object(runner.shutil, 'which', return_value='/fake/node'), \
                     patch.object(runner.subprocess, 'Popen', side_effect=spawn), \
                     patch('sys.argv', ['guard', mode] if module is runner else ['native-tests']), contextlib.redirect_stdout(io.StringIO()), \
                     contextlib.redirect_stderr(io.StringIO()), \
                     patch('native_daemon_preservation.sys.platform', 'test'):
                    code = module.main()
                self.assertEqual(code, 0) if mode == 'success' else self.assertNotEqual(code, 0)
                self.assertEqual(digest(self.path), self.original)
                summary = json.loads(next(p for p in reports.glob('*.json') if p.name != 'native-review-config.json').read_text())
                self.assertTrue(summary['daemon_artifact_preservation']['restored'])
                self.assertIsNone(summary['daemon_preservation_error'])

    def test_runner_restore_error_cannot_report_success(self):
        for module in (runner, native_tests):
            with self.subTest(runner=module.__name__):
                self.check_restore_error(module)

    def check_restore_error(self, module):
        reports = self.root / module.__name__
        reports.mkdir()
        (reports / 'native-review-config.json').write_text('{}')
        process = unittest.mock.Mock()
        process.poll.return_value = 0
        process.pid = 12345
        with patch.object(runner, 'TARGET', self.root), patch.object(runner, 'REPORTS', reports), \
             patch.object(runner, 'preflight'), patch.object(runner, 'free_gib', return_value=30), \
             patch.object(runner, 'wait_owned_group', return_value=True), \
             patch.object(runner.shutil, 'which', return_value='/fake/node'), \
             patch.object(runner.subprocess, 'Popen', return_value=process), \
             patch.object(DaemonPreservation, 'restore', side_effect=OSError('disk failure')), \
             patch('sys.argv', ['guard', 'restore-error'] if module is runner else ['native-tests']), contextlib.redirect_stdout(io.StringIO()), \
             contextlib.redirect_stderr(io.StringIO()), \
             patch('native_daemon_preservation.sys.platform', 'test'):
            code = module.main()
        self.assertEqual(code, 1)
        summary = json.loads(next(p for p in reports.glob('*.json') if p.name != 'native-review-config.json').read_text())
        self.assertIn('disk failure', summary['daemon_preservation_error'])
        backup = Path(summary['daemon_artifact_preservation']['backup'])
        self.assertEqual(digest(backup), self.original)


if __name__ == '__main__': unittest.main()
