#!/usr/bin/env python3
"""Explicit isolated frontend server and legitimate one-time pairing helper.
No default action launches anything; no browser/clipboard manipulation.
"""
import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import time
import uuid
import isolated_daemon as isolated

UI = isolated.LANE / 'ui/command-center'
CONFIG = UI / 'vite.record-review.config.mjs'
BROWSER_URL = 'http://127.0.0.1:5302/ui/'
# Existing coordinator runtime, resolved from mise's lts alias during preflight.
# Do not search inherited PATH or follow a moving version alias at launch.
NODE = Path('/Users/j/.local/share/mise/installs/node/24.19.0/bin/node')


def browser_port_free():
    with socket.socket() as sock:
        try:
            sock.bind(('127.0.0.1', 5302))
        except OSError:
            raise RuntimeError('Port5302 occupied; no existing listener will be stopped') from None


def clean_env():
    return {'PATH': str(NODE.parent) + ':/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin',
            'HOME': os.environ['HOME'], 'LANG': 'en_US.UTF-8',
            'NODE_OPTIONS': '--max-old-space-size=1024'}


def frontend_command():
    if not NODE.is_file() or not os.access(NODE, os.X_OK) or NODE.resolve() != NODE:
        raise RuntimeError(f'Trusted Node executable missing, non-executable or redirected: {NODE}')
    if not CONFIG.is_file():
        raise RuntimeError(f'Isolated Vite config missing: {CONFIG}')
    vite = UI / 'node_modules/vite/bin/vite.js'
    if not vite.is_file():
        raise RuntimeError(f'Installed Vite entry point missing: {vite}')
    return [str(NODE), str(vite), '--config', str(CONFIG)]


def pair_link():
    isolated.manifest()
    # Parent starts Vite first. This check never falls back to any other port.
    with socket.create_connection(('127.0.0.1', 5302), timeout=3):
        pass
    response = isolated.request(isolated.URL + '/api/devices/pair',
        {'name': 'Record QA browser5302'}, isolated.token())
    code = response.get('claim_code', '')
    import re
    if not re.fullmatch('[0-9a-fA-F]{32}', code):
        raise RuntimeError('Isolated pairing endpoint returned an unexpected claim format')
    directory = isolated.ROOT / 'browser-pairing'
    if directory.resolve() != directory:
        raise RuntimeError('Pairing handoff directory must be canonical')
    directory.mkdir(mode=0o700, exist_ok=True)
    os.chmod(directory, 0o700)
    path = directory / ('claim-' + uuid.uuid4().hex + '.txt')
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as handle:
        handle.write(BROWSER_URL + '#claim=' + code + '\n')
    # The only returned content is the private file path. Never log code/token/URL fragment.
    print(path)


def start(args):
    command = frontend_command()
    isolated.manifest()
    isolated.request(isolated.URL + '/api/projects', token=isolated.token())
    browser_port_free()
    stop = False
    def stop_signal(_signum, _frame):
        nonlocal stop
        stop = True
    signal.signal(signal.SIGINT, stop_signal)
    signal.signal(signal.SIGTERM, stop_signal)
    proc = subprocess.Popen(command, cwd=UI, env=clean_env(), start_new_session=True)
    started = time.monotonic()
    try:
        while proc.poll() is None and not stop and time.monotonic() - started < args.max_seconds:
            time.sleep(1)
        if proc.poll() is not None and not stop and proc.returncode:
            raise RuntimeError(f'Owned Vite exited with code {proc.returncode}')
    finally:
        isolated.stop_owned_group(proc)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest='action', required=True)
    launch = sub.add_parser('start')
    launch.add_argument('--max-seconds', type=int, default=3600)
    sub.add_parser('pair-link', help='Mint isolated one-use claim; return only private0600 handoff file path')
    sub.add_parser('dry-run')
    args = parser.parse_args()
    if args.action == 'start':
        if not 1 <= args.max_seconds <= 43200: parser.error('max-seconds must be 1..43200')
        start(args)
    elif args.action == 'pair-link': pair_link()
    else:
        print(json.dumps({'browser': BROWSER_URL, 'daemon': isolated.URL, 'config': str(CONFIG), 'command': frontend_command(),
                          'launched': False, 'pairing': 'Public one-time #claim flow; no bearer injection'}))

if __name__ == '__main__':
    try: main()
    except Exception as error:
        print(f'Isolated browser stopped: {error}', file=__import__('sys').stderr)
        raise SystemExit(1)
