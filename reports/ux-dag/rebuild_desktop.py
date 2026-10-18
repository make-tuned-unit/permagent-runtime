#!/usr/bin/env python3
"""Guarded phased rebuild of the Permagent macOS app.

Run backend, stage, app, verify in order. All cargo/tauri work goes through the
existing /Volumes/PermagentBuilds/permagent/run-build guard. This script does
not install or launch the app.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
TARGET = Path('/Volumes/PermagentBuilds/permagent/targets/world-production-20261006')
RUN_BUILD = Path('/Volumes/PermagentBuilds/permagent/run-build')
TAURI = ROOT / 'ui/desktop/src-tauri'
UI = ROOT / 'ui/command-center'
UI_DIST = TARGET / 'ux-ui-dist'
APP = TARGET / 'release/bundle/macos/Permagent.app'
ARTIFACTS = TARGET / 'artifacts'
MANIFEST = ROOT / 'reports/ux-dag/desktop-rebuild-manifest.json'
TRIPLE = 'aarch64-apple-darwin'
WHISPER_SHA = '7073e51db7ab02b38cc4fceeac39adc2d7a19beb98badf66aa708f4f0ac71aa9'
NODE_VERSION = 'v22.22.1'
RUST_PATHS = ('.cargo', 'crates', 'Cargo.toml', 'Cargo.lock')
APP_PATHS = ('ui/command-center', 'ui/desktop', 'ui/analytics-client')


def sha(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def git(*args: str) -> str:
    return subprocess.check_output(['git', '-C', str(ROOT), *args], text=True).strip()


def git_diff_sha(*paths: str, base: str = 'HEAD') -> str:
    patch = subprocess.check_output(['git', '-C', str(ROOT), 'diff', '--binary', base, '--', *paths])
    return hashlib.sha256(patch).hexdigest()


def tree_hashes(revision: str, paths: tuple[str, ...]) -> dict[str, str]:
    return {path: git('rev-parse', f'{revision}:{path}') for path in paths}


def require_clean(paths: tuple[str, ...], label: str) -> None:
    tracked = subprocess.run(['git', '-C', str(ROOT), 'diff', '--quiet', 'HEAD', '--', *paths], check=False)
    if tracked.returncode:
        raise RuntimeError(f'{label} tracked sources changed; commit them before this phase')
    status = git('status', '--porcelain', '--untracked-files=all', '--', *paths)
    if status:
        raise RuntimeError(f'{label} sources have untracked/dirty files; commit or remove them before this phase: {status}')


def load() -> dict:
    if MANIFEST.exists():
        return json.loads(MANIFEST.read_text())
    return {'schema': 1, 'worktree': str(ROOT), 'target': str(TARGET), 'phases': {}}


def save(data: dict) -> None:
    MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    tmp = MANIFEST.with_suffix('.tmp')
    tmp.write_text(json.dumps(data, indent=2) + '\n')
    tmp.replace(MANIFEST)


def guard_env() -> dict:
    if not RUN_BUILD.is_file() or not os.access(RUN_BUILD, os.X_OK):
        raise RuntimeError(f'guard launcher missing: {RUN_BUILD}')
    env = os.environ.copy()
    env.update(CARGO_TARGET_DIR=str(TARGET), CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0')
    return env


def guarded(args: list[str], cwd: Path, *, capture: bool = False) -> str:
    result = subprocess.run([str(RUN_BUILD), *args], cwd=cwd, env=guard_env(),
                            text=True, stdout=subprocess.PIPE if capture else None,
                            stderr=subprocess.STDOUT if capture else None)
    if result.returncode:
        raise RuntimeError(f'guarded command failed ({result.returncode}): {args}\n{result.stdout or ""}')
    return result.stdout or ''


def receipt_for_log(log: Path) -> dict:
    receipt = log.with_suffix('.json')
    if not receipt.is_file():
        raise RuntimeError(f'guard receipt missing beside Cargo log: {receipt}')
    data = json.loads(receipt.read_text())
    if data.get('exit_code') != 0 or data.get('target') != str(TARGET):
        raise RuntimeError(f'Cargo guard receipt is not a successful build for {TARGET}: {receipt}')
    return data


def cargo_artifacts(log: Path) -> dict:
    receipt_for_log(log)
    found: dict[str, list[dict]] = {'permagentd': [], 'permagent': []}
    for line in log.read_text(errors='replace').splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get('reason') != 'compiler-artifact':
            continue
        target = event.get('target', {})
        name = target.get('name')
        if name in found and 'bin' in target.get('kind', []):
            found[name].append(event)
    result = {}
    for name, events in found.items():
        if len(events) != 1:
            raise RuntimeError(f'Cargo log must identify exactly one compiler-artifact event for {name}; got {len(events)}')
        event = events[0]
        executable = Path(event.get('executable') or '').resolve()
        if executable != (TARGET / f'release/{name}').resolve():
            raise RuntimeError(f'Cargo reported unexpected executable path for {name}: {executable}')
        # Cargo also reports the fingerprinted executable in filenames. Prefer
        # that artifact because Tauri may have replaced the conventional release
        # path while Cargo still considers its build fresh.
        dep_paths = set()
        for filename in event.get('filenames', []):
            candidate = Path(filename).resolve()
            if (candidate.parent == (TARGET / 'release/deps').resolve()
                    and re.fullmatch(rf'{re.escape(name)}-[0-9a-f]{{16}}', candidate.name)
                    and candidate.is_file() and os.access(candidate, os.X_OK)):
                dep_paths.add(candidate)
        if len(dep_paths) > 1:
            raise RuntimeError(f'Cargo reported multiple fingerprinted executable artifacts for {name}: {sorted(map(str, dep_paths))}')
        if dep_paths:
            source = next(iter(dep_paths))
            artifact_kind = 'cargo-fingerprinted-deps'
        else:
            if event.get('fresh') is True:
                raise RuntimeError(f'{name} is fresh but Cargo log has no fingerprinted deps executable; refusing possibly stale release/{name}')
            if not executable.is_file() or not os.access(executable, os.X_OK):
                raise RuntimeError(f'Cargo executable is missing or not executable: {executable}')
            source = executable
            artifact_kind = 'cargo-just-built-executable'
        # Snapshot the Cargo-identified source before Tauri overwrites release/{name}.
        digest = sha(source)
        snapshot = TARGET / 'ux-artifacts' / f'{name}-{digest}'
        subprocess.run([str(RUN_BUILD), 'mkdir', '-p', str(snapshot.parent)], cwd=ROOT,
                       env=guard_env(), check=True)
        if snapshot.exists() and sha(snapshot) != digest:
            raise RuntimeError(f'content-addressed artifact collision: {snapshot}')
        if not snapshot.exists():
            subprocess.run([str(RUN_BUILD), 'cp', str(source), str(snapshot)], cwd=ROOT,
                           env=guard_env(), check=True)
        if sha(snapshot) != digest:
            raise RuntimeError(f'Cargo artifact snapshot failed hash check: {snapshot}')
        if source != executable and event.get('fresh') is False and text_section_sha(source) != text_section_sha(executable):
            raise RuntimeError(f'Cargo deps artifact and release executable disagree for {name}')
        result[name] = {'path': str(snapshot), 'cargo_executable': str(executable),
                        'cargo_artifact_source': str(source), 'artifact_kind': artifact_kind,
                        'cargo_fresh': event.get('fresh'), 'sha256': digest,
                        'bytes': snapshot.stat().st_size}
    return result


def phase_backend(args: argparse.Namespace) -> None:
    if args.cargo_log:
        if not args.source_head:
            raise RuntimeError('--source-head is required when importing a completed Cargo log')
        source_head = git('rev-parse', args.source_head)
        log = Path(args.cargo_log).resolve()
        artifacts = cargo_artifacts(log)
        command_log = str(log)
    else:
        source_head = git('rev-parse', 'HEAD')
        # run-build records Cargo's JSON stream in its own exact log path.
        output = guarded(['cargo', 'build', '--release', '--locked', '--bin', 'permagentd',
                          '--bin', 'permagent', '--message-format=json'], ROOT, capture=True)
        match = re.search(r'^Build log: (.+)$', output, re.M)
        if not match:
            raise RuntimeError('run-build did not report its Cargo log path')
        log = Path(match.group(1)).resolve()
        artifacts = cargo_artifacts(log)
        command_log = str(log)
    source_trees = tree_hashes(source_head, RUST_PATHS)
    current_trees = tree_hashes('HEAD', RUST_PATHS)
    require_clean(RUST_PATHS, 'Rust backend')
    if source_trees != current_trees:
        raise RuntimeError('Rust/Cargo source trees changed since the backend build started')
    data = load()
    data['backend_source'] = {'head': source_head, 'short_head': git('rev-parse', '--short=9', source_head),
                              'rust_trees': source_trees, 'rust_worktree_diff_sha256': git_diff_sha(*RUST_PATHS)}
    data['phases']['backend'] = {'guarded_log': command_log, 'artifacts': artifacts,
                                 'recorded_at': time.strftime('%Y-%m-%dT%H:%M:%S%z')}
    save(data)
    print(json.dumps(data['phases']['backend'], indent=2))


def backend_facts() -> tuple[dict, dict]:
    data = load()
    phase = data.get('phases', {}).get('backend')
    if not phase:
        raise RuntimeError('run backend phase first')
    backend_source = data.get('backend_source', {})
    if tree_hashes('HEAD', RUST_PATHS) != backend_source.get('rust_trees'):
        raise RuntimeError('Rust/Cargo source trees changed since backend build; rebuild the backend phase')
    require_clean(RUST_PATHS, 'Rust backend')
    if git_diff_sha(*RUST_PATHS) != backend_source.get('rust_worktree_diff_sha256'):
        raise RuntimeError('Rust working-tree source differs from backend build provenance')
    arts = phase['artifacts']
    for name in ('permagentd', 'permagent'):
        p = Path(arts[name]['path'])
        if not p.is_file() or sha(p) != arts[name]['sha256']:
            raise RuntimeError(f'{name} Cargo artifact changed or disappeared: {p}')
    return data, arts


def resource_facts() -> dict:
    helper = TAURI / 'audiocap/permagent-audiocap'
    committed = subprocess.check_output(['git', '-C', str(ROOT), 'show', f"HEAD:{helper.relative_to(ROOT)}"])
    if hashlib.sha256(committed).hexdigest() != sha(helper):
        raise RuntimeError('tracked audiocap helper differs from HEAD; refusing to bundle it')
    mcp = TAURI / 'resources/mcp-runtime'
    node = mcp / 'node'
    if not node.is_file() or subprocess.check_output([str(node), '--version'], text=True).strip() != NODE_VERSION:
        raise RuntimeError(f'required pinned bundled Node is not staged: {node}')
    expected = {
        mcp / 'node_modules/@brave/brave-search-mcp-server/package.json': '2.0.85',
        mcp / 'node_modules/tavily-mcp/package.json': '0.2.20',
    }
    for exact, version in expected.items():
        if not exact.is_file():
            raise RuntimeError(f'required pinned MCP package is absent: {exact}')
        if json.loads(exact.read_text()).get('version') != version:
            raise RuntimeError(f'MCP package version mismatch at {exact}; expected {version}')
    whisper = TAURI / 'resources/whisper/whisper-base-q8_0.gguf'
    if not whisper.is_file() or sha(whisper) != WHISPER_SHA:
        raise RuntimeError(f'Whisper resource absent or hash mismatch: {whisper}')
    paths = [node, mcp / 'node_modules/@brave/brave-search-mcp-server/package.json',
             mcp / 'node_modules/tavily-mcp/package.json', whisper, helper]
    return {str(p): {'sha256': sha(p), 'bytes': p.stat().st_size} for p in paths}


def local_resource_machos() -> list[dict]:
    """Enumerate local resource inputs and map each Mach-O to its exact bundle path."""
    roots = [
        (TAURI / 'resources/mcp-runtime', Path('Contents/Resources/mcp-runtime')),
        (TAURI / 'resources/whisper', Path('Contents/Resources/whisper')),
    ]
    result = []
    for source_root, bundle_root in roots:
        if not source_root.is_dir():
            raise RuntimeError(f'resource input directory missing: {source_root}')
        for parent, dirs, files in os.walk(source_root):
            dirs.sort()
            for filename in sorted(files):
                source = Path(parent) / filename
                desc = subprocess.run(['file', '-b', str(source)], text=True, errors='replace',
                                      stdout=subprocess.PIPE, check=True).stdout
                if 'Mach-O' in desc:
                    relative = source.relative_to(source_root)
                    result.append({'source': str(source), 'bundle_relative': str(bundle_root / relative),
                                   'sha256': sha(source), 'text_sha256': text_section_sha(source)})
    helper = TAURI / 'audiocap/permagent-audiocap'
    desc = subprocess.run(['file', '-b', str(helper)], text=True, errors='replace', stdout=subprocess.PIPE, check=True).stdout
    if 'Mach-O' in desc:
        result.append({'source': str(helper), 'bundle_relative': 'Contents/Resources/permagent-audiocap',
                       'sha256': sha(helper), 'text_sha256': text_section_sha(helper)})
    return result


def phase_stage(_: argparse.Namespace) -> None:
    data, arts = backend_facts()
    env = guard_env()
    # Tauri copies these conventional release paths over the freshly compiled files;
    # restore them from the Cargo-selected artifact before staging.
    for name, dest in (('permagentd', TARGET / 'release/permagentd'),
                       ('permagent', TARGET / 'release/permagent')):
        subprocess.run([str(RUN_BUILD), 'cp', arts[name]['path'], str(dest)], cwd=ROOT,
                       env=guard_env(), check=True)
        if sha(dest) != arts[name]['sha256']:
            raise RuntimeError(f'copy verification failed for {dest}')
    for script in ('copy-sidecar.sh', 'copy-cli.sh'):
        subprocess.run([str(RUN_BUILD), str(TAURI.parent / 'scripts' / script)], cwd=ROOT,
                       env=env, check=True)
    staged = {}
    for name, path in (
        ('daemon', TAURI / f'binaries/permagentd-{TRIPLE}'),
        ('cli', TAURI / f'binaries/permagent-{TRIPLE}'),
        ('sherpa', TAURI / 'binaries/libsherpa-onnx-c-api.dylib'),
        ('onnxruntime', TAURI / 'binaries/libonnxruntime.dylib'),
    ):
        if not path.is_file():
            raise RuntimeError(f'sidecar script did not stage {path}')
        subprocess.run(['codesign', '--verify', '--strict', str(path)], check=True)
        staged[name] = {'path': str(path), 'sha256': sha(path), 'bytes': path.stat().st_size}
    runtime = resource_facts()
    data['phases']['stage'] = {'sidecars': staged, 'resources': runtime,
                               'resource_machos': local_resource_machos(),
                               'recorded_at': time.strftime('%Y-%m-%dT%H:%M:%S%z')}
    save(data)
    print(json.dumps(data['phases']['stage'], indent=2))


def vite_command() -> str:
    return f"cd ../command-center && TAURI_ENV_PLATFORM=darwin npm run build -- --outDir {UI_DIST} --manifest"


def ui_facts(source_revision: str) -> tuple[dict, dict]:
    ui_manifest = UI_DIST / '.vite/manifest.json'
    index = UI_DIST / 'index.html'
    if not ui_manifest.is_file() or not index.is_file():
        raise RuntimeError('Tauri beforeBuildCommand did not produce expected external dist and manifest')
    outputs = json.loads(ui_manifest.read_text())
    hashes = {str(index): sha(index), str(ui_manifest): sha(ui_manifest)}
    markers = {'permagent.world.legacy': False, 'world/vista/': False,
               'w/ Subscription': False, 'cursor-agent': False, source_revision: False}
    for entry in outputs.values():
        for rel in [entry.get('file'), *entry.get('css', []), *entry.get('assets', [])]:
            if not rel:
                continue
            path = UI_DIST / rel
            if not path.is_file():
                raise RuntimeError(f'Vite manifest output missing: {path}')
            hashes[str(path)] = sha(path)
            if path.suffix == '.js':
                content = path.read_bytes()
                for marker in markers:
                    markers[marker] |= marker.encode() in content
    if not all(markers.values()):
        raise RuntimeError(f'fresh Vite JavaScript lacks expected UI markers: {markers}')
    return hashes, markers


def phase_app(_: argparse.Namespace) -> None:
    data, _ = backend_facts()
    if not data.get('phases', {}).get('stage'):
        raise RuntimeError('run stage phase first')
    require_clean(APP_PATHS, 'UI/Tauri app')
    app_source_head = git('rev-parse', 'HEAD')
    app_source_trees = tree_hashes(app_source_head, APP_PATHS)
    resource_facts()
    source_machos = local_resource_machos()
    if source_machos != data['phases']['stage'].get('resource_machos'):
        raise RuntimeError('resource Mach-O provenance changed after stage phase')
    config = {
        'build': {'frontendDist': str(UI_DIST), 'beforeBuildCommand': vite_command()},
        'bundle': {'createUpdaterArtifacts': False},
    }
    cfg = json.dumps(config, separators=(',', ':'))
    # Tauri's beforeBuildCommand performs the one frontend build to the external
    # target. Keep the native build in a single outer launcher invocation.
    signing = signing_identity()
    env = guard_env()
    env['VITE_GIT_SHA'] = app_source_head[:9]
    if signing:
        env['APPLE_SIGNING_IDENTITY'] = signing
    else:
        env.pop('APPLE_SIGNING_IDENTITY', None)
    cli = UI / 'node_modules/@tauri-apps/cli/tauri.js'
    run = subprocess.run([str(RUN_BUILD), 'node', str(cli), 'build', '--bundles', 'app', '--config', cfg, '--', '--locked'],
                         cwd=ROOT / 'ui/desktop', env=env)
    if run.returncode:
        raise RuntimeError(f'guarded Tauri build failed with exit {run.returncode}')
    if not APP.is_dir():
        raise RuntimeError(f'Tauri did not produce expected app: {APP}')
    require_clean(APP_PATHS, 'UI/Tauri app')
    if tree_hashes('HEAD', APP_PATHS) != app_source_trees:
        raise RuntimeError('UI/Tauri source trees changed during app build')
    sign_resource_machos(APP, source_machos, signing)
    ui_hashes, ui_markers = ui_facts(app_source_head[:9])
    subprocess.run([str(RUN_BUILD), 'mkdir', '-p', str(ARTIFACTS)], cwd=ROOT,
                   env=guard_env(), check=True)
    packaged = ARTIFACTS / f"Permagent-{app_source_head[:9]}-{time.strftime('%Y%m%d-%H%M%S')}.app"
    if packaged.exists():
        raise RuntimeError(f'refusing to overwrite packaged app: {packaged}')
    subprocess.run([str(RUN_BUILD), 'ditto', str(APP), str(packaged)], cwd=ROOT,
                   env=guard_env(), check=True)
    data['phases']['app'] = {
        'source_head': app_source_head, 'source_trees': app_source_trees,
        'app': str(APP), 'packaged_app': str(packaged), 'ui_dist': str(UI_DIST), 'ui_hashes': ui_hashes,
        'ui_markers': ui_markers,
        'resource_machos': source_machos,
        'tauri_config': config, 'signing_identity': signing,
        'recorded_at': time.strftime('%Y-%m-%dT%H:%M:%S%z'),
    }
    save(data)
    print(f'Built {APP}; signing identity: {signing or "ad hoc / none available"}')


def text_section_sha(path: Path) -> str:
    import tempfile
    with tempfile.TemporaryDirectory(prefix='permagent-section-') as td:
        out = Path(td) / 'text'
        subprocess.run(['segedit', str(path), '-extract', '__TEXT', '__text', str(out)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        return sha(out)


def signing_identity() -> str | None:
    output = subprocess.run(['security', 'find-identity', '-v', '-p', 'codesigning'],
                            text=True, stdout=subprocess.PIPE, check=False).stdout
    return next((line.split('"')[1] for line in output.splitlines()
                 if '"Developer ID Application:' in line), None)


def signature_details(path: Path) -> str:
    output = subprocess.run(['codesign', '-dvv', str(path)], text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False).stdout
    return output


def has_identity(path: Path, identity: str) -> bool:
    return any(line.strip() == f'Authority={identity}' for line in signature_details(path).splitlines())


def is_adhoc(path: Path) -> bool:
    return any(line.strip() == 'Signature=adhoc' for line in signature_details(path).splitlines())


def sign_resource_machos(app: Path, machos: list[dict], identity: str | None) -> None:
    entitlements = TAURI / 'Entitlements.plist'
    changed = False
    for item in machos:
        path = app / item['bundle_relative']
        if not path.is_file() or text_section_sha(path) != item['text_sha256']:
            raise RuntimeError(f'bundled resource Mach-O code differs from source: {path}')
        if identity and not has_identity(path, identity):
            subprocess.run(['codesign', '--force', '--options', 'runtime', '--timestamp',
                            '--entitlements', str(entitlements), '--sign', identity, str(path)], check=True)
            changed = True
        elif not identity and subprocess.run(['codesign', '--verify', '--strict', str(path)], check=False).returncode:
            subprocess.run(['codesign', '--force', '--sign', '-', str(path)], check=True)
            changed = True
        subprocess.run(['codesign', '--verify', '--strict', str(path)], check=True)
        if identity and not has_identity(path, identity):
            raise RuntimeError(f'resource Mach-O lacks the selected signing identity: {path}')
    if changed:
        if identity:
            subprocess.run(['codesign', '--force', '--options', 'runtime', '--timestamp',
                            '--entitlements', str(entitlements), '--sign', identity, str(app)], check=True)
        else:
            subprocess.run(['codesign', '--force', '--deep', '--sign', '-', str(app)], check=True)
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)


def phase_verify(_: argparse.Namespace) -> None:
    data, arts = backend_facts()
    phase = data.get('phases', {}).get('app')
    stage = data.get('phases', {}).get('stage')
    if not phase or not stage:
        raise RuntimeError('run stage and app phases first')
    app = Path(phase['packaged_app'])
    if not app.is_dir() or app.parent != ARTIFACTS:
        raise RuntimeError(f'unexpected or absent app path: {app}')
    subprocess.run(['codesign', '--verify', '--deep', '--strict', '--verbose=2', str(app)], check=True)
    expected_identity = phase.get('signing_identity')
    signed_paths = [app, app / 'Contents/MacOS/permagent-app',
                    app / 'Contents/MacOS/permagentd', app / 'Contents/MacOS/permagent',
                    app / 'Contents/Frameworks/libsherpa-onnx-c-api.dylib',
                    app / 'Contents/Frameworks/libonnxruntime.dylib']
    for path in signed_paths:
        if expected_identity and not has_identity(path, expected_identity):
            raise RuntimeError(f'{path} does not carry the selected signing identity {expected_identity}')
        if not expected_identity and not is_adhoc(path):
            raise RuntimeError(f'{path} is not ad-hoc signed and no Developer ID identity was selected')
    require_clean(APP_PATHS, 'UI/Tauri app')
    if tree_hashes('HEAD', APP_PATHS) != phase.get('source_trees'):
        raise RuntimeError('UI/Tauri source trees changed after app build')
    bundle_id = subprocess.check_output(['/usr/libexec/PlistBuddy', '-c', 'Print :CFBundleIdentifier',
                                         str(app / 'Contents/Info.plist')], text=True).strip()
    if bundle_id != 'ai.permagent.desktop':
        raise RuntimeError(f'app bundle identifier changed: {bundle_id}')
    actual = {}
    for name in ('permagentd', 'permagent'):
        bundled = app / f'Contents/MacOS/{name}'
        if not bundled.is_file() or text_section_sha(bundled) != text_section_sha(Path(arts[name]['path'])):
            raise RuntimeError(f'bundled {name} code differs from recorded Cargo artifact')
        actual[name] = {'path': str(bundled), 'sha256': sha(bundled),
                        'text_sha256': text_section_sha(bundled),
                        'cargo_sha256': arts[name]['sha256']}
    resources = resource_facts()
    if resources != stage.get('resources'):
        raise RuntimeError('staged resource provenance changed after the stage phase')
    if phase.get('resource_machos') != stage.get('resource_machos'):
        raise RuntimeError('resource Mach-O provenance changed after staging')
    for path, expected in phase.get('ui_hashes', {}).items():
        exact = Path(path)
        if not exact.is_file() or sha(exact) != expected:
            raise RuntimeError(f'frontend build input changed after app build: {exact}')
    fixed = {
        'daemon': app / 'Contents/MacOS/permagentd',
        'cli': app / 'Contents/MacOS/permagent',
        'sherpa': app / 'Contents/Frameworks/libsherpa-onnx-c-api.dylib',
        'onnxruntime': app / 'Contents/Frameworks/libonnxruntime.dylib',
        'audiocap': app / 'Contents/Resources/permagent-audiocap',
        'node': app / 'Contents/Resources/mcp-runtime/node',
        'brave_manifest': app / 'Contents/Resources/mcp-runtime/node_modules/@brave/brave-search-mcp-server/package.json',
        'tavily_manifest': app / 'Contents/Resources/mcp-runtime/node_modules/tavily-mcp/package.json',
        'whisper': app / 'Contents/Resources/whisper/whisper-base-q8_0.gguf',
    }
    resource_hashes = {}
    source_for = {'daemon': 'daemon', 'cli': 'cli', 'sherpa': 'sherpa', 'onnxruntime': 'onnxruntime'}
    staged_frameworks = {
        'sherpa': TAURI / 'binaries/libsherpa-onnx-c-api.dylib',
        'onnxruntime': TAURI / 'binaries/libonnxruntime.dylib',
    }
    for name, path in fixed.items():
        if not path.is_file():
            raise RuntimeError(f'bundle resource missing: {path}')
        digest = sha(path)
        if name in source_for:
            # install_name_tool/signing alter whole Mach-O bytes; compare executable code for binaries.
            if name in ('daemon', 'cli') and text_section_sha(path) != text_section_sha(Path(arts[{'daemon': 'permagentd', 'cli': 'permagent'}[name]]['path'])):
                raise RuntimeError(f'bundled {name} text differs from Cargo artifact')
            if name in ('sherpa', 'onnxruntime') and text_section_sha(path) != text_section_sha(staged_frameworks[name]):
                raise RuntimeError(f'bundled {name} code differs from staged framework')
            if name in ('sherpa', 'onnxruntime'):
                subprocess.run(['codesign', '--verify', '--strict', str(path)], check=True)
        elif name == 'audiocap':
            helper = TAURI / 'audiocap/permagent-audiocap'
            if text_section_sha(path) != text_section_sha(helper):
                raise RuntimeError('bundled audiocap code differs from HEAD-tracked helper')
        elif name == 'node':
            src = TAURI / 'resources/mcp-runtime/node'
            if text_section_sha(path) != text_section_sha(src):
                raise RuntimeError('bundled Node code differs from staged runtime')
        elif name in ('brave_manifest', 'tavily_manifest'):
            src = (TAURI / 'resources/mcp-runtime/node_modules/@brave/brave-search-mcp-server/package.json'
                   if name == 'brave_manifest' else TAURI / 'resources/mcp-runtime/node_modules/tavily-mcp/package.json')
            if digest != sha(src):
                raise RuntimeError(f'bundled {name} differs from staged runtime')
        elif name == 'whisper':
            if digest != WHISPER_SHA:
                raise RuntimeError('bundled Whisper model hash mismatch')
        resource_hashes[name] = {'path': str(path), 'sha256': digest}
    for item in phase['resource_machos']:
        path = app / item['bundle_relative']
        if not path.is_file() or text_section_sha(path) != item['text_sha256']:
            raise RuntimeError(f'bundled resource Mach-O code differs from recorded source: {path}')
        subprocess.run(['codesign', '--verify', '--strict', str(path)], check=True)
        if phase.get('signing_identity') and not has_identity(path, phase['signing_identity']):
            raise RuntimeError(f'bundled resource Mach-O lacks the selected signing identity: {path}')
        resource_hashes[item['bundle_relative']] = {
            'path': str(path), 'sha256': sha(path), 'text_sha256': item['text_sha256']}
    # The executable contains compressed Tauri assets. Source markers are checked
    # in the exact Vite-manifest outputs above; the recorded config binds that dist
    # directory to this build. Do not search the external bundle recursively.
    if not all(phase.get('ui_markers', {}).values()):
        raise RuntimeError('recorded Vite output did not contain the expected UI markers')
    main = app / 'Contents/MacOS/permagent-app'
    data['phases']['verify'] = {
        'app': str(app), 'app_sha256': sha(main), 'bundle_identifier': bundle_id,
        'signature': 'codesign --deep --strict passed',
        'binaries': actual, 'resources': resource_hashes,
        'backend_source': data['backend_source'],
        'app_source': {'head': phase['source_head'], 'trees': phase['source_trees']},
        'ui_hashes': phase['ui_hashes'], 'recorded_at': time.strftime('%Y-%m-%dT%H:%M:%S%z'),
    }
    save(data)
    print(json.dumps(data['phases']['verify'], indent=2))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('phase', choices=('backend', 'stage', 'app', 'verify'))
    parser.add_argument('--cargo-log', help='exact run-build Cargo JSON log; its adjacent guard receipt must be successful')
    parser.add_argument('--source-head', help='required with --cargo-log; commit used when the backend build started')
    args = parser.parse_args()
    try:
        {'backend': phase_backend, 'stage': phase_stage, 'app': phase_app, 'verify': phase_verify}[args.phase](args)
    except (OSError, subprocess.CalledProcessError, RuntimeError, ValueError, KeyError) as exc:
        print(f'ERROR: {exc}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
