"""Guard regression tests: fake processes/signals only; no builds or real kills."""
import contextlib
import io
import json
from pathlib import Path
import signal
import tempfile
import unittest
from unittest.mock import Mock, patch
import run_guarded_native as guard
import run_record_backend_tests as backend
import run_record_native_tests as native_tests


class BuildGuardTests(unittest.TestCase):
    def test_eperm_probe_uses_process_table_and_distinguishes_live_zombie_absent(self):
        for rows, expected in [('321 S\n', True), ('321 Z\n', False), ('322 S\n', False)]:
            with self.subTest(rows=rows), patch.object(guard.os, 'killpg', side_effect=PermissionError('probe denied')), patch.object(guard.subprocess, 'run', return_value=Mock(stdout=rows)):
                self.assertEqual(guard.group_alive(321), expected)

    def test_signal_denied_waits_and_accepts_only_observed_exit(self):
        proc = Mock(pid=321)
        with patch.object(guard.os, 'killpg', side_effect=PermissionError('signal denied')), patch.object(guard, 'wait_owned_group', return_value=True) as wait:
            guard.stop_owned_group(proc)
        wait.assert_called_once_with(proc)
        proc.wait.assert_called_once()

    def test_signal_denied_and_still_live_retains_failure(self):
        proc = Mock(pid=321)
        with patch.object(guard.os, 'killpg', side_effect=PermissionError('signal denied')), patch.object(guard, 'wait_owned_group', return_value=False):
            with self.assertRaisesRegex(PermissionError, 'signal denied'):
                guard.stop_owned_group(proc)
        proc.wait.assert_not_called()

    def test_kill_denied_and_still_live_is_not_reported_stopped(self):
        proc = Mock(pid=321)
        with patch.object(guard.os, 'killpg', side_effect=[None, PermissionError('kill denied')]) as kill, patch.object(guard, 'wait_owned_group', return_value=False):
            with self.assertRaisesRegex(PermissionError, 'kill denied'):
                guard.stop_owned_group(proc)
        self.assertEqual(kill.call_args_list[1].args, (321, signal.SIGKILL))
        proc.wait.assert_not_called()

    def test_wait_is_bounded_and_reaps_the_leader_before_probe(self):
        proc = Mock(pid=321)
        def probe(_):
            proc.poll.assert_called()
            return True
        with patch.object(guard.time, 'monotonic', side_effect=[0, 0, 11]), patch.object(guard.time, 'sleep'), patch.object(guard, 'group_alive', side_effect=probe):
            self.assertFalse(guard.wait_owned_group(proc))
        self.assertEqual(proc.poll.call_count, 2)

    def test_both_runners_preserve_disk_floor_reason_when_cleanup_fails(self):
        for module, argv in [(guard, ['guard', 'unit']), (backend, ['backend']), (native_tests, ['native-tests'])]:
            with self.subTest(runner=module.__name__), tempfile.TemporaryDirectory() as temp, contextlib.ExitStack() as stack:
                stack.enter_context(patch.object(guard, 'REPORTS', Path(temp)))
                stack.enter_context(patch.object(guard, 'TARGET', Path(temp).resolve() / 'target'))
                stack.enter_context(patch.object(guard, 'preflight'))
                stack.enter_context(patch.object(guard, 'free_gib', return_value=19.57))
                stack.enter_context(patch.object(guard.shutil, 'which', return_value='/usr/bin/node'))
                stack.enter_context(patch.object(guard.subprocess, 'Popen', return_value=Mock(pid=321)))
                stopped = stack.enter_context(patch.object(guard, 'stop_owned_group', side_effect=PermissionError('fixture cleanup denied')))
                stack.enter_context(patch.object(guard.sys, 'argv', argv))
                stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
                stack.enter_context(contextlib.redirect_stderr(io.StringIO()))
                # Backend reads only this configuration; no native subprocess runs.
                (Path(temp) / 'native-review-config.json').write_text('{}')
                self.assertEqual(module.main(), 1)
                summaries = [p for p in Path(temp).glob('*.json') if p.name != 'native-review-config.json']
                self.assertEqual(len(summaries), 1)
                summary = json.loads(summaries[0].read_text())
                self.assertIn('19.57', summary['reason'])
                self.assertIn('PermissionError', summary['cleanup_error'])
                self.assertFalse(summary['owned_group_stopped'])
                stopped.assert_called_once()  # Never re-enter cleanup from its own exception.


if __name__ == '__main__':
    unittest.main()
