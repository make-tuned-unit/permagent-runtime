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
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter', default='system_audio::tests')
    args = parser.parse_args()
    guard.preflight()
    env = os.environ.copy()
    for name in ('PERMAGENT_REVIEW_ROOT', 'PERMAGENT_REVIEW_PORT', 'VITE_DAEMON_URL', 'VITE_API_BASE_URL'):
        env.pop(name, None)
    env.update({
        'PATH': '/Users/j/Documents/dev/permagent-runtime/bin:' + env.get('PATH', ''),
        'WORKTREES_ROOT': str(guard.LANE.parent),
        'CARGO_TARGET_DIR': str(guard.TARGET),
        'CARGO_NET_OFFLINE': 'true', 'CARGO_BUILD_JOBS': '1',
        'CARGO_PROFILE_DEV_DEBUG': '0', 'CARGO_PROFILE_TEST_DEBUG': '0', 'CARGO_INCREMENTAL': '0',
        'TAURI_CONFIG': (guard.REPORTS / 'native-review-config.json').read_text(),
    })
    command = ['/Users/j/Documents/dev/permagent-runtime/bin/cargo', 'test', '--manifest-path',
               'ui/desktop/src-tauri/Cargo.toml', '--locked', '-j', '1', args.filter, '--', '--nocapture']
    stamp = time.strftime('%Y%m%d-%H%M%S')
    log_path = guard.REPORTS / f'record-native-tests-{stamp}.log'
    summary_path = log_path.with_suffix('.json')
    started = time.monotonic()
    proc = None
    minimum_free = guard.free_gib()
    reason = None
    rc = 1
    cleanup_attempted = False
    cleanup_error = None
    preservation = guard.DaemonPreservation(guard.TARGET)
    preservation_error = None
    owned_group_stopped = True
    def interrupted(signum, _frame):
        raise KeyboardInterrupt(f'Signal {signum}')
    previous = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        with log_path.open('x', buffering=1) as log:
            log.write('Command: ' + json.dumps(command) + '\n')
            print(f'Test log: {log_path}', flush=True)
            preservation.prepare()
            log.write('Daemon preservation: ' + json.dumps(preservation.evidence) + '\n')
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
        try:
            if proc is not None:
                owned_group_stopped = False
                if cleanup_error is None:
                    owned_group_stopped = guard.wait_owned_group(proc)
                    if not owned_group_stopped:
                        cleanup_attempted = True
                        guard.stop_owned_group(proc)
                        owned_group_stopped = True
            preservation.restore(group_stopped=owned_group_stopped)
        except BaseException as error:
            preservation_error = f'{type(error).__name__}: {error}'
            rc = 1
            reason = reason or 'Daemon artifact restoration failed; preserved backup requires review'
        for sig, handler in previous.items():
            signal.signal(sig, handler)
        summary = dict(command=command, log=str(log_path), exit_code=rc, reason=reason,
                       cleanup_error=cleanup_error, owned_group_stopped=owned_group_stopped,
                       daemon_artifact_preservation=preservation.evidence,
                       daemon_preservation_error=preservation_error,
                       elapsed_s=round(time.monotonic() - started, 2), minimum_free_gib=round(minimum_free, 3),
                       startup_minimum_gib=guard.START_GIB, continuous_floor_gib=guard.FLOOR_GIB,
                       timeout_s=guard.DEADLINE_SECONDS, staged=False, launched=False)
        with summary_path.open('x') as handle:
            handle.write(json.dumps(summary, indent=2) + '\n')
        print(json.dumps(summary), flush=True)
    return rc if rc >= 0 else 128 - rc

if __name__ == '__main__':
    sys.exit(main())
