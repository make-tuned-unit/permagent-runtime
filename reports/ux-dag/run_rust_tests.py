#!/usr/bin/env python3
"""Run daemon lib regressions inside run-build, including macOS dylib signing.

Uses Cargo's exact artifact path and otool's exact dependency names; never
searches the external build volume. The outer guard owns all child processes.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--filter')
    parser.add_argument('--executable', type=Path, help='Reuse an explicitly relinked daemon test artifact')
    parser.add_argument('--source-revision', help='Required with --executable; current clean Rust source revision')
    parser.add_argument('--http-staging', action='store_true')
    parser.add_argument('--browser', action='store_true', help='Hold the isolated HTTP test for a browser walkthrough (10 minutes maximum)')
    args = parser.parse_args()
    target = Path(os.environ.get('CARGO_TARGET_DIR', '')).resolve()
    expected = Path('/Volumes/PermagentBuilds/permagent/targets/world-production-20261006')
    if target != expected or os.environ.get('CARGO_BUILD_JOBS') != '1':
        raise RuntimeError('Run through /Volumes/PermagentBuilds/permagent/run-build')
    environment = os.environ.copy()
    environment.setdefault('RUST_TEST_THREADS', '2')
    if args.executable:
        revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip()
        if args.source_revision != revision:
            raise RuntimeError('An existing artifact requires the exact current source revision')
        subprocess.run(['git', 'diff', '--exit-code', 'HEAD', '--',
                        'crates', 'Cargo.toml', 'Cargo.lock', '.cargo'], check=True)
        executable = args.executable.resolve()
        if (executable.parent != target / 'debug' / 'deps'
                or not executable.name.startswith('permagent_daemon-')
                or not executable.is_file()):
            raise RuntimeError('Expected an explicitly built isolated daemon test executable')
        print(f'Reusing explicitly relinked artifact for source {revision}', flush=True)
    else:
        # Reuse the recovered successful test configuration: default features,
        # unchanged dependencies, no debugger tables for the two large crates.
        command = ['cargo', '--config', 'profile.dev.package.permagent.debug=0',
                   '--config', 'profile.dev.package.permagent-daemon.debug=0',
                   'test', '-p', 'permagent-daemon', '--lib',
                   '--no-run', '--message-format=json']
        process = subprocess.Popen(command, stdout=subprocess.PIPE, text=True, env=environment)
        executable = None
        for line in process.stdout:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                print(line, end='', flush=True)
                continue
            if message.get('reason') == 'compiler-message':
                print(message['message'].get('rendered', ''), end='', flush=True)
            if (message.get('reason') == 'compiler-artifact'
                    and message.get('profile', {}).get('test')
                    and message.get('executable')
                    and message.get('target', {}).get('name') == 'permagent_daemon'):
                executable = Path(message['executable']).resolve()
        result = process.wait()
        if result:
            return result
        if executable is None or not executable.is_relative_to(target):
            raise RuntimeError('Cargo did not provide an isolated daemon test artifact')

    if sys.platform == 'darwin':
        profile = target / 'debug'
        pending, seen = [executable], set()
        # Resolve only libraries explicitly named by the built binary or its
        # dylibs, bounded to the same build profile. System libraries are read
        # by the loader normally and are never signed or modified here.
        while pending:
            artifact = pending.pop()
            dependencies = subprocess.check_output(['otool', '-L', str(artifact)], text=True)
            for line in dependencies.splitlines()[1:]:
                name = line.strip().split(' (', 1)[0]
                if not name.startswith('@rpath/'):
                    continue
                library = (profile / name.removeprefix('@rpath/')).resolve()
                if not library.is_relative_to(profile) or not library.is_file():
                    raise RuntimeError(f'Missing isolated dylib: {name}')
                if library in seen:
                    continue
                if len(seen) >= 16:
                    raise RuntimeError('Unexpected dylib dependency count')
                seen.add(library)
                pending.append(library)
                subprocess.run(['codesign', '--force', '--sign', '-', str(library)], check=True)
                subprocess.run(['codesign', '--verify', '--strict', str(library)], check=True)
        subprocess.run(['codesign', '--force', '--sign', '-', str(executable)], check=True)
        subprocess.run(['codesign', '--verify', '--strict', str(executable)], check=True)
        environment['DYLD_LIBRARY_PATH'] = str(profile)

    test_args = [str(executable)]
    browser_dir = None
    if args.http_staging or args.browser:
        test_args += ['--ignored', '--exact', 'routes::projects::tests::ux_stack_discovery_http_staging', '--nocapture']
        if args.browser:
            browser_dir = Path(tempfile.mkdtemp(prefix='ux-browser-', dir=environment['TMPDIR']))
            environment['PERMAGENT_UX_BROWSER_DIR'] = str(browser_dir)
            print(f'Browser review directory: {browser_dir}', flush=True)
    elif args.filter:
        test_args.append(args.filter)
    print('Running isolated daemon tests:', ' '.join(test_args), flush=True)
    try:
        return subprocess.call(test_args, env=environment)
    finally:
        if browser_dir:
            for name in ('state.json', 'finish'):
                (browser_dir / name).unlink(missing_ok=True)
            browser_dir.rmdir()


if __name__ == '__main__':
    sys.exit(main())
