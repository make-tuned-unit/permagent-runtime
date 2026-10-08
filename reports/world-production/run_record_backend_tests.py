#!/usr/bin/env python3
"""Focused native Record tests, with the isolated review build's process/disk guard."""
import json
import argparse
import os
import signal
import subprocess
import sys
import time
import run_guarded_native as guard

def main():
    parser = argparse.ArgumentParser(description='Run a focused guarded backend check')
    parser.add_argument('--filter', default='meeting_triage')
    parser.add_argument('--daemon-tests', action='store_true')
    parser.add_argument('--build-daemon', action='store_true')
    args = parser.parse_args()
    guard.preflight()
    env = os.environ.copy()
    env.update({
        'PATH': '/Users/j/Documents/dev/permagent-runtime/bin:' + env.get('PATH', ''),
        'WORKTREES_ROOT': str(guard.LANE.parent),
        'CARGO_TARGET_DIR': str(guard.TARGET),
        'CARGO_NET_OFFLINE': 'false', 'CARGO_BUILD_JOBS': '1',
        'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_TEST_DEBUG': '0', 'CARGO_INCREMENTAL': '0',
        'TAURI_CONFIG': (guard.REPORTS / 'native-review-config.json').read_text(),
    })
    command = ['/Users/j/Documents/dev/permagent-runtime/bin/cargo', 'test', '-p', 'permagent', args.filter, '--lib', '--locked']
    if args.daemon_tests:
        command = [str(guard.LANE / 'scripts/test-daemon.sh'), '--lib', args.filter]
    if args.build_daemon:
        command = ['/Users/j/Documents/dev/permagent-runtime/bin/cargo', 'build', '-p', 'permagent-daemon', '--bin', 'permagentd', '--locked']

    stamp = time.strftime('%Y%m%d-%H%M%S')
    log_path = guard.REPORTS / f'record-backend-tests-{stamp}.log'
    summary_path = log_path.with_suffix('.json')
    started = time.monotonic()
    proc = None
    minimum_free = guard.free_gib()
    reason = None
    cleanup_attempted = False
    cleanup_error = None
    rc = 1
    def interrupted(signum, _frame):
        raise KeyboardInterrupt(f'Signal {signum}')
    previous = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        with log_path.open('x', buffering=1) as log:
            log.write('Command: ' + json.dumps(command) + '\n')
            print(f'Test log: {log_path}', flush=True)
            proc = subprocess.Popen(command, cwd=guard.LANE, env=env, stdin=subprocess.DEVNULL,
                                    stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            while True:
                free = guard.free_gib()
                minimum_free = min(minimum_free, free)
                if free < guard.FLOOR_GIB:
                    reason = f'Free disk below {guard.FLOOR_GIB} GiB: {free:.2f}'
                    break
                if time.monotonic() - started >= guard.DEADLINE_SECONDS:
                    reason = 'Native test deadline exceeded'
                    break
                status = proc.poll()
                if status is not None:
                    rc = status
                    break
                time.sleep(2)
            if reason:
                cleanup_attempted = True
                guard.stop_owned_group(proc)
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
                    guard.stop_owned_group(proc)
                except BaseException as cleanup:
                    cleanup_error = f'{type(cleanup).__name__}: {cleanup}'
        rc = 130 if isinstance(error, KeyboardInterrupt) else 1
    finally:
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        summary = dict(command=command, log=str(log_path), exit_code=rc, reason=reason, cleanup_error=cleanup_error,
                       owned_group_stopped=cleanup_attempted and cleanup_error is None,
                       elapsed_s=round(time.monotonic() - started, 2), minimum_free_gib=round(minimum_free, 3),
                       startup_minimum_gib=guard.START_GIB, continuous_floor_gib=guard.FLOOR_GIB,
                       timeout_s=guard.DEADLINE_SECONDS, staged=False, launched=False)
        summary_path.write_text(json.dumps(summary, indent=2) + '\n')
        print(json.dumps(summary), flush=True)
    return rc if rc >= 0 else 128 - rc

if __name__ == '__main__':
    sys.exit(main())
