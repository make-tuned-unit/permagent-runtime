#!/usr/bin/env python3
"""One-off, exact-baseline catalogue repair; default is read-only plan generation."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import time
import uuid
import isolated_daemon as isolated
import test_isolated_meeting_http as http

REPORTS = Path(__file__).resolve().parent
INPUTS = {
    'restart-baseline-20261007.json': '6600eed7c6355ee86193cc363d26cde48a36f681cf3621bd28c35bcd443e6d0c',
    'isolated-meeting-http-20261008-015317.json': '60ea1de3019faa3c5348457dd87ff13eae3efbc892020ffb176174840c8004f7',
    'isolated-meeting-http-20261008-015348.json': '6c8b2c71ab39f3a843915611364c8b78a3393e796df5457338e13da3066c423f',
}


def fingerprint(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


def read_json(path, expected=None):
    if path.resolve() != path or not path.is_file():
        raise RuntimeError('Evidence must be a canonical regular file')
    data = path.read_bytes()
    if len(data) > 2 * 1024 * 1024 or (expected and hashlib.sha256(data).hexdigest() != expected):
        raise RuntimeError('Evidence input hash/size mismatch')
    return json.loads(data)


def original_payloads():
    inputs = {name: read_json(REPORTS / name, sha) for name, sha in INPUTS.items()}
    baseline = inputs['restart-baseline-20261007.json']
    if baseline['root'] != str(isolated.ROOT) or len(baseline['triages']) != 3:
        raise RuntimeError('Only the exact three-record isolated baseline is supported')
    targets = []
    for triage in baseline['triages']:
        candidates = []
        for name, evidence in inputs.items():
            if name.startswith('isolated-meeting-http-'):
                for row in evidence.get('projects', []):
                    if row['project']['id'] == triage['project_id'] and row['savedNoteId'] == triage['note_id']:
                        payload = row['payload']
                        if (set(payload) != {'title', 'body', 'kind', 'recordingId'} or
                                payload['kind'] != 'meeting' or payload['body'] != evidence['fixtureBody'] or
                                payload['recordingId'] != evidence['recordingId'] or
                                row['initialReceipt']['id'] != triage['id']):
                            raise RuntimeError('Original payload provenance mismatch')
                        candidates.append({'baseline': triage, 'payload': payload, 'source': name})
        if len(candidates) != 1:
            raise RuntimeError('Each old triage must have exactly one preserved original payload')
        targets.extend(candidates)
    return targets


def snapshot(targets):
    isolated.root_check()
    db = http.db_open()
    brain_path = isolated.ROOT / 'brain/memory.db'
    if brain_path.resolve() != brain_path or not brain_path.is_file():
        raise RuntimeError('Brain database must be canonical')
    brain = sqlite3.connect(brain_path.as_uri() + '?mode=ro', uri=True)
    brain.row_factory = sqlite3.Row
    brain.execute('PRAGMA query_only=ON')
    result = []
    try:
        db.execute('BEGIN')
        brain.execute('BEGIN')
        for target in targets:
            prior, payload = target['baseline'], target['payload']
            pid, nid = prior['project_id'], prior['note_id']
            rows = [dict(row) for row in db.execute('SELECT * FROM meeting_triage WHERE project_id=?', (pid,))]
            if len(rows) != 1 or any(rows[0].get(key) != value for key, value in prior.items()):
                raise RuntimeError('Old triage state differs from completed restart baseline')
            triage = rows[0]
            expected_body_hash = hashlib.sha256(payload['body'].strip().encode()).hexdigest()
            if triage['recording_id'] != payload['recordingId'] or triage['body_hash'] != expected_body_hash:
                raise RuntimeError('Original payload does not match immutable stored body hash')
            notes = [dict(row) for row in db.execute('SELECT * FROM project_notes WHERE project_id=? ORDER BY id', (pid,))]
            cards = [dict(row) for row in db.execute('SELECT * FROM cards WHERE project_id=? ORDER BY id', (pid,))]
            if len(notes) != 1 or notes[0]['id'] != nid or {c['id'] for c in cards} != set(json.loads(prior['card_ids_json'])):
                raise RuntimeError('Prior project note/card footprint changed')
            messages = [dict(row) for row in db.execute('SELECT * FROM messages WHERE session_id=? ORDER BY id', (triage['session_id'],))]
            key = f'note:{pid}:{nid}'
            memories = [dict(row) for row in brain.execute('SELECT id, content FROM memories WHERE key=?', (key,))]
            association_count = 0 if len(memories) != 1 else db.execute(
                'SELECT COUNT(*) FROM project_memories WHERE project_id=? AND memory_id=?', (pid, memories[0]['id'])).fetchone()[0]
            outbox = [dict(row) for row in db.execute('SELECT status, attempts FROM brain_feed_outbox WHERE memory_key=?', (key,))]
            result.append({'triageId': prior['id'], 'projectId': pid, 'noteId': nid,
                'recordingId': triage['recording_id'], 'source': target['source'], 'payloadSha256': fingerprint(payload),
                'originalBodySha256': expected_body_hash,
                'stableStateSha256': fingerprint({'triage': triage, 'notes': notes, 'cards': cards, 'messages': messages}),
                'noteCount': len(notes), 'cardIds': [c['id'] for c in cards], 'messageCount': len(messages),
                'memoryKey': key, 'memoryCount': len(memories), 'associationCount': association_count,
                'fullSavedBodyCatalogued': len(memories) == 1 and notes[0]['body'] in memories[0]['content'],
                'outbox': outbox})
    finally:
        db.rollback(); brain.rollback(); db.close(); brain.close()
    return result


def private_receipt(prefix, value):
    path = REPORTS / (prefix + '-' + uuid.uuid4().hex + '.json')
    if REPORTS.resolve() != REPORTS:
        raise RuntimeError('Reports directory must be canonical')
    fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as handle:
        json.dump(value, handle, indent=2); handle.write('\n')
    return path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true')
    parser.add_argument('--plan')
    parser.add_argument('--plan-sha256')
    parser.add_argument('--daemon-sha256')
    args = parser.parse_args()
    targets = original_payloads()
    before = snapshot(targets)  # All three validated before any HTTP request.
    if not args.execute:
        path = private_receipt('old-association-repair-plan', {'mode': 'read-only plan; zero API calls',
            'root': str(isolated.ROOT), 'url': isolated.URL, 'inputs': INPUTS, 'before': before,
            'requests': [{'method': 'POST', 'path': f"/api/projects/{r['projectId']}/notes", 'payloadSha256': r['payloadSha256']} for r in before]})
        print(json.dumps({'plan': str(path), 'planSha256': isolated.digest(path), 'targets': len(before), 'apiCalls': 0}))
        return
    if not all([args.plan, args.plan_sha256, args.daemon_sha256]):
        parser.error('--execute requires exact --plan, --plan-sha256 and --daemon-sha256')
    plan_path = Path(args.plan)
    if plan_path.parent != REPORTS or not plan_path.name.startswith('old-association-repair-plan-'):
        raise RuntimeError('Only this helper’s canonical plan is accepted')
    plan = read_json(plan_path, args.plan_sha256)
    if plan['root'] != str(isolated.ROOT) or plan['url'] != isolated.URL or plan['inputs'] != INPUTS:
        raise RuntimeError('Repair plan identity mismatch')
    if plan['before'] != before:
        raise RuntimeError('Old baseline changed after plan generation; inspect before retrying')
    manifest = isolated.manifest()
    if manifest['binary']['sha256'] != args.daemon_sha256:
        raise RuntimeError('Expected rebuilt daemon identity mismatch')
    evidence = {'mode': 'three exact idempotent note-save replays, no inference request',
                'daemonSha256': args.daemon_sha256, 'plan': str(plan_path), 'before': before, 'responses': [], 'passed': False}
    try:
        for target, row in zip(targets, before):
            response = http.api(f"/api/projects/{row['projectId']}/notes", target['payload'])
            receipt = response.get('triage', {})
            identity_ok = (response.get('id') == row['noteId'] and receipt.get('id') == row['triageId'] and
                           receipt.get('created') is False and receipt.get('status') == 'completed' and
                           set(receipt.get('cardIds', [])) == set(row['cardIds']))
            evidence['responses'].append({'noteId': response.get('id'), 'triageId': receipt.get('id'), 'identityPassed': identity_ok})
            if not identity_ok:
                raise RuntimeError('Replay failed immutable identity/completed/card checks; no further POST attempted')
        until = time.monotonic() + 90
        while True:
            after = snapshot(targets)
            evidence['after'] = after
            stable = all(a['stableStateSha256'] == b['stableStateSha256'] for a, b in zip(after, before))
            if not stable:
                raise RuntimeError('Replay changed existing notes/cards/triages/session messages')
            complete = all(row['memoryCount'] == row['associationCount'] == 1 and row['fullSavedBodyCatalogued'] and not row['outbox'] for row in after)
            if complete:
                evidence['passed'] = True
                break
            if time.monotonic() >= until:
                raise RuntimeError('Catalogue project association did not complete within90s')
            time.sleep(2)
    except Exception as error:
        evidence['error'] = http.failure_details(error)
    path = private_receipt('old-association-repair-result', evidence)
    print(json.dumps({'passed': evidence['passed'], 'evidence': str(path)}))
    raise SystemExit(0 if evidence['passed'] else 1)


if __name__ == '__main__':
    main()
