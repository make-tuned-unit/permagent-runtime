#!/usr/bin/python3
"""Run a build on the verified external APFS image; never format or auto-mount.

Usage: /usr/bin/python3 reports/world-production/external_build.py -- COMMAND ...
For example: ... -- /usr/bin/python3 reports/world-production/run_record_backend_tests.py --build-daemon
Guards: 25 GiB startup / 20 GiB stop on the external image and its backing drive
(where Cargo output, staged apps and build temp live); 12 GiB startup / 8 GiB stop
on the internal data volume, which now only carries swap and small test fixtures
(user-approved 2026-10-09).
"""
import fcntl
import json
import os
from pathlib import Path
import plistlib
import shutil
import signal
import subprocess
import sys
import time

from run_guarded_native import stop_owned_group

LANE = Path(__file__).resolve().parents[2]
MOUNT = Path('/Volumes/PermagentBuilds')
DRIVE = Path('/Volumes/UNTITLED')
IMAGE = DRIVE / 'CodexBuilds/PermagentBuilds.sparsebundle'
ROOT = MOUNT / 'permagent'
TARGET = ROOT / 'targets' / LANE.name
TEMP = ROOT / 'tmp' / LANE.name
VOLUMES = {
    MOUNT: '85630790-0A49-4D4E-B683-888258B80A99',
    DRIVE: '53500C9A-452C-35D6-9F4E-1240FDD8DB9D',
}
INTERNAL = '/System/Volumes/Data'
# Per-volume guards (GiB), user-approved 2026-10-09. The external image and its
# backing drive take the Cargo target, staged apps and build temp, so they keep
# the original 25/20. The internal volume now only carries swap and small test
# fixtures; a benchmark peer makes it swing by several GiB, so 12/8 protects
# swap headroom without stalling builds that write nothing large there.
START_GIB = {str(MOUNT): 25, str(DRIVE): 25, INTERNAL: 12}
FLOOR_GIB = {str(MOUNT): 20, str(DRIVE): 20, INTERNAL: 8}


def check_mounts():
    for path, uuid in VOLUMES.items():
        if not path.is_mount():
            raise RuntimeError(f'Expected volume is not mounted: {path}')
        info = plistlib.loads(subprocess.check_output(['/usr/sbin/diskutil', 'info', '-plist', str(path)]))
        if info.get('VolumeUUID') != uuid or info.get('MountPoint') != str(path):
            raise RuntimeError(f'Wrong volume at {path}')
    images = plistlib.loads(subprocess.check_output(['/usr/bin/hdiutil', 'info', '-plist']))['images']
    if not any(item.get('image-path') == str(IMAGE) and any(
        entity.get('mount-point') == str(MOUNT) for entity in item.get('system-entities', [])
    ) for item in images):
        raise RuntimeError('Build volume is not backed by the expected external image')


def space():
    if not all(path.is_mount() for path in VOLUMES):
        raise RuntimeError('External build drive disconnected')
    return {str(path): shutil.disk_usage(path).free / 1024**3
            for path in (*VOLUMES, Path('/System/Volumes/Data'))}


def main():
    command = sys.argv[1:]
    if command[:1] == ['--']:
        command = command[1:]
    if not command:
        raise SystemExit(__doc__)
    check_mounts()
    initial = space()
    short = {path: free for path, free in initial.items() if free < START_GIB[path]}
    if short:
        raise RuntimeError('startup minimum not met (GiB free vs required): '
                           + str({k: (round(v, 1), START_GIB[k]) for k, v in short.items()}))
    for path in (TARGET, TEMP, ROOT / 'logs'):
        path.mkdir(parents=True, exist_ok=True)
        if path.resolve() != path or path.stat().st_dev != MOUNT.stat().st_dev:
            raise RuntimeError(f'Build path is redirected off the external volume: {path}')
    env = os.environ.copy()
    env.update(PERMAGENT_BUILD_TARGET=str(TARGET), CARGO_TARGET_DIR=str(TARGET), TMPDIR=str(TEMP),
               CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0', WORKTREES_ROOT=str(LANE.parent))
    env['PATH'] = '/Users/j/Documents/dev/permagent-runtime/bin:' + env.get('PATH', '')
    # The native runner already wraps Tauri in this exact repository guard.
    # Wrapping that runner again makes its child wait on its own parent's lock.
    # Recognize only the direct, known Python entry point; all other commands
    # still receive the repository guard here. The external flock and all
    # volume/deadline checks remain active in either case.
    native_runner = (
        len(command) >= 2
        # Apple's /usr/bin shim reports the Xcode interpreter as sys.executable.
        and Path(command[0]).resolve() in {Path(sys.executable).resolve(), Path('/usr/bin/python3')}
        and Path(command[1]).resolve() == LANE / 'reports/world-production/run_guarded_native.py'
    )
    if not native_runner:
        # The repository guard checks the internal data volume only; give it the
        # same internal threshold as this launcher (user-approved 2026-10-09).
        command = [str(LANE / 'scripts/build-guard.sh'), '--min-free-gb', str(START_GIB[INTERNAL]), '--', *command]
    log_path = ROOT / 'logs' / (time.strftime('%Y%m%d-%H%M%S') + f'-{os.getpid()}.log')
    def interrupted(signum, _frame):
        raise KeyboardInterrupt(f'Signal {signum}')
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, interrupted)
    proc = None
    reason = 'completed'
    minimum = initial.copy()
    with (ROOT / (LANE.name + '.lock')).open('a') as lock, log_path.open('x') as log:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        try:
            print(f'External build target: {TARGET}\nBuild log: {log_path}', flush=True)
            proc = subprocess.Popen(command, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            started = time.monotonic()
            while proc.poll() is None:
                current = space()
                minimum = {path: min(minimum[path], value) for path, value in current.items()}
                low = {path: free for path, free in current.items() if free < FLOOR_GIB[path]}
                if low:
                    raise RuntimeError('continuous floor reached (GiB free vs floor): '
                                       + str({k: (round(v, 1), FLOOR_GIB[k]) for k, v in low.items()}))
                if time.monotonic() - started > 3600:
                    raise RuntimeError('One-hour build deadline reached')
                time.sleep(1)
        except BaseException as error:
            reason = str(error)
            raise
        finally:
            if proc is not None:
                if proc.poll() is None:
                    stop_owned_group(proc)
                log_path.with_suffix('.json').write_text(json.dumps({
                    'target': str(TARGET), 'tmpdir': str(TEMP), 'exit_code': proc.returncode,
                    'reason': reason, 'initial_free_gib': initial, 'minimum_free_gib': minimum,
                }, indent=2) + '\n')
    print(f'Build exited {proc.returncode}', flush=True)
    return proc.returncode


if __name__ == '__main__':
    sys.exit(main())
