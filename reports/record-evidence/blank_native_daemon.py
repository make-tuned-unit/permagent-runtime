#!/usr/bin/env python3
"""Opt-in, guarded blank-profile daemon for the isolated native review app.

Run only after reviewing the newly built daemon hash. This script does not
seed config, credentials, models, or fixtures. It supervises its own process
group on loopback port 3002 and exits within --max-seconds (at most one hour).
Stop the foreground supervisor with Ctrl-C, or SIGTERM its printed supervisor
PID. Never signal a process found by port or process name.
"""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import stat
import subprocess
import sys
import time
import urllib.error
import urllib.request

import isolated_daemon as isolated


PORT = 3002
OLLAMA_DENIED_PORT = 43009
DISK_PATHS = (Path('/System/Volumes/Data'), Path('/Volumes/PermagentBuilds'), Path('/Volumes/UNTITLED'))
START_FREE = 25 * 1024**3
RUN_FREE = 20 * 1024**3
# Internal volume uses the user-approved 2026-10-09 thresholds (12 start / 8 floor),
# the same numbers as external_build.py; every external volume keeps 25/20.
INTERNAL_VOLUME = Path('/System/Volumes/Data')
INTERNAL_START_FREE = 12 * 1024**3
INTERNAL_RUN_FREE = 8 * 1024**3
READINESS_SECONDS = 120


def check_disks(floor: int) -> dict[str, float]:
    """Fail closed if any required volume is unavailable or below the floor."""
    free = {}
    for path in (*DISK_PATHS, isolated.TARGET):
        if not path.is_dir():
            raise RuntimeError(f'Required volume is unavailable: {path}')
        available = shutil.disk_usage(path).free
        required = floor
        if path == INTERNAL_VOLUME and floor == START_FREE:
            required = INTERNAL_START_FREE
        elif path == INTERNAL_VOLUME and floor == RUN_FREE:
            required = INTERNAL_RUN_FREE
        if available < required:
            raise RuntimeError(f'Disk safety floor reached on {path}')
        free[str(path)] = round(available / 1024**3, 2)
    return free


def require_free_port(port: int) -> None:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            sock.bind(('127.0.0.1', port))
        except OSError as error:
            raise RuntimeError(f'Loopback port {port} is occupied; no listener will be stopped') from error


def reviewed_artifacts(expected_sha: str) -> dict:
    if not re.fullmatch(r'[0-9a-f]{64}', expected_sha):
        raise RuntimeError('A lowercase SHA-256 digest is required')
    if not isolated.TARGET.is_absolute() or isolated.TARGET.resolve() != isolated.TARGET:
        raise RuntimeError('Build target must be an existing canonical absolute directory')
    binary_path = isolated.DEBUG / 'permagentd'
    binary = isolated.artifact(binary_path, expected_sha)
    if Path(binary['path']) != binary_path or not os.access(binary_path, os.X_OK):
        raise RuntimeError('Only the executable in the owned debug target may run')
    libraries = isolated.libraries()
    for path in libraries:
        subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(path)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    # Config::default and its init path discover files beside the executable or
    # in a workspace ancestor. A blank-profile run must not silently import one.
    for ancestor in binary_path.parents:
        if any((ancestor / name).exists() for name in ('defaults.yaml', 'init-config.yaml')):
            raise RuntimeError('Bundled defaults or init config found; review before blank-profile run')
    return {'binary': binary, 'libraries': [isolated.artifact(path) for path in libraries]}


def fresh_root() -> tuple[Path, Path]:
    root = isolated.ROOT
    if root != isolated.TARGET / 'record-qa-runtime' or root.name != 'record-qa-runtime':
        raise RuntimeError('Unexpected native review root')
    if root == Path.home() or '.permagent' in root.parts:
        raise RuntimeError('Production profile is not an allowed runtime root')
    root.mkdir(mode=0o700, exist_ok=True)
    isolated.root_check()
    isolated.fresh_root_check()  # only a private logs/ui-heartbeat may preexist
    if root.stat().st_mode & 0o077:
        raise RuntimeError('Native review root must be private')
    home = root / 'home'
    home.mkdir(mode=0o700)
    if list(home.iterdir()):
        raise RuntimeError('Isolated HOME must begin empty')
    logs = root / 'logs'
    logs.mkdir(mode=0o700, exist_ok=True)
    if logs.is_symlink() or logs.stat().st_mode & 0o077:
        raise RuntimeError('Log directory must be private and canonical')
    return root, home


def child_env(root: Path, home: Path, use_existing_local_ollama: bool = False) -> dict[str, str]:
    # Construct from nothing: never inherit provider keys, proxies, user HOME,
    # model choices, Tailscale state, DYLD overrides, or Keychain access.
    # An explicit positive-path review may reuse the already-running local
    # service. This never starts Ollama, downloads a model, or selects a route.
    ollama_url = 'http://127.0.0.1:11434' if use_existing_local_ollama else f'http://127.0.0.1:{OLLAMA_DENIED_PORT}'
    return {
        'PATH': '/usr/bin:/bin:/usr/sbin:/sbin',
        'LANG': 'en_US.UTF-8',
        'HOME': str(home),
        'PERMAGENT_PATH_ROOT': str(root),
        'PERMAGENT_CONFIG': str(root / 'config.yaml'),
        'PERMAGENT_SPECTRAL_DB': str(root / 'spectral/permagent.db'),
        'PERMAGENT_DISABLE_KEYRING': '1',
        'DYLD_LIBRARY_PATH': str(isolated.DEBUG),
        'GOOSE_TELEMETRY_ENABLED': 'false',
        'COUNCIL_ENABLED': 'false',
        'CONCIERGE_ENABLED': 'false',
        'STRIX_ENABLED': 'false',
        'STEWARD_SCAN_ENABLED': 'false',
        'POLYBOT_ENABLED': 'false',
        'TUNNEL_AUTO_START': 'false',
        'PERMAGENT_MESH_TRUSTED': '0',
        'PERMAGENT_MESH_ENGINE': '0',
        'OLLAMA_HOST': ollama_url,
        'PERMAGENT_OLLAMA_HOST': ollama_url,
    }


def private_token(path: Path) -> str:
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
            raise RuntimeError('Daemon bearer file is not a private regular file')
        with os.fdopen(fd, 'r') as handle:
            fd = -1
            value = json.load(handle)['token']
    finally:
        if fd >= 0:
            os.close(fd)
    if not isinstance(value, str) or not re.fullmatch(r'[0-9a-f]{64}', value):
        raise RuntimeError('Unexpected daemon bearer format')
    return value


def authenticated_projects(token: str) -> None:
    request = urllib.request.Request(
        f'http://127.0.0.1:{PORT}/api/projects',
        headers={'Authorization': 'Bearer ' + token},
    )
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(request, timeout=5) as response:
        if response.status != 200:
            raise RuntimeError('Local projects readiness request did not succeed')
        value = json.load(response)
    if not isinstance(value, (list, dict)):
        raise RuntimeError('Unexpected projects readiness response')
    # Do not persist project names, response bodies, or the bearer.


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true', help='Required to launch the verified daemon')
    parser.add_argument('--sha256', required=True, help='Reviewed SHA-256 of debug/permagentd')
    parser.add_argument('--max-seconds', type=int, default=1800, help='Supervisor lifetime, 1..3600 seconds')
    parser.add_argument('--use-existing-local-ollama', action='store_true',
                        help='Explicit positive-path review: use an existing localhost:11434 service without starting it or selecting a model')
    args = parser.parse_args()
    if not args.execute:
        raise RuntimeError('Refusing launch without --execute')
    if not 1 <= args.max_seconds <= 3600:
        raise RuntimeError('--max-seconds must be between 1 and 3600')
    initial_free = check_disks(START_FREE)
    selected = reviewed_artifacts(args.sha256)
    require_free_port(PORT)
    require_free_port(OLLAMA_DENIED_PORT)
    root, home = fresh_root()
    check_disks(START_FREE)
    require_free_port(PORT)
    require_free_port(OLLAMA_DENIED_PORT)

    stamp = time.strftime('%Y%m%d-%H%M%S', time.gmtime())
    evidence_path = root / f'blank-native-daemon-{stamp}.json'
    log_path = root / 'logs' / f'blank-native-supervisor-{stamp}.log'
    command = [selected['binary']['path'], 'agent', '--host', '127.0.0.1', '--port', str(PORT)]
    report = {
        'schema': 1, 'kind': 'blank-native-daemon', 'root': str(root),
        'url': f'http://127.0.0.1:{PORT}', 'binary': selected['binary'],
        'libraries': selected['libraries'], 'log': str(log_path),
        'startedUtc': stamp, 'ready': False, 'tokenLogged': False,
        'inheritedEnvironment': False, 'defaultConfigSeeded': False,
        'runtimeOverrides': sorted(child_env(root, home)),
        'modelRuntime': 'existing-local-ollama' if args.use_existing_local_ollama else 'unavailable-fixture',
        'initialFreeGiB': initial_free,
        'maxSeconds': args.max_seconds,
    }
    stopped = False
    def stop_signal(_signum, _frame):
        nonlocal stopped
        stopped = True
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, stop_signal)
    log_fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(log_fd, 'w') as log:
        log.write('Blank native daemon supervisor starting; daemon output discarded.\n')
        log.flush()
        # Discard daemon stdout/stderr so a future daemon log change cannot put
        # bearer values or private user content in a persisted review log.
        proc = subprocess.Popen(command, cwd=root, env=child_env(root, home, args.use_existing_local_ollama),
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                                start_new_session=True)
        started = time.monotonic()
        report['daemonPid'] = proc.pid
        report['supervisorPid'] = os.getpid()
        isolated.write_json(evidence_path, report)
        finish_reason = 'deadline'
        try:
            while True:
                check_disks(RUN_FREE)
                elapsed = time.monotonic() - started
                if stopped:
                    finish_reason = 'signal'
                    break
                if proc.poll() is not None:
                    finish_reason = 'daemon-exit'
                    raise RuntimeError('Owned daemon exited')
                if elapsed >= args.max_seconds:
                    if not report['ready']:
                        finish_reason = 'readiness-timeout'
                        raise RuntimeError('Daemon did not become ready within the supervisor lifetime')
                    break
                if not report['ready']:
                    if elapsed >= READINESS_SECONDS:
                        finish_reason = 'readiness-timeout'
                        raise RuntimeError('Authenticated readiness exceeded 120 seconds')
                    try:
                        token = private_token(root / 'secrets/daemon_token.json')
                    except (FileNotFoundError, json.JSONDecodeError):
                        pass
                    else:
                        try:
                            authenticated_projects(token)
                        except (urllib.error.URLError, TimeoutError, ValueError, OSError):
                            pass  # Startup may not yet be accepting authenticated requests.
                        else:
                            report['ready'] = True
                            report['readySeconds'] = round(elapsed, 2)
                            isolated.write_json(evidence_path, report)
                            log.write('Authenticated /api/projects readiness succeeded.\n')
                            log.flush()
                            print(json.dumps({'ready': True, 'url': report['url'],
                                              'supervisorPid': os.getpid(),
                                              'evidence': str(evidence_path)}), flush=True)
                time.sleep(0.5)
        except Exception as error:
            report['errorType'] = type(error).__name__
            report['error'] = str(error)[:160]
            finish_reason = finish_reason if finish_reason != 'deadline' else 'guard-error'
            raise
        finally:
            # Only our own start_new_session child group; never a listener PID.
            isolated.stop_owned_group(proc)
            report.update(finishedUtc=time.strftime('%Y%m%d-%H%M%S', time.gmtime()),
                          finishReason=finish_reason, daemonExitCode=proc.returncode)
            try:
                report['finalFreeGiB'] = check_disks(0)
            except RuntimeError:
                report['finalDiskCheckUnavailable'] = True
            isolated.write_json(evidence_path, report)
            log.write(f'Supervisor stopped owned daemon group: {finish_reason}.\n')
            log.flush()
    return 0 if report['ready'] else 1


if __name__ == '__main__':
    try:
        sys.exit(main())
    except Exception as error:
        # The runner never includes response bodies, authorization headers or
        # token values in exceptions, logs, metadata, or command arguments.
        print(f'Blank native daemon stopped: {type(error).__name__}: {error}', file=sys.stderr)
        sys.exit(1)
