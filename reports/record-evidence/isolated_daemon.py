#!/usr/bin/env python3
"""Explicit isolated Record daemon preparation/supervision; never touches port 3001.

No command builds, stages an app, imports credentials, or starts Ollama.
prepare writes only a fresh owned root. run is the only daemon-launch command.
"""
from __future__ import annotations
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request
import uuid

LANE = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(LANE / 'reports/world-production'))
from run_guarded_native import stop_owned_group

TARGET = Path(os.environ.get('PERMAGENT_BUILD_TARGET', str(LANE.parent / '.shared-target/world-production-20261006')))
ROOT = TARGET / 'record-qa-runtime'
PROFILES = ('debug', 'release')
PROFILE = 'debug'
# Despite the historical name, DEBUG is the ACTIVE profile directory (TARGET/<profile>);
# it is debug unless set_profile('release') is called before any artifact is touched.
DEBUG = TARGET / PROFILE


def set_profile(name):
    global PROFILE, DEBUG
    if name not in PROFILES:
        raise RuntimeError(f'Unknown build profile: {name!r}')
    PROFILE = name
    DEBUG = TARGET / name
    return DEBUG
PORT = 3002
URL = f'http://127.0.0.1:{PORT}'
MODEL = 'qwen2.5:7b'
WHISPER_NAME = 'whisper-base-q8_0.gguf'
WHISPER_SHA = '7073e51db7ab02b38cc4fceeac39adc2d7a19beb98badf66aa708f4f0ac71aa9'
MARKER = ROOT / 'record-review-owner.json'
ACTIVE = ROOT / 'record-review-active.json'
STOP = ROOT / 'record-review-stop.json'
EMPTY_ONTOLOGY = b'version = 1\n'
STARTERS = ['model_review', 'workspace-snapshot', 'storage-insights', 'git-steward', 'health-review']


def digest(path):
    h = hashlib.sha256()
    with Path(path).open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    # Do not expose credentials in these documents; all stored values are metadata.
    tmp = path.with_name(path.name + '.tmp')
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as f:
        json.dump(value, f, indent=2)
        f.write('\n')
    os.replace(tmp, path)


def root_check():
    if ROOT.name != 'record-qa-runtime' or ROOT.resolve() != ROOT or not ROOT.is_dir():
        raise RuntimeError('Review root must exist at its exact canonical path')
    if ROOT == Path.home() or '.permagent' in ROOT.parts:
        raise RuntimeError('Production root is forbidden')


def port_free():
    with socket.socket() as sock:
        try:
            sock.bind(('127.0.0.1', PORT))
        except OSError as e:
            raise RuntimeError('Port 3002 is occupied; refusing to launch or kill a listener') from e


def artifact(path, expected=None):
    path = Path(path)
    if not path.is_absolute() or path.resolve() != path or not path.is_file():
        raise RuntimeError('Artifact must be an absolute canonical regular file')
    actual = digest(path)
    if expected is not None and (not re.fullmatch('[0-9a-f]{64}', expected) or actual != expected):
        raise RuntimeError(f'Artifact SHA-256 mismatch: {path}')
    return {'path': str(path), 'sha256': actual}


def libraries():
    items = [DEBUG / 'libsherpa-onnx-c-api.dylib', *sorted(DEBUG.glob('libonnxruntime*.dylib'))]
    if len(items) < 2 or not all(p.is_file() for p in items):
        raise RuntimeError(f'Expected sherpa and onnxruntime {PROFILE} dylibs are missing in {DEBUG} (libraries of another profile are never borrowed)')
    resolved = sorted(set(p.resolve() for p in items))
    if any(not p.is_relative_to(DEBUG.resolve()) for p in resolved):
        raise RuntimeError(f'Dylib symlink escapes owned {PROFILE} target')
    return resolved


def sign_libraries():
    # Only explicit sign-libs mutates build products; installed app is never a target.
    for path in libraries():
        subprocess.run(['/usr/bin/codesign', '--force', '--sign', '-', str(path)], check=True)
        subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(path)], check=True)
    print(json.dumps({'signed_libraries': [artifact(p) for p in libraries()]}, indent=2))


def config():
    value = {'GOOSE_PROVIDER': 'ollama', 'GOOSE_MODEL': MODEL, 'GOOSE_MODE': 'chat',
             'OLLAMA_HOST': 'http://127.0.0.1:11434', 'meeting_writeup_local_only': True,
             'chat_provider': 'ollama', 'chat_model': MODEL,
             'voice_provider': 'ollama', 'voice_model': MODEL,
             'wizard_complete': False, 'GOOSE_TELEMETRY_ENABLED': False,
             'council_enabled': False, 'concierge_enabled': False, 'strix_enabled': False,
             'steward_scan_enabled': False, 'polybot_enabled': False, 'tunnel_auto_start': False}
    for pack in ['LOCAL', 'EDIT', 'HARD', 'MECHANICAL', 'REVIEW']:
        value[f'PERMAGENT_PACK_{pack}_PROVIDER'] = 'ollama'
        value[f'PERMAGENT_PACK_{pack}_MODEL'] = MODEL
    return value


def child_env():
    # Deliberately do not inherit API keys, proxies, provider config or DYLD values.
    env = {'PATH': '/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin',
           'HOME': os.environ['HOME'], 'LANG': 'en_US.UTF-8',
           'PERMAGENT_PATH_ROOT': str(ROOT), 'PERMAGENT_DISABLE_KEYRING': '1',
           'PERMAGENT_MESH_TRUSTED': '0', 'PERMAGENT_MESH_ENGINE': '0',
           'PERMAGENT_OLLAMA_HOST': 'http://localhost:11434',
           'DYLD_LIBRARY_PATH': str(DEBUG)}
    for key, value in config().items():
        if key == 'wizard_complete':
            continue  # UI can complete the wizard persistently.
        env[key.upper()] = str(value).lower() if isinstance(value, bool) else str(value)
    return env


def fresh_root_check():
    # The explicitly isolated native app may already have stamped its liveness file.
    # Permit only that exact source-proven artifact; retain it without reading/deleting.
    for path in ROOT.rglob('*'):
        relative = path.relative_to(ROOT).as_posix()
        valid = ((relative == 'logs' and path.is_dir()) or
                 (relative == 'logs/ui-heartbeat' and path.is_file() and path.stat().st_size <= 32))
        if path.is_symlink() or not valid:
            raise RuntimeError('prepare requires fresh state (only native logs/ui-heartbeat is permitted)')


def resume_root_check(requested):
    """Staging-only resume gate: accept only the exact canonical owned root, no wipe.

    Returns a one-line summary. Never reads secrets; refuses symlinks, anything
    outside TARGET/record-qa-runtime, production paths and a root with a live daemon.
    """
    text = str(requested)
    if not os.path.isabs(text) or os.path.normpath(text) != text:
        raise RuntimeError('--resume-root must be an absolute normalised path')
    path = Path(text)
    if path != ROOT or ROOT != TARGET / 'record-qa-runtime':
        raise RuntimeError(f'--resume-root accepts only the canonical owned staging root: {ROOT}')
    home = Path.home()
    for forbidden in (home / '.permagent', Path('/Applications')):
        if forbidden == path or forbidden in path.parents or path in forbidden.parents:
            raise RuntimeError('--resume-root may never point at a production location')
    if '.permagent' in path.parts or path == home:
        raise RuntimeError('--resume-root may never point at a production location')
    probe = Path(path.anchor)
    for part in path.parts[1:]:
        probe = probe / part
        if probe.is_symlink():
            raise RuntimeError(f'--resume-root path component is a symlink: {probe}')
    if not path.is_dir() or path.resolve() != path:
        raise RuntimeError('--resume-root must be an existing canonical directory')
    if path.stat().st_mode & 0o077:
        raise RuntimeError('--resume-root must be private (0700)')
    if ACTIVE.exists() or ACTIVE.is_symlink() or STOP.exists() or STOP.is_symlink():
        raise RuntimeError('--resume-root refused: an owned runner/stop marker remains')
    # The blank runner records daemonPid/supervisorPid in an evidence file; an
    # unfinished record whose pid is alive means a daemon still owns this root.
    for evidence in sorted(path.glob('blank-native-daemon-*.json')):
        try:
            record = json.loads(evidence.read_text())
        except (OSError, ValueError):
            continue
        if record.get('finishedUtc'):
            continue
        for key in ('daemonPid', 'supervisorPid'):
            pid = record.get(key)
            if isinstance(pid, int) and pid != os.getpid():
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    continue
                except PermissionError:
                    pass
                raise RuntimeError(f'--resume-root refused: {key} {pid} from {evidence.name} is still alive')
    port_free()
    names = sorted(entry.name for entry in path.iterdir())
    return (f'resuming existing staging profile at {path}: {len(names)} top-level entries '
            f'(config.yaml={"yes" if "config.yaml" in names else "no"}, '
            f'secrets={"yes" if "secrets" in names else "no"}, '
            f'spectral={"yes" if "spectral" in names else "no"}, '
            f'previous runs={len(list(path.glob("blank-native-daemon-*.json")))})')


def reviewed_artifacts(binary_path, binary_sha, whisper_path):
    binary = artifact(binary_path, binary_sha)
    if Path(binary['path']) != DEBUG / 'permagentd' or not os.access(binary['path'], os.X_OK):
        raise RuntimeError(f'Only the owned {PROFILE}/permagentd executable is allowed')
    whisper = artifact(whisper_path, WHISPER_SHA)
    if Path(whisper['path']).name != WHISPER_NAME:
        raise RuntimeError('Unexpected bundled Whisper filename')
    libs = libraries()
    for path in libs:
        subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(path)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    return {'binary': binary, 'whisper': whisper, 'libraries': [artifact(p) for p in libs]}


def prepare(args):
    root_check()
    fresh_root_check()
    selected = reviewed_artifacts(args.binary, args.sha256, args.whisper)
    manifest = {'schema': 1, 'root': str(ROOT), 'url': URL, 'owner': uuid.uuid4().hex, **selected}
    write_json(ROOT / 'config.yaml', config())  # JSON is valid YAML.
    write_json(ROOT / 'automation/disabled_starters.json', STARTERS)
    write_json(ROOT / 'librarian_schedule.json', {
        'enabled': False, 'start_time': '02:00', 'duration_minutes': 240, 'model': MODEL,
        'run_if_launched_in_window': False, 'pruning_enabled': False})
    ensure_empty_ontology()
    write_json(MARKER, manifest)
    print(json.dumps({'prepared': str(ROOT), 'binary': selected['binary'], 'wizard_complete': False,
                      'ontology': 'empty version 1'}))


def owned_manifest():
    """Validate ownership and artifact identities without accepting changed content hashes."""
    root_check()
    if MARKER.resolve() != MARKER or not MARKER.is_file() or MARKER.stat().st_mode & 0o077:
        raise RuntimeError('Ownership manifest must be a private canonical regular file')
    value = json.loads(MARKER.read_text())
    if (value.get('root') != str(ROOT) or value.get('url') != URL or value.get('schema') != 1 or
            not re.fullmatch('[0-9a-f]{32}', value.get('owner', ''))):
        raise RuntimeError('Review ownership manifest mismatch')
    if value['binary']['path'] != str(DEBUG / 'permagentd'):
        raise RuntimeError('Ownership manifest binary identity mismatch')
    whisper = value['whisper']
    if Path(whisper['path']).name != WHISPER_NAME or whisper['sha256'] != WHISPER_SHA:
        raise RuntimeError('Ownership manifest Whisper identity mismatch')
    library_paths = [entry['path'] for entry in value['libraries']]
    if len(library_paths) < 2 or len(set(library_paths)) != len(library_paths):
        raise RuntimeError('Ownership manifest dylib identities are incomplete or duplicated')
    for entry in [value['binary'], whisper, *value['libraries']]:
        path = Path(entry['path'])
        if not path.is_absolute() or path.resolve() != path or not re.fullmatch('[0-9a-f]{64}', entry['sha256']):
            raise RuntimeError('Ownership manifest contains an invalid artifact identity')
    for path in map(Path, library_paths):
        if path.parent != DEBUG or not (path.name == 'libsherpa-onnx-c-api.dylib' or
                                       re.fullmatch(r'libonnxruntime.*\.dylib', path.name)):
            raise RuntimeError(f'Ownership manifest dylib escapes the owned {PROFILE} target')
    return value


def manifest():
    value = owned_manifest()
    for entry in [value['binary'], value['whisper'], *value['libraries']]:
        artifact(entry['path'], entry['sha256'])
    return value


def require_stopped():
    if ACTIVE.exists() or ACTIVE.is_symlink() or STOP.exists() or STOP.is_symlink():
        raise RuntimeError('Owned runner/stop marker remains; finish supervised shutdown before setup or refresh')
    port_free()


def ensure_empty_ontology():
    """Create only this one minimal file; never import or replace an ontology."""
    root_check()
    directory = ROOT / 'brain'
    if directory.resolve() != directory or directory.is_symlink():
        raise RuntimeError('Brain directory must remain inside the canonical isolated root')
    directory.mkdir(mode=0o700, exist_ok=True)
    path = directory / 'ontology.toml'
    if path.is_symlink() or path.resolve() != path:
        raise RuntimeError('Ontology symlinks are forbidden')
    if path.exists():
        if not path.is_file() or path.read_bytes() != EMPTY_ONTOLOGY:
            raise RuntimeError('Existing ontology is not the empty review ontology; refusing to replace it')
        return False
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'wb') as handle:
        handle.write(EMPTY_ONTOLOGY)
        handle.flush()
        os.fsync(handle.fileno())
    return True


def setup_brain():
    manifest()
    require_stopped()
    created = ensure_empty_ontology()
    print(json.dumps({'ontology': str(ROOT / 'brain/ontology.toml'), 'created': created,
                      'content': EMPTY_ONTOLOGY.decode(), 'launched': False}))


def refresh_artifacts(args):
    previous = owned_manifest()
    require_stopped()
    # Explicit old-manifest hash prevents blessing a different prior review or racing a refresh.
    artifact(MARKER, args.previous_manifest_sha256)
    if args.binary != previous['binary']['path']:
        raise RuntimeError('Artifact refresh may not change the daemon path')
    selected = reviewed_artifacts(args.binary, args.sha256, previous['whisper']['path'])
    if {entry['path'] for entry in selected['libraries']} != {entry['path'] for entry in previous['libraries']}:
        raise RuntimeError('Artifact refresh may not change dylib identities')
    installed = ROOT / 'models' / WHISPER_NAME
    if installed.exists() or installed.is_symlink():
        artifact(installed, WHISPER_SHA)
    updated = {**previous, **selected}
    evidence = ROOT / ('record-review-artifact-refresh-' + uuid.uuid4().hex + '.json')
    require_stopped()
    artifact(MARKER, args.previous_manifest_sha256)
    write_json(evidence, {'previousManifestSha256': args.previous_manifest_sha256,
                         'previous': previous, 'updated': updated,
                         'createdUtc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()),
                         'testDataPreserved': True})
    write_json(MARKER, updated)
    print(json.dumps({'refreshed': str(MARKER), 'manifestSha256': digest(MARKER),
                      'binary': updated['binary'], 'evidence': str(evidence), 'launched': False}))


def request(url, data=None, token=None):
    headers = {'Content-Type': 'application/json'}
    if token:
        headers['Authorization'] = 'Bearer ' + token
    req = urllib.request.Request(url, data=None if data is None else json.dumps(data).encode(), headers=headers)
    # No inherited proxy routing, and no response/token output on failure.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(req, timeout=15) as response:
            return json.load(response)
    except urllib.error.HTTPError as e:
        raise RuntimeError(f'Local API returned HTTP {e.code} for {req.selector}') from None


def token():
    path = ROOT / 'secrets/daemon_token.json'
    if path.resolve() != path or path.stat().st_mode & 0o077:
        raise RuntimeError('Isolated bearer file is not private/canonical')
    value = json.loads(path.read_text())['token']
    if not re.fullmatch('[0-9a-f]{64}', value):
        raise RuntimeError('Invalid isolated bearer format')
    return value


def provision(value):
    result = request(URL + '/api/dictation/provision', {'bundled_path': value['whisper']['path']}, token())
    if result != {'status': 'ready', 'model': 'base'}:
        raise RuntimeError('Whisper API did not report the expected ready model')
    artifact(ROOT / 'models' / WHISPER_NAME, WHISPER_SHA)
    actual_config = request(URL + '/config', token=token()).get('config', {})
    if actual_config.get('LOCAL_WHISPER_MODEL') != 'base' or actual_config.get('meeting_writeup_local_only') is not True:
        raise RuntimeError('Local Whisper/privacy config verification failed')
    return {'whisper_status': 'ready', 'whisper_sha256': WHISPER_SHA}


def probe():
    manifest()
    # Projects endpoint proves this bearer authenticates against the selected daemon.
    projects = request(URL + '/api/projects', token=token())
    artifact(ROOT / 'models' / WHISPER_NAME, WHISPER_SHA)
    return {'url': URL, 'authenticated_projects_response': isinstance(projects, (list, dict)),
            'whisper_sha256': WHISPER_SHA, 'token_logged': False}


def run(args):
    value = manifest()
    ontology = ROOT / 'brain/ontology.toml'
    if ontology.resolve() != ontology or not ontology.is_file() or ontology.read_bytes() != EMPTY_ONTOLOGY:
        raise RuntimeError('Run setup-brain before startup; empty review ontology is required')
    port_free()
    if ACTIVE.exists():
        raise RuntimeError('Previous active marker exists; inspect prior owned runner before removing it')
    if shutil.disk_usage(ROOT).free < 25 * 1024**3:
        raise RuntimeError('25 GiB startup disk minimum not met')
    tags = request('http://127.0.0.1:11434/api/tags')
    if MODEL not in {m.get('name') for m in tags.get('models', [])}:
        raise RuntimeError('Installed local qwen2.5:7b is required; nothing will be pulled')
    command = [value['binary']['path'], 'agent', '--host', '127.0.0.1', '--port', str(PORT)]
    stamp = time.strftime('%Y%m%d-%H%M%S', time.gmtime())
    log_path = ROOT / 'logs' / f'record-review-daemon-{stamp}.log'
    log_path.parent.mkdir(mode=0o700, exist_ok=True)
    evidence_path = ROOT / f'record-review-run-{stamp}.json'
    run_id = uuid.uuid4().hex
    evidence = {'run_id': run_id, 'command': command, 'root': str(ROOT), 'url': URL,
                'binary': value['binary'], 'libraries': value['libraries'], 'log': str(log_path),
                'started_utc': stamp, 'ready': False, 'token_logged': False}
    stopped = False
    def stop_signal(_signum, _frame):
        nonlocal stopped
        stopped = True
    signal.signal(signal.SIGTERM, stop_signal)
    signal.signal(signal.SIGINT, stop_signal)
    port_free()
    with log_path.open('xb') as log:
        os.chmod(log_path, 0o600)
        proc = subprocess.Popen(command, cwd=ROOT, env=child_env(), stdout=log,
                                stderr=subprocess.STDOUT, start_new_session=True)
        write_json(ACTIVE, {'owner': value['owner'], 'run_id': run_id, 'pid': proc.pid})
        evidence['pid'] = proc.pid
        started = time.monotonic()
        try:
            while proc.poll() is None and not stopped and time.monotonic() - started < args.max_seconds:
                if STOP.exists():
                    stop = json.loads(STOP.read_text())
                    if stop.get('run_id') == run_id and stop.get('owner') == value['owner']:
                        stopped = True
                        break
                if shutil.disk_usage(ROOT).free < 20 * 1024**3:
                    raise RuntimeError('20 GiB continuous disk floor reached')
                if not evidence['ready']:
                    try:
                        request(URL + '/api/projects', token=token())
                    except (OSError, ValueError, KeyError, RuntimeError):
                        if time.monotonic() - started > 120:
                            raise RuntimeError('Daemon readiness deadline exceeded; inspect isolated log') from None
                    else:
                        evidence.update(provision(value))
                        evidence['ready'] = True
                        write_json(evidence_path, evidence)
                        print(json.dumps({'ready': True, 'evidence': str(evidence_path), 'url': URL}), flush=True)
                time.sleep(1)
            if proc.poll() is not None and not stopped:
                raise RuntimeError(f'Owned daemon exited with code {proc.returncode}')
        finally:
            # Only the exact process group created above; never listener lookup/global pkill.
            stop_owned_group(proc)
            evidence.update({'exit_code': proc.returncode, 'finished_utc': time.strftime('%Y%m%d-%H%M%S', time.gmtime())})
            write_json(evidence_path, evidence)
            ACTIVE.unlink(missing_ok=True)
            STOP.unlink(missing_ok=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    parser.add_argument('--profile', choices=PROFILES, default='debug', help='Build profile directory under the target (default debug)')
    sub.add_parser('sign-libs', help='Explicitly ad-hoc sign only owned sherpa/onnx dylibs of the selected --profile')
    prep = sub.add_parser('prepare', help='Initialize empty isolated root after build and signing')
    prep.add_argument('--binary', required=True)
    prep.add_argument('--sha256', required=True)
    prep.add_argument('--whisper', required=True)
    refresh = sub.add_parser('refresh-artifacts', help='Explicitly adopt reviewed rebuilt artifacts, preserving all test data')
    refresh.add_argument('--binary', required=True)
    refresh.add_argument('--sha256', required=True)
    refresh.add_argument('--previous-manifest-sha256', required=True)
    sub.add_parser('setup-brain', help='Create only the empty ontology in an existing stopped owned runtime')
    launch = sub.add_parser('run', help='Launch and supervise only the prepared isolated daemon')
    launch.add_argument('--max-seconds', type=int, default=7200)
    sub.add_parser('probe', help='Read authenticated isolated status and verify copied model hash')
    sub.add_parser('stop', help='Request this supervisor stop its owned group; never signals by stored PID')
    sub.add_parser('dry-run', help='Show public target/config/env keys; no filesystem mutation or launch')
    args = parser.parse_args()
    set_profile(args.profile)
    if args.action == 'sign-libs': sign_libraries()
    elif args.action == 'prepare': prepare(args)
    elif args.action == 'refresh-artifacts': refresh_artifacts(args)
    elif args.action == 'setup-brain': setup_brain()
    elif args.action == 'run':
        if not 1 <= args.max_seconds <= 43200: raise RuntimeError('max-seconds must be 1..43200')
        run(args)
    elif args.action == 'probe': print(json.dumps(probe()))
    elif args.action == 'stop':
        value = manifest()
        active = json.loads(ACTIVE.read_text())
        if active.get('owner') != value['owner']: raise RuntimeError('Active owner mismatch')
        write_json(STOP, {'owner': value['owner'], 'run_id': active['run_id']})
        print('Stop requested for owned supervisor; wait for its completion evidence.')
    elif args.action == 'dry-run':
        print(json.dumps({'root': str(ROOT), 'url': URL, 'profile': PROFILE, 'debug_libraries': str(DEBUG),
                          'config': config(), 'child_env_keys': sorted(child_env()),
                          'whisper_sha256': WHISPER_SHA, 'launched': False}, indent=2))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        # Exception text never includes HTTP response bodies, headers or bearer values.
        print(f'Isolated review stopped: {error}', file=sys.stderr)
        sys.exit(1)
