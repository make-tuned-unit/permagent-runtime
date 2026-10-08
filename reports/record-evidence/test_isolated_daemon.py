"""No daemon, permission request, network call, production files or signing."""
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from contextlib import contextmanager, ExitStack, redirect_stdout
from io import StringIO
from types import SimpleNamespace
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('isolated_daemon', Path(__file__).with_name('isolated_daemon.py'))
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


@contextmanager
def owned_fixture(prepared=True):
    with tempfile.TemporaryDirectory() as directory, ExitStack() as stack:
        base = Path(directory).resolve()
        root, debug = base / 'record-qa-runtime', base / 'debug'
        root.mkdir()
        debug.mkdir()
        binary = debug / 'permagentd'
        binary.write_bytes(b'synthetic daemon old')
        binary.chmod(0o700)
        libs = [debug / 'libsherpa-onnx-c-api.dylib', debug / 'libonnxruntime.1.dylib']
        for path in libs:
            path.write_bytes(b'synthetic dylib')
        whisper = base / runner.WHISPER_NAME
        whisper.write_bytes(b'synthetic whisper fixture')
        for key, value in {'ROOT': root, 'DEBUG': debug, 'MARKER': root / 'record-review-owner.json',
                           'ACTIVE': root / 'record-review-active.json', 'STOP': root / 'record-review-stop.json',
                           'WHISPER_SHA': runner.digest(whisper)}.items():
            stack.enter_context(patch.object(runner, key, value))
        stack.enter_context(patch.object(runner, 'port_free'))  # No socket operations in pure tests.
        signing = stack.enter_context(patch.object(runner.subprocess, 'run'))  # No signing/execution.
        stack.enter_context(redirect_stdout(StringIO()))
        if prepared:
            runner.write_json(runner.MARKER, {'schema': 1, 'root': str(root), 'url': runner.URL,
                'owner': 'a' * 32, 'binary': runner.artifact(binary), 'whisper': runner.artifact(whisper),
                'libraries': [runner.artifact(p) for p in libs]})
        yield SimpleNamespace(root=root, debug=debug, binary=binary, whisper=whisper, libs=libs, signing=signing)


def refresh_args(fixture):
    return SimpleNamespace(binary=str(fixture.binary), sha256=runner.digest(fixture.binary),
                           previous_manifest_sha256=runner.digest(runner.MARKER))

class IsolationTests(unittest.TestCase):
    def test_clean_environment_does_not_import_credentials_or_redirects(self):
        with patch.dict(os.environ, {'OPENAI_API_KEY': 'synthetic', 'HTTP_PROXY': 'bad',
                                      'PERMAGENT_PATH_ROOT': '/bad', 'DYLD_LIBRARY_PATH': '/bad'}):
            env = runner.child_env()
        self.assertNotIn('OPENAI_API_KEY', env)
        self.assertNotIn('HTTP_PROXY', env)
        self.assertEqual(env['HOME'], os.environ['HOME'])
        self.assertEqual(env['PERMAGENT_PATH_ROOT'], str(runner.ROOT))
        self.assertEqual(env['DYLD_LIBRARY_PATH'], str(runner.DEBUG))
        self.assertEqual(env['MEETING_WRITEUP_LOCAL_ONLY'], 'true')
        self.assertEqual(env['GOOSE_MODEL'], 'qwen2.5:7b')
        self.assertNotIn('WIZARD_COMPLETE', env)

    def test_artifact_rejects_wrong_hash_and_symlink(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory).resolve() / 'fixture'
            source.write_bytes(b'fixture')
            actual = runner.artifact(source)
            self.assertEqual(runner.artifact(source, actual['sha256']), actual)
            with self.assertRaises(RuntimeError): runner.artifact(source, '0' * 64)
            link = source.with_name('link')
            link.symlink_to(source)
            with self.assertRaises(RuntimeError): runner.artifact(link)

    def test_local_defaults_disable_background_opt_ins_and_starters(self):
        config = runner.config()
        for key in ['wizard_complete', 'council_enabled', 'concierge_enabled', 'strix_enabled',
                    'steward_scan_enabled', 'polybot_enabled', 'tunnel_auto_start']:
            self.assertIs(config[key], False)
        self.assertEqual(len(runner.STARTERS), 5)
        self.assertIs(config['meeting_writeup_local_only'], True)

    def test_prepare_allows_only_native_heartbeat_and_preserves_it(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / 'logs').mkdir()
            heartbeat = root / 'logs/ui-heartbeat'
            heartbeat.write_text('1234567890')
            with patch.object(runner, 'ROOT', root):
                runner.fresh_root_check()
                self.assertEqual(heartbeat.read_text(), '1234567890')
                (root / 'config.yaml').write_text('{}')
                with self.assertRaisesRegex(RuntimeError, 'fresh state'): runner.fresh_root_check()

    def test_occupied_port_fails_without_signaling(self):
        with patch.object(runner.socket, 'socket') as factory:
            factory.return_value.__enter__.return_value.bind.side_effect = OSError('occupied')
            with self.assertRaisesRegex(RuntimeError, 'occupied'): runner.port_free()

    def test_metadata_written_private_and_ownership_manifest_checked(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve() / 'record-qa-runtime'
            root.mkdir()
            marker = root / 'record-review-owner.json'
            with patch.object(runner, 'ROOT', root), patch.object(runner, 'MARKER', marker):
                runner.write_json(marker, {'schema': 1, 'root': '/wrong', 'url': runner.URL})
                self.assertEqual(marker.stat().st_mode & 0o777, 0o600)
                with self.assertRaisesRegex(RuntimeError, 'mismatch'): runner.manifest()

    def test_refresh_accepts_new_hash_preserves_data_owner_and_records_old_artifacts(self):
        with owned_fixture() as fixture:
            sentinel = fixture.root / 'spectral/permagent.db'
            sentinel.parent.mkdir()
            sentinel.write_bytes(b'synthetic existing database bytes')
            config = fixture.root / 'config.yaml'
            config.write_bytes(b'custom synthetic settings')
            previous = json.loads(runner.MARKER.read_text())
            fixture.binary.write_bytes(b'synthetic daemon rebuilt')
            fixture.libs[0].write_bytes(b'synthetic dylib rebuilt')
            with self.assertRaisesRegex(RuntimeError, 'SHA-256 mismatch'): runner.manifest()
            runner.refresh_artifacts(refresh_args(fixture))
            updated = runner.manifest()
            self.assertEqual(updated['owner'], previous['owner'])
            self.assertNotEqual(updated['binary']['sha256'], previous['binary']['sha256'])
            self.assertEqual(sentinel.read_bytes(), b'synthetic existing database bytes')
            self.assertEqual(config.read_bytes(), b'custom synthetic settings')
            evidence = list(fixture.root.glob('record-review-artifact-refresh-*.json'))
            self.assertEqual(len(evidence), 1)
            self.assertEqual(json.loads(evidence[0].read_text())['previous'], previous)
            self.assertTrue(all('--verify' in c.args[0] for c in fixture.signing.call_args_list))

    def test_refresh_rejects_stale_manifest_hash_without_writes(self):
        with owned_fixture() as fixture:
            before = runner.MARKER.read_bytes()
            args = refresh_args(fixture)
            args.previous_manifest_sha256 = '0' * 64
            with self.assertRaisesRegex(RuntimeError, 'SHA-256 mismatch'): runner.refresh_artifacts(args)
            self.assertEqual(runner.MARKER.read_bytes(), before)
            self.assertEqual(list(fixture.root.glob('record-review-artifact-refresh-*.json')), [])

    def test_refresh_rejects_active_runner_changed_library_identity_and_wrong_binary_hash(self):
        with owned_fixture() as fixture:
            before = runner.MARKER.read_bytes()
            runner.ACTIVE.write_text('{}')
            with self.assertRaisesRegex(RuntimeError, 'shutdown'): runner.refresh_artifacts(refresh_args(fixture))
            runner.ACTIVE.unlink()
            args = refresh_args(fixture)
            args.sha256 = '0' * 64
            with self.assertRaisesRegex(RuntimeError, 'SHA-256 mismatch'): runner.refresh_artifacts(args)
            (fixture.debug / 'libonnxruntime.unexpected.dylib').write_bytes(b'unexpected')
            with self.assertRaisesRegex(RuntimeError, 'dylib identities'): runner.refresh_artifacts(refresh_args(fixture))
            self.assertEqual(runner.MARKER.read_bytes(), before)

    def test_refresh_rejects_corrupt_installed_whisper(self):
        with owned_fixture() as fixture:
            installed = fixture.root / 'models' / runner.WHISPER_NAME
            installed.parent.mkdir()
            installed.write_bytes(b'incorrect model')
            with self.assertRaisesRegex(RuntimeError, 'SHA-256 mismatch'): runner.refresh_artifacts(refresh_args(fixture))

    def test_setup_brain_creates_private_exact_ontology_and_is_idempotent(self):
        with owned_fixture() as fixture:
            before = runner.MARKER.read_bytes()
            runner.setup_brain()
            path = fixture.root / 'brain/ontology.toml'
            self.assertEqual(path.read_bytes(), b'version = 1\n')
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            timestamp = path.stat().st_mtime_ns
            runner.setup_brain()
            self.assertEqual(path.stat().st_mtime_ns, timestamp)
            self.assertEqual(runner.MARKER.read_bytes(), before)

    def test_setup_brain_preserves_nonempty_ontology_and_rejects_symlink(self):
        with owned_fixture() as fixture:
            brain = fixture.root / 'brain'
            brain.mkdir()
            path = brain / 'ontology.toml'
            path.write_bytes(b'version = 1\n# existing custom ontology\n')
            before = path.read_bytes()
            with self.assertRaisesRegex(RuntimeError, 'refusing to replace'): runner.setup_brain()
            self.assertEqual(path.read_bytes(), before)
            path.unlink()
            path.symlink_to(fixture.whisper)
            with self.assertRaisesRegex(RuntimeError, 'symlinks'): runner.setup_brain()

    def test_prepare_includes_ontology_and_run_state_is_not_reset(self):
        with owned_fixture(prepared=False) as fixture:
            args = SimpleNamespace(binary=str(fixture.binary), sha256=runner.digest(fixture.binary),
                                   whisper=str(fixture.whisper))
            runner.prepare(args)
            self.assertEqual((fixture.root / 'brain/ontology.toml').read_bytes(), b'version = 1\n')
            runner.manifest()
            with self.assertRaisesRegex(RuntimeError, 'fresh state'): runner.prepare(args)

if __name__ == '__main__': unittest.main()
