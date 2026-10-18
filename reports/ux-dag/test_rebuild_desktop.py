import hashlib
import json
from pathlib import Path
import argparse
import tempfile
import unittest
from unittest.mock import patch

import rebuild_desktop as rebuild


class CargoArtifactSelectionTests(unittest.TestCase):
    def make_log(self, root: Path, *, fresh: bool, include_deps: bool, exit_code: int = 0):
        target = root / 'target'
        release = target / 'release'
        deps = release / 'deps'
        deps.mkdir(parents=True)
        events = []
        for name in ('permagentd', 'permagent'):
            executable = release / name
            executable.write_bytes(f'possibly-stale-{name}'.encode())
            executable.chmod(0o755)
            filenames = []
            if include_deps:
                dep = deps / f'{name}-0123456789abcdef'
                dep.write_bytes(f'cargo-built-{name}'.encode())
                dep.chmod(0o755)
                filenames.append(str(dep))
            events.append(json.dumps({
                'reason': 'compiler-artifact', 'target': {'kind': ['bin'], 'name': name},
                'executable': str(executable), 'filenames': filenames, 'fresh': fresh,
            }))
        log = root / 'cargo.log'
        log.write_text('\n'.join(events) + '\n')
        log.with_suffix('.json').write_text(json.dumps({'target': str(target), 'exit_code': exit_code}))
        return target, log

    def run_parser(self, target: Path, log: Path):
        def fake_run(command, **kwargs):
            if command[1] == 'mkdir':
                Path(command[-1]).mkdir(parents=True, exist_ok=True)
                return type('Result', (), {'returncode': 0})()
            if command[1] == 'cp':
                import shutil
                shutil.copy2(command[-2], command[-1])
                return type('Result', (), {'returncode': 0})()
            raise AssertionError(f'unexpected command in artifact parser: {command}')

        with patch.object(rebuild, 'TARGET', target), patch.object(rebuild, 'RUN_BUILD', Path('/mock/run-build')), \
                patch.object(rebuild, 'ROOT', target.parent), patch.object(rebuild, 'guard_env', return_value={}), \
                patch.object(rebuild.subprocess, 'run', side_effect=fake_run):
            return rebuild.cargo_artifacts(log)

    def test_fresh_build_uses_fingerprinted_artifact_and_snapshots_it(self):
        with tempfile.TemporaryDirectory() as td:
            target, log = self.make_log(Path(td), fresh=True, include_deps=True)
            artifacts = self.run_parser(target, log)
            for name in ('permagentd', 'permagent'):
                info = artifacts[name]
                expected = hashlib.sha256(f'cargo-built-{name}'.encode()).hexdigest()
                self.assertEqual(info['artifact_kind'], 'cargo-fingerprinted-deps')
                self.assertEqual(info['sha256'], expected)
                self.assertEqual(Path(info['path']).read_bytes(), f'cargo-built-{name}'.encode())

    def test_fresh_event_without_fingerprinted_artifact_is_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            target, log = self.make_log(Path(td), fresh=True, include_deps=False)
            with self.assertRaisesRegex(RuntimeError, 'no fingerprinted deps executable'):
                self.run_parser(target, log)

    def test_failed_guard_receipt_is_rejected(self):
        with tempfile.TemporaryDirectory() as td:
            target, log = self.make_log(Path(td), fresh=True, include_deps=True, exit_code=1)
            with self.assertRaisesRegex(RuntimeError, 'successful build'):
                self.run_parser(target, log)

    def test_app_phase_keeps_app_and_backend_provenance_separate(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            app = root / 'release/bundle/macos/Permagent.app'
            artifacts = root / 'artifacts'
            manifest = root / 'manifest.json'
            data = {'backend_source': {'head': 'backend-full', 'short_head': '7620c68f'},
                    'phases': {'stage': {'resource_machos': []}}}
            captured = {}

            def fake_run(command, **kwargs):
                if command[1] == 'node':
                    app.mkdir(parents=True)
                elif command[1] == 'mkdir':
                    Path(command[-1]).mkdir(parents=True, exist_ok=True)
                elif command[1] == 'ditto':
                    Path(command[-1]).mkdir(parents=True)
                else:
                    raise AssertionError(f'unexpected mocked phase command: {command}')
                return type('Result', (), {'returncode': 0})()

            with patch.object(rebuild, 'backend_facts', return_value=(data, {})), \
                    patch.object(rebuild, 'require_clean'), \
                    patch.object(rebuild, 'git', return_value='app-full'), \
                    patch.object(rebuild, 'tree_hashes', return_value={'ui/command-center': 'tree'}), \
                    patch.object(rebuild, 'resource_facts', return_value={}), \
                    patch.object(rebuild, 'local_resource_machos', return_value=[]), \
                    patch.object(rebuild, 'signing_identity', return_value=None), \
                    patch.object(rebuild, 'guard_env', return_value={}), \
                    patch.object(rebuild, 'sign_resource_machos'), \
                    patch.object(rebuild, 'ui_facts', return_value=({'index': 'hash'}, {'marker': True})), \
                    patch.object(rebuild, 'save', side_effect=lambda value: captured.update(value)), \
                    patch.object(rebuild, 'RUN_BUILD', Path('/mock/run-build')), \
                    patch.object(rebuild, 'APP', app), patch.object(rebuild, 'ARTIFACTS', artifacts), \
                    patch.object(rebuild, 'MANIFEST', manifest), \
                    patch.object(rebuild.subprocess, 'run', side_effect=fake_run), \
                    patch.object(rebuild.time, 'strftime', return_value='20261018-120000'):
                rebuild.phase_app(argparse.Namespace())

            phase = captured['phases']['app']
            self.assertEqual(phase['source_head'], 'app-full')
            self.assertIn('Permagent-app-full-20261018-120000.app', phase['packaged_app'])
            self.assertEqual(captured['backend_source']['head'], 'backend-full')

    def test_signing_identity_requires_an_exact_authority_line(self):
        path = Path('/not-read')
        with patch.object(rebuild, 'signature_details', return_value='Authority=Developer ID Application: Team X Extra\n'):
            self.assertFalse(rebuild.has_identity(path, 'Developer ID Application: Team X'))
        with patch.object(rebuild, 'signature_details', return_value='Authority=Developer ID Application: Team X\n'):
            self.assertTrue(rebuild.has_identity(path, 'Developer ID Application: Team X'))

    def run_mocked_verify(self, root: Path, *, current_trees: dict[str, str]):
        artifacts = root / 'artifacts'
        app = artifacts / 'Permagent-test.app'
        for rel in (
            'Contents/MacOS/permagent-app', 'Contents/MacOS/permagentd', 'Contents/MacOS/permagent',
            'Contents/Frameworks/libsherpa-onnx-c-api.dylib', 'Contents/Frameworks/libonnxruntime.dylib',
            'Contents/Resources/permagent-audiocap', 'Contents/Resources/mcp-runtime/node',
            'Contents/Resources/mcp-runtime/node_modules/@brave/brave-search-mcp-server/package.json',
            'Contents/Resources/mcp-runtime/node_modules/tavily-mcp/package.json',
            'Contents/Resources/whisper/whisper-base-q8_0.gguf',
        ):
            path = app / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'fixture:' + rel.encode())
        tauri = root / 'tauri'
        source_files = {
            'binaries/libsherpa-onnx-c-api.dylib': b'fixture:Contents/Frameworks/libsherpa-onnx-c-api.dylib',
            'binaries/libonnxruntime.dylib': b'fixture:Contents/Frameworks/libonnxruntime.dylib',
            'audiocap/permagent-audiocap': b'fixture:Contents/Resources/permagent-audiocap',
            'resources/mcp-runtime/node': b'fixture:Contents/Resources/mcp-runtime/node',
            'resources/mcp-runtime/node_modules/@brave/brave-search-mcp-server/package.json': b'fixture:Contents/Resources/mcp-runtime/node_modules/@brave/brave-search-mcp-server/package.json',
            'resources/mcp-runtime/node_modules/tavily-mcp/package.json': b'fixture:Contents/Resources/mcp-runtime/node_modules/tavily-mcp/package.json',
        }
        for rel, content in source_files.items():
            path = tauri / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
        ui_input = root / 'ui-input.js'
        ui_input.write_text('fresh input')
        binary_paths = {}
        for name in ('permagentd', 'permagent'):
            path = root / f'cargo-{name}'
            path.write_bytes(f'cargo:{name}'.encode())
            binary_paths[name] = {'path': str(path), 'sha256': rebuild.sha(path)}
        data = {
            'backend_source': {'head': 'backend-rev', 'rust_trees': {'crates': 'rust-tree'}},
            'phases': {
                'stage': {'resources': {}, 'resource_machos': [], 'sidecars': {}},
                'app': {'packaged_app': str(app), 'source_head': 'app-rev',
                        'source_trees': {'ui/command-center': 'app-tree'},
                        'signing_identity': None, 'resource_machos': [],
                        'ui_hashes': {str(ui_input): rebuild.sha(ui_input)},
                        'ui_markers': {'permagent.world.legacy': True, 'world/vista/': True}},
            },
        }
        captured = {}
        with patch.object(rebuild, 'backend_facts', return_value=(data, binary_paths)), \
                patch.object(rebuild, 'ARTIFACTS', artifacts), patch.object(rebuild, 'TAURI', tauri), \
                patch.object(rebuild, 'MANIFEST', root / 'manifest.json'), \
                patch.object(rebuild, 'require_clean'), patch.object(rebuild, 'tree_hashes', return_value=current_trees), \
                patch.object(rebuild, 'resource_facts', return_value={}), \
                patch.object(rebuild, 'text_section_sha', return_value='same-text'), \
                patch.object(rebuild, 'is_adhoc', return_value=True), \
                patch.object(rebuild, 'sha', wraps=rebuild.sha), \
                patch.object(rebuild, 'save', side_effect=lambda value: captured.update(value)), \
                patch.object(rebuild.subprocess, 'run', return_value=type('Result', (), {'returncode': 0})()), \
                patch.object(rebuild.subprocess, 'check_output', return_value='ai.permagent.desktop'), \
                patch.object(rebuild, 'WHISPER_SHA', rebuild.sha(app / 'Contents/Resources/whisper/whisper-base-q8_0.gguf')):
            rebuild.phase_verify(argparse.Namespace())
        return captured

    def test_verify_phase_accepts_new_app_manifest_handoff(self):
        with tempfile.TemporaryDirectory() as td:
            trees = {'ui/command-center': 'app-tree'}
            captured = self.run_mocked_verify(Path(td), current_trees=trees)
            verification = captured['phases']['verify']
            self.assertEqual(verification['backend_source']['head'], 'backend-rev')
            self.assertEqual(verification['app_source']['head'], 'app-rev')
            self.assertEqual(verification['bundle_identifier'], 'ai.permagent.desktop')

    def test_verify_phase_rejects_changed_app_source_tree(self):
        with tempfile.TemporaryDirectory() as td:
            with self.assertRaisesRegex(RuntimeError, 'source trees changed after app build'):
                self.run_mocked_verify(Path(td), current_trees={'ui/command-center': 'changed-tree'})


if __name__ == '__main__':
    unittest.main()
