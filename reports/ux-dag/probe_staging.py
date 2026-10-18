#!/usr/bin/env python3
"""Independently probe the test-owned :3002 daemon; never browser evidence.

Run while run_rust_tests.py --browser holds its private review directory.
Only that test's synthetic project/person may be edited. A failed probe leaves
the finish marker absent, so failure cannot turn into a passing daemon test.
"""
import argparse
import http.client
import json
from pathlib import Path
import time
from uuid import UUID


REVIEW_ROOT = Path('/Volumes/PermagentBuilds/permagent/tmp/world-production-20261006')


def request(path, token, method='GET', body=None):
    connection = http.client.HTTPConnection('127.0.0.1', 3002, timeout=3)
    try:
        connection.request(method, path,
                           body=None if body is None else json.dumps(body),
                           headers={'Authorization': 'Bearer ' + token,
                                    'Content-Type': 'application/json'})
        response = connection.getresponse()
        data = response.read()
        if not 200 <= response.status < 300:
            raise RuntimeError(f'{method} {path}: HTTP {response.status}')
        return json.loads(data) if data else None
    except (OSError, http.client.HTTPException, json.JSONDecodeError) as error:
        # Name the stalled surface without copying headers or response bodies.
        raise RuntimeError(f'{method} {path}: {type(error).__name__}') from None
    finally:
        connection.close()


def probe(state, seconds, call=request, clock=time.monotonic, pause=time.sleep):
    if not isinstance(state, dict):
        raise RuntimeError('Review state must be a JSON object')
    if not isinstance(state.get('token'), str) or not state['token'].strip():
        raise RuntimeError('Review state has no usable bearer token')
    project, person = str(UUID(state['projectId'])), str(UUID(state['personId']))
    token = state['token']
    project_path = f'/api/projects/{project}'
    # Prove the two records belong to this test before any mutation.
    record = call(project_path, token)
    if not record.get('name', '').startswith('UX HTTP fixture '):
        raise RuntimeError('Refusing a project outside the synthetic UX fixture')
    people = call('/api/people/directory', token)
    contact = next((p for p in people if p.get('entity_uuid') == person), None)
    if not contact or not contact.get('display_name', '').lower().startswith('ux fixture '):
        raise RuntimeError('Refusing a person outside the synthetic UX fixture')

    paths = [project_path, project_path + '/stack/detected', project_path + '/cards',
             '/api/people/directory', '/schedule/list',
             '/api/sessions?types=user,scheduled,sub_agent']
    started, requests, slowest = clock(), 0, 0.0
    deadline = started + seconds
    while clock() < deadline:
        for path in paths:
            # Do not start another surface read after the deadline. The
            # HTTPConnection timeout applies to socket operations; a peer that
            # keeps trickling data can extend an in-flight request's duration.
            if clock() >= deadline:
                break
            before = clock()
            call(path, token)
            slowest = max(slowest, clock() - before)
            requests += 1
        remaining = deadline - clock()
        if remaining > 0:
            pause(min(2, remaining))

    call(project_path, token, method='PATCH', body={'status': 'active'})
    call(f'/api/people/{person}/fields', token, method='PATCH',
         body={'fields': {'notes': 'UX browser wiring verified'}})
    # The daemon test independently checks these same values after finish.
    if call(project_path, token).get('status') != 'active':
        raise RuntimeError('Project status did not persist')
    edited = next(p for p in call('/api/people/directory', token) if p.get('entity_uuid') == person)
    if edited.get('notes') != 'UX browser wiring verified':
        raise RuntimeError('Person notes did not persist')
    return {'read_requests': requests, 'elapsed_seconds': round(clock() - started, 2),
            'slowest_read_seconds': round(slowest, 3), 'persistence_verified': True}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--seconds', type=int, default=240, choices=range(1, 301), metavar='1..300')
    args = parser.parse_args()
    if args.directory.is_symlink():
        raise RuntimeError('Review directory must not be a symlink')
    directory = args.directory.resolve()
    if directory.parent != REVIEW_ROOT or not directory.name.startswith('ux-browser-'):
        raise RuntimeError('Use the exact private directory printed by the isolated test runner')
    state_path = directory / 'state.json'
    if state_path.is_symlink() or not state_path.is_file() or state_path.stat().st_mode & 0o077:
        raise RuntimeError('Review state must be a private, regular test credential file')
    try:
        state = json.loads(state_path.read_text())
    except (OSError, UnicodeError, json.JSONDecodeError):
        raise RuntimeError('Review state could not be read as JSON') from None
    result = probe(state, args.seconds)
    (directory / 'finish').touch(exist_ok=False)
    print(json.dumps(result), flush=True)


if __name__ == '__main__':
    main()
