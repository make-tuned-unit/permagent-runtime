#!/usr/bin/env python3
"""Build only the isolated native review shell; never stage, launch or install.

Run after the coordinator releases the heavy-build slot:
  python3 reports/world-production/run_guarded_native.py r29

Per-volume guards (internal 12/8, TARGET's external volume 25/20 GiB start/floor,
user-approved 2026-10-09), 30 minute deadline. Signals
only the new process group owned by this runner; never global cargo/rustc PIDs.
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
import subprocess
import sys
import time

from native_daemon_preservation import DaemonPreservation

LANE = Path(__file__).resolve().parents[2]
TARGET = Path(os.environ.get('PERMAGENT_BUILD_TARGET', str(LANE.parent / '.shared-target/world-production-20261006')))
REPORTS = LANE / 'reports/world-production'
DATA_VOLUME = Path('/System/Volumes/Data')
# Per-volume guards (GiB), user-approved 2026-10-09 and mirrored from
# external_build.py (which imports this module, so it cannot be imported here).
# The internal volume carries only swap and small fixtures: 12 start / 8 floor.
# The volume holding TARGET (the external image) keeps 25 start / 20 floor.
INTERNAL_START_GIB = 12
INTERNAL_FLOOR_GIB = 8
EXTERNAL_START_GIB = 25
EXTERNAL_FLOOR_GIB = 20
START_GIB = INTERNAL_START_GIB  # internal value; build-guard.sh checks the internal volume
FLOOR_GIB = INTERNAL_FLOOR_GIB
DEADLINE_SECONDS = 30 * 60
GIB = 1024 ** 3


def digest(path: Path) -> str:
    value = hashlib.sha256()
    with path.open('rb') as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b''):
            value.update(chunk)
    return value.hexdigest()


def free_gib() -> float:
    return shutil.disk_usage(DATA_VOLUME).free / GIB


def mount_of(path: Path) -> Path:
    path = Path(path)
    while not path.exists() and path != path.parent:
        path = path.parent
    return Path(os.path.realpath(path)) if os.path.ismount(path) else _mount_walk(path)


def _mount_walk(path: Path) -> Path:
    path = Path(os.path.realpath(path))
    while not os.path.ismount(path) and path != path.parent:
        path = path.parent
    return path


def guard_volumes() -> dict:
    """name -> (start GiB, floor GiB); internal always, TARGET volume if external."""
    volumes = {str(DATA_VOLUME): (INTERNAL_START_GIB, INTERNAL_FLOOR_GIB)}
    mount = mount_of(TARGET)
    if os.path.realpath(mount) != os.path.realpath(DATA_VOLUME) and str(mount) != '/':
        volumes[str(mount)] = (EXTERNAL_START_GIB, EXTERNAL_FLOOR_GIB)
    return volumes


def volume_free() -> dict:
    free = {}
    for name in guard_volumes():
        free[name] = free_gib() if name == str(DATA_VOLUME) else shutil.disk_usage(name).free / GIB
    return free


def startup_shortfall(free: dict):
    short = {n: (round(v, 2), guard_volumes()[n][0]) for n, v in free.items() if v < guard_volumes()[n][0]}
    return f'Refusing build below startup minimum (GiB free, required): {short}' if short else None


def floor_breach(free: dict):
    low = {n: (round(v, 2), guard_volumes()[n][1]) for n, v in free.items() if v < guard_volumes()[n][1]}
    return f'Free space fell below floor (GiB free, floor): {low}' if low else None


def group_alive(pgid: int) -> bool:
    try:
        os.killpg(pgid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        # Some macOS execution contexts report EPERM after the owned group
        # exits. Confirm absence through the process table; never treat an
        # inaccessible but present group as stopped.
        rows = subprocess.run(['ps', '-axo', 'pgid=,stat='], check=True,
                              capture_output=True, text=True).stdout.splitlines()
        active = [row.split() for row in rows if row.strip()]
        return any(group == str(pgid) and not state.startswith('Z')
                   for group, state in active)


def wait_owned_group(proc: subprocess.Popen, timeout: float = 10) -> bool:
    """Reap the owned leader and wait a bounded time; True means no live members."""
    until = time.monotonic() + timeout
    while True:
        proc.poll()
        if not group_alive(proc.pid):
            return True
        if time.monotonic() >= until:
            return False
        time.sleep(.2)


def stop_owned_group(proc: subprocess.Popen) -> None:
    # start_new_session=True makes the child PID its session/group identifier.
    # Its group cannot be reused while any descendant still belongs to it.
    pgid = proc.pid
    try:
        os.killpg(pgid, signal.SIGTERM)
    except ProcessLookupError:
        proc.wait()
        return
    except PermissionError:
        # A denied signal is not proof of exit. Teardown may be in progress;
        # retain the real signal failure if the group is still live at deadline.
        if wait_owned_group(proc):
            proc.wait()
            return
        raise
    if not wait_owned_group(proc):
        try:
            os.killpg(pgid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except PermissionError:
            if wait_owned_group(proc):
                proc.wait()
                return
            raise
        if not wait_owned_group(proc):
            raise RuntimeError(f'Owned process group {pgid} is still live after SIGKILL')
    proc.wait()


def preflight(record_review: bool = False) -> None:
    if LANE.name != 'world-production-20261006':
        raise RuntimeError(f'Unexpected lane: {LANE}')
    for relative in (
        'scripts/build-guard.sh',
        'ui/command-center/node_modules/@tauri-apps/cli/tauri.js',
        'ui/desktop/src-tauri/Cargo.lock',
        'ui/desktop/src-tauri/Entitlements.plist',
        'ui/desktop/src-tauri/audiocap/permagent-audiocap',
    ):
        if not (LANE / relative).is_file():
            raise RuntimeError(f'Missing build prerequisite: {relative}')
    for subdir in ('mcp-runtime', 'whisper'):
        root = LANE / 'ui/desktop/src-tauri/resources' / subdir
        if not root.is_dir() or not any(p.is_file() for p in root.rglob('*')):
            raise RuntimeError(f'Missing resource inputs: {root}')
    ledger = json.loads((REPORTS / 'native-baseline-copy-ledger.json').read_text())
    for row in ledger:
        source = (LANE / row['destination']).resolve()
        if not source.is_relative_to(LANE):
            raise RuntimeError('Prerequisite ledger path leaves lane')
        if not source.is_file() or digest(source) != row['sha256']:
            raise RuntimeError(f'Prerequisite differs from preserved baseline ledger: {source}')
    config_name = 'native-record-review-config.json' if record_review else 'native-review-config.json'
    config = json.loads((REPORTS / config_name).read_text())
    expected_id = 'ai.permagent.record-review-20261007' if record_review else 'ai.permagent.world-review-20261006'
    if config.get('identifier') != expected_id:
        raise RuntimeError('Review identifier mismatch')
    if config.get('plugins', {}).get('updater', {}).get('endpoints') != []:
        raise RuntimeError('Review updater endpoints must remain disabled')
    if config.get('bundle', {}).get('createUpdaterArtifacts') is not False:
        raise RuntimeError('Review updater artifacts must remain disabled')
    if 'beforeBuildCommand' in config.get('build', {}):
        raise RuntimeError('Review override must inherit the native frontend build')
    if record_review:
        review_root = TARGET / 'record-qa-runtime'
        review_root.mkdir(mode=0o700, parents=True, exist_ok=True)
        if review_root.resolve() != review_root or not review_root.is_dir():
            raise RuntimeError('Record review root must be a canonical isolated directory')
        csp = config.get('app', {}).get('security', {}).get('csp', '')
        if ':3001' in csp or 'http://127.0.0.1:3002' not in csp or 'ws://127.0.0.1:3002' not in csp:
            raise RuntimeError('Record review CSP must target 3002 and exclude 3001')
    shortfall = startup_shortfall(volume_free())
    if shortfall:
        raise RuntimeError(shortfall)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('revision', help='New evidence tag, e.g. r29; existing logs are never overwritten')
    parser.add_argument('--record-review', action='store_true', help='Build the isolated Record app for externally managed localhost3002')
    args = parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]{0,63}', args.revision):
        parser.error('Revision must contain only letters, digits, underscores or hyphens')
    preflight(args.record_review)
    config_name = 'native-record-review-config.json' if args.record_review else 'native-review-config.json'
    env = os.environ.copy()
    # Explicit mode owns these values. Inherited review/Vite environment must
    # never silently redirect a normal World review build.
    for name in ('PERMAGENT_REVIEW_ROOT', 'PERMAGENT_REVIEW_PORT', 'VITE_DAEMON_URL', 'VITE_API_BASE_URL'):
        env.pop(name, None)
    if args.record_review:
        env.update(PERMAGENT_REVIEW_ROOT=str(TARGET / 'record-qa-runtime'),
                   PERMAGENT_REVIEW_PORT='3002', VITE_DAEMON_URL='http://127.0.0.1:3002')
    env.update({
        'PATH': '/Users/j/Documents/dev/permagent-runtime/bin:' + env.get('PATH', ''),
        'WORKTREES_ROOT': str(LANE.parent),
        'CARGO_TARGET_DIR': str(TARGET),
        'CARGO_NET_OFFLINE': 'true',
        'CARGO_BUILD_JOBS': '1',
        'CARGO_PROFILE_DEV_DEBUG': '0',
        'CARGO_INCREMENTAL': '0',
    })
    # Prevent inherited inline config from changing review identity/build hooks.
    env.pop('TAURI_CONFIG', None)
    node = shutil.which('node', path=env['PATH'])
    if not node:
        raise RuntimeError('node is unavailable on the configured PATH')
    command = [
        str(LANE / 'scripts/build-guard.sh'), '--min-free-gb', str(START_GIB), '--',
        node, '../command-center/node_modules/@tauri-apps/cli/tauri.js',
        'build', '--debug', '--no-bundle', '--config', str(REPORTS / config_name),
        '--', '--locked', '-j', '1',
    ]
    stamp = time.strftime('%Y%m%d-%H%M%S')
    mode_prefix = 'native-record-build' if args.record_review else 'native-build'
    log_path = REPORTS / f'{mode_prefix}-{args.revision}-{stamp}.log'
    summary_path = log_path.with_suffix('.json')
    started = time.monotonic()
    proc = None
    reason = None
    cleanup_attempted = False
    cleanup_error = None
    rc = 1
    minimum_free = free_gib()
    minimum_by_volume = dict(volume_free())
    samples = []
    preservation = DaemonPreservation(TARGET)
    preservation_error = None
    owned_group_stopped = True

    def interrupted(signum, _frame):
        raise KeyboardInterrupt(f'Signal {signum}')

    old_handlers = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        with log_path.open('x', buffering=1) as log:
            log.write('Command: ' + json.dumps(command) + '\n')
            print(f'Build log: {log_path}', flush=True)
            preservation.prepare()
            log.write('Daemon preservation: ' + json.dumps(preservation.evidence) + '\n')
            proc = subprocess.Popen(command, cwd=LANE / 'ui/desktop', env=env,
                                    stdin=subprocess.DEVNULL, stdout=log,
                                    stderr=subprocess.STDOUT, start_new_session=True)
            while True:
                by_volume = volume_free()
                current_free = by_volume[str(DATA_VOLUME)]
                minimum_free = min(minimum_free, current_free)
                for name, value in by_volume.items():
                    minimum_by_volume[name] = min(minimum_by_volume.get(name, value), value)
                elapsed = time.monotonic() - started
                samples.append({'elapsed_s': round(elapsed, 2), 'free_gib': round(current_free, 3),
                                'volumes_gib': {n: round(v, 3) for n, v in by_volume.items()}})
                breach = floor_breach(by_volume)
                if breach:
                    reason = breach
                    break
                if elapsed >= DEADLINE_SECONDS:
                    reason = '30 minute native build deadline exceeded'
                    break
                status = proc.poll()
                if status is not None:
                    rc = status
                    break
                time.sleep(2)
            if reason:
                log.write('\nABORT: ' + reason + '\n')
                print(reason, file=sys.stderr, flush=True)
                cleanup_attempted = True
                stop_owned_group(proc)
                rc = 124
    except BaseException as error:
        detail = f'{type(error).__name__}: {error}'
        if cleanup_attempted:
            cleanup_error = detail
        else:
            reason = reason or detail
            if proc is not None:
                cleanup_attempted = True
                try:
                    stop_owned_group(proc)
                except BaseException as cleanup:
                    cleanup_error = f'{type(cleanup).__name__}: {cleanup}'
        rc = 130 if isinstance(error, KeyboardInterrupt) else 1
        print(reason, file=sys.stderr, flush=True)
    finally:
        # Tauri copies externalBin into target/debug even with --no-bundle.
        # Restore only after the complete owned group has stopped, on success,
        # compilation failure, launch failure, interruption and disk/deadline exit.
        try:
            if proc is not None:
                owned_group_stopped = False
                if cleanup_error is None:
                    owned_group_stopped = wait_owned_group(proc)
                    if not owned_group_stopped:
                        cleanup_attempted = True
                        stop_owned_group(proc)
                        owned_group_stopped = True
            preservation.restore(group_stopped=owned_group_stopped)
        except BaseException as error:
            preservation_error = f'{type(error).__name__}: {error}'
            rc = 1
            reason = reason or 'Daemon artifact restoration failed; preserved backup requires review'
            print(preservation_error, file=sys.stderr, flush=True)
        for sig, previous in old_handlers.items():
            signal.signal(sig, previous)
        summary = {
            'revision': args.revision, 'record_review': args.record_review,
            'review_config': str(REPORTS / config_name),
            'compiled_review_root': env.get('PERMAGENT_REVIEW_ROOT'),
            'compiled_review_port': env.get('PERMAGENT_REVIEW_PORT'),
            'frontend_api_base': env.get('VITE_DAEMON_URL'),
            'lane': str(LANE), 'target': str(TARGET),
            'command': command, 'log': str(log_path), 'exit_code': rc,
            'reason': reason, 'cleanup_error': cleanup_error,
            'owned_group_stopped': owned_group_stopped,
            'daemon_artifact_preservation': preservation.evidence,
            'daemon_preservation_error': preservation_error,
            'elapsed_s': round(time.monotonic() - started, 2),
            'minimum_free_gib': round(minimum_free, 3), 'samples': samples,
            'minimum_free_by_volume_gib': {n: round(v, 3) for n, v in minimum_by_volume.items()},
            'startup_minimum_gib': START_GIB, 'continuous_floor_gib': FLOOR_GIB,
            'volume_guards_gib': {n: {'start': a, 'floor': b} for n, (a, b) in guard_volumes().items()},
            'timeout_s': DEADLINE_SECONDS, 'staged': False, 'launched': False,
        }
        binary = TARGET / 'debug/permagent-app'
        if rc == 0 and binary.is_file():
            summary['native_binary_sha256'] = digest(binary)
        with summary_path.open('x') as handle:
            json.dump(summary, handle, indent=2)
            handle.write('\n')
        print(json.dumps({k: summary[k] for k in ('exit_code', 'reason', 'elapsed_s', 'log')}), flush=True)
    return rc if rc >= 0 else 128 - rc


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (RuntimeError, OSError, ValueError) as error:
        print(f'Native build refused: {error}', file=sys.stderr)
        sys.exit(1)
