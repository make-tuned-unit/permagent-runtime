#!/usr/bin/env python3
"""Opt-in empty-state daemon startup/restart probe; no inference or UI claims."""
import argparse
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

import isolated_daemon as isolated
from run_guarded_native import stop_owned_group


def free_port(port):
    with socket.socket() as probe:
        probe.settimeout(1)
        if probe.connect_ex(('127.0.0.1', port)) == 0:
            raise RuntimeError('Requested loopback port has a listener')
    with socket.socket() as sock:
        # Permit restart after our own clean shutdown leaves TCP TIME_WAIT.
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.bind(('127.0.0.1', port))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--sha256', required=True)
    args = parser.parse_args()
    if not args.execute:
        print('Requires --execute; creates a new private state root and uses only loopback3003.')
        return
    def interrupted(signum, _frame):
        raise InterruptedError('Probe interrupted by signal ' + str(signum))
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, interrupted)
    binary = isolated.artifact(isolated.DEBUG / 'permagentd', args.sha256)
    for library in isolated.libraries():
        subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(library)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    # Bundled defaults are discovered relative to the executable; refuse hidden seeding.
    for ancestor in Path(binary['path']).parents:
        for name in ('defaults.yaml', 'init-config.yaml'):
            if (ancestor / name).exists():
                raise RuntimeError('Executable ancestor has configuration defaults; review before fresh probe')
    free_port(3003)
    # A high unbound port can be verified without privileged-port bind rights.
    unavailable_service_port = 43009
    free_port(unavailable_service_port)
    if shutil.disk_usage(isolated.TARGET).free < 25 * 2**30:
        raise RuntimeError('25GiB startup floor not met')
    stamp = time.strftime('%Y%m%d-%H%M%S', time.gmtime())
    root = isolated.TARGET / ('first-user-qa-' + stamp)
    if root.parent.resolve() != isolated.TARGET:
        raise RuntimeError('Unexpected target path')
    root.mkdir(mode=0o700)
    state = root / 'state'
    state.mkdir(mode=0o700)
    empty_home = root / 'home'
    empty_home.mkdir(mode=0o700)
    if list(state.iterdir()):
        raise RuntimeError('State must begin empty')
    env = {
        'HOME': str(empty_home), 'PATH': '/usr/bin:/bin:/usr/sbin:/sbin', 'LANG': 'en_US.UTF-8',
        'PERMAGENT_PATH_ROOT': str(state), 'PERMAGENT_CONFIG': str(state / 'config.yaml'),
        'PERMAGENT_SPECTRAL_DB': str(state / 'spectral/permagent.db'),
        'PERMAGENT_DISABLE_KEYRING': '1', 'DYLD_LIBRARY_PATH': str(isolated.DEBUG),
        'GOOSE_TELEMETRY_ENABLED': 'false', 'COUNCIL_ENABLED': 'false',
        'CONCIERGE_ENABLED': 'false', 'STRIX_ENABLED': 'false', 'STEWARD_SCAN_ENABLED': 'false',
        'POLYBOT_ENABLED': 'false', 'TUNNEL_AUTO_START': 'false',
        'PERMAGENT_MESH_TRUSTED': '0', 'PERMAGENT_MESH_ENGINE': '0',
        'OLLAMA_HOST': 'http://127.0.0.1:' + str(unavailable_service_port),
        'PERMAGENT_OLLAMA_HOST': 'http://127.0.0.1:' + str(unavailable_service_port),
    }
    report = {'binary': binary, 'root': str(root), 'port': 3003, 'initialStateEmpty': True,
              'method': 'real daemon empty data-root and HOME startup/restart; same OS account, no native/UI acceptance',
              'initialHomeEmpty': True,
              'keyringDisabled': True, 'inheritedProviderOrCredentialEnvironment': False,
              'serviceOverrides': sorted(k for k in env if k.endswith('_ENABLED') or k.endswith('_HOST')),
              'phases': [], 'tokenLogged': False}
    report_path = root / 'evidence.json'
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    first_ontology = None
    try:
        for phase in ('fresh', 'restart'):
            free_port(3003)
            minimum = shutil.disk_usage(root).free / 2**30
            if minimum < 25:
                raise RuntimeError('25GiB restart floor not met')
            row = {'phase': phase, 'checks': {}}
            report['phases'].append(row)
            started = time.monotonic()
            log_path = root / (phase + '.log')
            fd = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, 'wb') as log:
                proc = subprocess.Popen([binary['path'], 'agent', '--host', '127.0.0.1', '--port', '3003'],
                                        cwd=root, env=env, stdout=log, stderr=subprocess.STDOUT,
                                        start_new_session=True)
                try:
                    def request(path, token=None, data=None, origin=None):
                        nonlocal minimum
                        minimum = min(minimum, shutil.disk_usage(root).free / 2**30)
                        if minimum < 20 or time.monotonic() - started > 180:
                            raise RuntimeError('Disk or180second runtime guard reached')
                        headers = {'Content-Type': 'application/json'}
                        if token:
                            headers['Authorization'] = 'Bearer ' + token
                        if origin:
                            headers['Origin'] = origin
                        req = urllib.request.Request('http://127.0.0.1:3003' + path, headers=headers,
                            data=None if data is None else json.dumps(data).encode())
                        with opener.open(req, timeout=10) as response:
                            return json.load(response)

                    token_path = state / 'secrets/daemon_token.json'
                    while True:
                        if proc.poll() is not None:
                            raise RuntimeError('Owned daemon exited before readiness')
                        try:
                            if token_path.resolve() != token_path or token_path.stat().st_mode & 0o077:
                                raise RuntimeError('Token file must be private and canonical')
                            token = json.loads(token_path.read_text())['token']
                            if not re.fullmatch('[0-9a-f]{64}', token):
                                raise RuntimeError('Unexpected token format')
                            projects = request('/api/projects', token)
                            break
                        except (OSError, ValueError, KeyError):
                            if time.monotonic() - started > 120:
                                raise RuntimeError('Fresh readiness deadline exceeded') from None
                            time.sleep(1)
                    config = {key: request('/config/read', token, {'key': key, 'is_secret': False})
                              for key in ('GOOSE_PROVIDER', 'GOOSE_MODEL', 'wizard_complete', 'LOCAL_WHISPER_MODEL')}
                    status = request('/api/dictation/status', token)
                    memories = request('/api/brain/memories?limit=1', token)
                    owner = request('/api/owner', token)
                    pronunciations = request('/voice/pronunciations', token)
                    schedules = request('/schedule/list', token)
                    starter_jobs = [job for job in schedules.get('jobs', []) if job.get('starter_id')]
                    ontology = state / 'brain/ontology.toml'
                    ontology_hash = isolated.digest(ontology)
                    if first_ontology is None:
                        first_ontology = ontology_hash
                    row['dictationStatus'] = status
                    project_rows = projects if isinstance(projects, list) else projects.get('projects')
                    if not isinstance(project_rows, list):
                        raise RuntimeError('Unexpected projects response shape')
                    row['projectCount'] = len(project_rows)
                    row['memoryCount'] = memories.get('total')
                    row['checks'].update({
                        'noProvider': not config['GOOSE_PROVIDER'], 'noModel': not config['GOOSE_MODEL'],
                        'wizardIncomplete': config['wizard_complete'] in (None, False),
                        'dictationUnconfigured': status == {'status': 'unavailable', 'reason': 'not_configured',
                                                           'model': None, 'inference_verified': False},
                        'brainReadable': isinstance(memories.get('memories'), list),
                        'brainEmpty': memories.get('total') == 0,
                        'ownerEmpty': isinstance(owner, dict) and all(
                            owner.get(key) is None for key in
                            ('display_name', 'email', 'linked_person_id', 'linked_person_name')),
                        'pronunciationsEmpty': pronunciations == {},
                        'allStartersPresent': {job.get('starter_id') for job in starter_jobs} == set(isolated.STARTERS),
                        'startersAwaitReview': bool(starter_jobs) and all(
                            job.get('paused') is True and job.get('requires_approval') is True
                            and job.get('currently_running') is False and job.get('last_run') is None
                            for job in starter_jobs),
                        # Product migration seeds one neutral, folderless Personal project.
                        'onlyProductStarterProject': len(project_rows) == 1 and all(
                            project.get('id') == '00000000-0000-0000-0000-000000000001'
                            and project.get('slug') == 'personal' and project.get('name') == 'Personal'
                            and all(not project.get(key) for key in ('root_path', 'site_url', 'repo_url'))
                            for project in project_rows),
                        'ontologyPreserved': ontology_hash == first_ontology,
                        'productOntology': ontology.read_bytes() == (isolated.LANE / 'crates/goose-server/src/brain_default_ontology.toml').read_bytes(),
                    })
                    for name, auth, origin in [('unauthorizedRejected', None, None),
                                               ('foreignOriginRejected', token, 'https://qa-untrusted.invalid')]:
                        try:
                            request('/api/dictation/status', auth, origin=origin)
                            row['checks'][name] = False
                        except urllib.error.HTTPError as error:
                            row['checks'][name] = error.code in (401, 403)
                            row[name + 'Status'] = error.code
                finally:
                    stop_owned_group(proc)
                    row.update(exitCode=proc.returncode, elapsedSeconds=time.monotonic() - started,
                               minimumFreeGiB=minimum, stopped=True)
                    isolated.write_json(report_path, report)
        report['passed'] = all(all(row['checks'].values()) for row in report['phases'])
    except Exception as error:
        report.update(passed=False, error=type(error).__name__ + ': ' + str(error)[:250])
    finally:
        isolated.write_json(report_path, report)
    print(json.dumps({'passed': report['passed'], 'evidence': str(report_path)}))
    raise SystemExit(0 if report['passed'] else 1)


if __name__ == '__main__':
    main()
