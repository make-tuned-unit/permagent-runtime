#!/usr/bin/env python3
"""Explicit synthetic HTTP acceptance on the prepared Record daemon at 3002.
This is backend acceptance, NOT native capture or microphone evidence.
"""
import argparse
import json
import os
from pathlib import Path
import sqlite3
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
import isolated_daemon as isolated

REPORTS = Path(__file__).resolve().parent
TAIL = 'Final commitment: Morgan will confirm the venue step-free route by Tuesday.'
TRANSCRIPT = ('[00:02] Riley: This is a synthetic meeting for Record acceptance. '
              'I will draft the amber launch checklist by Friday.\n'
              '[00:14] Morgan: The equipment purchase is cancelled. Do not buy equipment. '
              'Do not send any email. The launch is not approved.\n'
              '[00:49] Morgan: ' + TAIL)
BODY = '## Your notes\nSynthetic QA only. No external actions authorized.\n\n## Transcript\n' + TRANSCRIPT


SCENARIOS = {
    'original': {'transcript': TRANSCRIPT, 'tail': TAIL,
                 'negations': ['The equipment purchase is cancelled.', 'Do not send any email.']},
    'paraphrase-a': {
        'transcript': "[00:03] Riley: I'll put together the internal amber launch checklist for Friday.\n"
                      "[00:17] Morgan: The earlier equipment order is off; nobody should purchase it. "
                      "Keep email out of scope: don't contact anyone outside this project. "
                      "That restriction does not cancel our internal planning work.\n"
                      "[00:51] Morgan: My remaining action is to verify the venue's step-free route on the plan by Tuesday.",
        'tail': "My remaining action is to verify the venue's step-free route on the plan by Tuesday.",
        'negations': ['The earlier equipment order is off; nobody should purchase it.',
                      "Keep email out of scope: don't contact anyone outside this project."]},
    'paraphrase-b': {
        'transcript': '[00:04] Riley: Put me down to prepare the internal amber launch checklist before Friday.\n'
                      '[00:19] Morgan: We have abandoned buying the equipment; remove that purchase idea. '
                      'Sending messages to external recipients is prohibited. '
                      'Our two internal follow-ups are still agreed.\n'
                      '[00:53] Morgan: I take responsibility for checking the venue plan and confirming its step-free route by Tuesday.',
        'tail': 'I take responsibility for checking the venue plan and confirming its step-free route by Tuesday.',
        'negations': ['We have abandoned buying the equipment; remove that purchase idea.',
                      'Sending messages to external recipients is prohibited.']},
}


def scenario_fixture(name):
    scenario = SCENARIOS[name]
    return {**scenario, 'body': '## Your notes\nSynthetic QA only. No external actions authorized.\n\n## Transcript\n' + scenario['transcript']}


class ApiFailure(RuntimeError):
    def __init__(self, method, path, status=None, category='http_status'):
        self.details = {'method': method, 'path': path, 'status': status, 'category': category,
                        'retryable': status in [429, 502, 503, 504] or category == 'transport'}
        super().__init__(f'{method} {path}: {category}' + (f' HTTP {status}' if status is not None else ''))


def failure_details(error):
    return error.details if isinstance(error, ApiFailure) else {'category': type(error).__name__, 'message': str(error)[:500]}


def evidence_path(scenario):
    if REPORTS.resolve() != REPORTS or not REPORTS.is_dir():
        raise RuntimeError('Evidence directory must be canonical')
    return REPORTS / ('isolated-meeting-http-' + time.strftime('%Y%m%d-%H%M%S', time.gmtime()) +
                      '-' + scenario + '-' + uuid.uuid4().hex[:8] + '.json')


def save_evidence(path, evidence):
    if path.parent != REPORTS or path.resolve() != path or not path.name.startswith('isolated-meeting-http-'):
        raise RuntimeError('Evidence path must be a canonical owned report file')
    # Never overwrite the earlier failed runs or follow a destination symlink.
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as handle:
        json.dump(evidence, handle, indent=2)
        handle.write('\n')


def api(path, body=None, expected=200):
    if not path.startswith('/') or path.startswith('//'):
        raise ValueError('Only relative isolated API paths are allowed')
    req = urllib.request.Request(isolated.URL + path,
        data=None if body is None else json.dumps(body).encode(),
        headers={'Content-Type': 'application/json', 'Authorization': 'Bearer ' + isolated.token()})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        with opener.open(req, timeout=25) as response:
            status = response.status
            raw = response.read(2 * 1024 * 1024 + 1)
    except urllib.error.HTTPError as error:
        status, raw = error.code, b''
        error.close()
    except (urllib.error.URLError, OSError):
        raise ApiFailure(req.get_method(), path, category='transport') from None
    if status != expected:
        raise ApiFailure(req.get_method(), path, status) from None
    if status >= 400:
        return {'httpStatus': status}  # No server error bodies in evidence.
    if len(raw) > 2 * 1024 * 1024:
        raise ApiFailure(req.get_method(), path, status, 'response_too_large')
    try:
        return json.loads(raw) if raw else None
    except (ValueError, UnicodeDecodeError):
        raise ApiFailure(req.get_method(), path, status, 'invalid_json') from None


def db_open():
    path = isolated.ROOT / 'spectral/permagent.db'
    if isolated.ROOT.resolve() != isolated.ROOT or path.resolve() != path or not path.is_file():
        raise RuntimeError('Isolated SQLite database is absent or noncanonical')
    db = sqlite3.connect(path.as_uri() + '?mode=ro', uri=True, timeout=5)
    db.row_factory = sqlite3.Row
    db.execute('PRAGMA query_only = ON')
    return db


def text_content(value):
    if isinstance(value, list):
        return '\n'.join(text_content(item) for item in value)
    if isinstance(value, dict):
        return str(value.get('text', '')) + '\n' + '\n'.join(text_content(v) for k, v in value.items() if k != 'text')
    return ''


def check(evidence, name, condition, details=None):
    evidence['checks'].append({'name': name, 'passed': bool(condition), 'details': details})


def owned_db_evidence(project_id, recording_id, receipt, tail=TAIL):
    with db_open() as db:
        triages = [dict(r) for r in db.execute(
            'SELECT id, status, attempt_count, writeup_provider, writeup_model, writeup_privacy, '
            'privacy_route, session_id, note_id FROM meeting_triage WHERE project_id=? AND recording_id=?',
            (project_id, recording_id))]
        session_id = receipt.get('sessionId')
        session = db.execute('SELECT id, name, session_type FROM sessions WHERE id=?', (session_id,)).fetchone()
        messages = db.execute('SELECT role, content_json FROM messages WHERE session_id=? ORDER BY id', (session_id,)).fetchall()
        parsed = [json.loads(r['content_json']) for r in messages]
        serialized = json.dumps(parsed)
        plain = '\n'.join(text_content(v) for v in parsed)
        key = f"note:{project_id}:{receipt['noteId']}"
        outbox = [dict(r) for r in db.execute('SELECT memory_key, source, status, attempts FROM brain_feed_outbox WHERE memory_key=?', (key,))]
    return {'triageRows': triages, 'session': dict(session) if session else None,
            'messageCount': len(messages), 'transcriptTailInSession': tail in plain,
            'toolContentPresent': any(t in serialized for t in ['"toolRequest"', '"toolResponse"', '"frontendToolRequest"']),
            'noteCatalogueOutbox': outbox, 'memoryKey': key}


def commitment_checks(cards):
    titles = [card['title'].lower() for card in cards]
    checklists = [i for i, title in enumerate(titles) if 'checklist' in title]
    routes = [i for i, title in enumerate(titles) if 'route' in title]
    return {'exactlyTwo': len(cards) == 2 and len(checklists) == 1 and len(routes) == 1 and
            checklists[0] != routes[0],
            'noCancelledOrExternalTask': all(not any(term in title for term in
                ['purchase', 'buy ', 'equipment', 'email', 'send ', 'contact ', 'external recipient']) for title in titles)}


def execute(evidence, timeout, scenario_name='original'):
    fixture = scenario_fixture(scenario_name)
    body, transcript, tail = fixture['body'], fixture['transcript'], fixture['tail']
    evidence['scenario'] = scenario_name
    isolated.manifest()  # Exact root/binary/model/library evidence from launcher.
    cfg = api('/config')['config']
    expected_provider = cfg.get('PERMAGENT_PACK_LOCAL_PROVIDER')
    expected_model = cfg.get('PERMAGENT_PACK_LOCAL_MODEL')
    if cfg.get('meeting_writeup_local_only') is not True or expected_provider != 'ollama' or not expected_model:
        raise RuntimeError('An explicitly configured local-only Ollama meeting model is required')
    evidence['configuredLocalRoute'] = {'provider': expected_provider, 'model': expected_model}
    run_id = uuid.uuid4().hex[:10]
    recording_id = 'synthetic-http-' + run_id
    evidence.update({'recordingId': recording_id, 'fixtureBody': body, 'projects': []})
    deadline = time.monotonic() + timeout
    for label in ['Amber Launch', 'Cobalt Venue']:
        project = api('/api/projects', {'name': f'Record HTTP QA {scenario_name} {label} {run_id}',
                      'slug': 'record-http-' + label.lower().replace(' ', '-') + '-' + run_id,
                      'description': 'Synthetic isolated meeting acceptance; no external actions.'}, expected=201)
        pid = project['id']
        base = '/api/projects/' + urllib.parse.quote(pid, safe='')
        columns = api(base + '/columns')
        if not columns:
            api(base + '/columns', {'name': 'To do', 'position': 0}, expected=201)
            columns = api(base + '/columns')
        payload = {'title': f'Synthetic {label} meeting', 'body': body, 'kind': 'meeting', 'recordingId': recording_id}
        saved = api(base + '/notes', payload)
        row = {'project': {'id': pid, 'name': project['name']}, 'base': base,
               'payload': payload, 'savedNoteId': saved['id'], 'initialReceipt': saved['triage'], 'columns': columns}
        evidence['projects'].append(row)
        replay = api(base + '/notes', payload)
        check(evidence, label + ': identical save deduplicates', replay['id'] == saved['id'] and
              replay['triage']['id'] == saved['triage']['id'] and replay['triage']['created'] is False)
        check(evidence, label + ': changed transcript conflicts',
              api(base + '/notes', {**payload, 'body': body + '\nChanged body must conflict.'}, expected=409)['httpStatus'] == 409)
    pending = list(evidence['projects'])
    while pending and time.monotonic() < deadline:
        for row in list(pending):
            path = row['base'] + '/recordings/' + recording_id + '/triage'
            try:
                receipt = api(path)
            except ApiFailure as error:
                row['lastPollError'] = failure_details(error)
                if not error.details['retryable']:
                    pending.remove(row)
                continue
            row['receipt'] = receipt
            if receipt['status'] in ['completed', 'failed', 'skipped']:
                pending.remove(row)
        if pending:
            time.sleep(3)
    for row in evidence['projects']:
        try:
            pid, base, receipt = row['project']['id'], row['base'], row.get('receipt', row['initialReceipt'])
            label = row['project']['name']
            check(evidence, label + ': triage completed', receipt['status'] == 'completed', receipt)
            check(evidence, label + ': local-only privacy', receipt.get('privacyRoute') == 'local_only')
            notes = api(base + '/notes')
            own = [n for n in notes if n['id'] == row['savedNoteId']]
            row['savedNotes'] = own
            check(evidence, label + ': one saved note with complete transcript', len(notes) == 1 and len(own) == 1 and transcript in own[0]['body'])
            check(evidence, label + ': typed notes retained and no truncation', len(own) == 1 and
                  'Synthetic QA only. No external actions authorized.' in own[0]['body'] and receipt.get('truncated') is False)
            check(evidence, label + ': final negation retained', len(own) == 1 and all(cue in own[0]['body'] for cue in fixture['negations']) and tail in own[0]['body'])
            cards = api(base + '/cards')
            row['cards'] = cards
            commitments = commitment_checks(cards)
            check(evidence, label + ': exactly two commitments', commitments['exactlyTwo'])
            check(evidence, label + ': no cancelled purchase/email task', commitments['noCancelledOrExternalTask'])
            first_column = min(row['columns'], key=lambda col: col['position'])['id']
            check(evidence, label + ': correct board and unexecuted cards', all(
                c['projectId'] == pid and c['cardType'] == 'standard' and c['columnId'] == first_column and
                c['assignedTo'] is None and c['archivedAt'] is None and c['createdBy'] == 'henry' and
                set(c['metadataJson']) == {'created_by_agent', 'source', 'source_note_id', 'source_triage_id',
                                           'source_item_key', 'source_recording_id'} for c in cards))
            check(evidence, label + ': source evidence links', all(
                c['metadataJson'].get('source_note_id') == row['savedNoteId'] and
                c['metadataJson'].get('source_triage_id') == receipt['id'] and
                c['metadataJson'].get('source_recording_id') == recording_id and
                c['metadataJson'].get('source') == 'meeting_triage' and
                c['metadataJson'].get('source_item_key') for c in cards))
            check(evidence, label + ': receipt/card identities agree', set(receipt['cardIds']) == {c['id'] for c in cards})
            if receipt.get('sessionId'):
                row['sessionApi'] = api(base + '/recordings/' + recording_id + '/triage/session')
                check(evidence, label + ': Orchestrator session project context', row['sessionApi']['projectId'] == pid and
                      row['sessionApi']['noteId'] == row['savedNoteId'] and row['sessionApi']['recordingId'] == recording_id and
                      bool(row['sessionApi']['orchestratorName']))
            row['databaseProof'] = owned_db_evidence(pid, recording_id, receipt, tail)
            proof = row['databaseProof']
            check(evidence, label + ': one durable triage row', len(proof['triageRows']) == 1)
            check(evidence, label + ': local model provenance', len(proof['triageRows']) == 1 and
                  proof['triageRows'][0]['writeup_provider'] == expected_provider and proof['triageRows'][0]['writeup_model'] == expected_model)
            check(evidence, label + ': hidden tool-free session received final transcript',
                  proof['session'] is not None and proof['session']['session_type'] == 'hidden' and
                  proof['transcriptTailInSession'] and not proof['toolContentPresent'])
            # APIs expose exact catalogue lookup. SQLite outbox is fallback durability evidence only.
            while True:
                try:
                    memory = api('/api/brain/memory?' + urllib.parse.urlencode({'key': proof['memoryKey']}))
                    row['catalogue'] = {'delivered': True, 'key': proof['memoryKey'], 'tailRetained': tail in json.dumps(memory)}
                    break
                except ApiFailure as error:
                    row['catalogueLastError'] = failure_details(error)
                    if time.monotonic() >= deadline:
                        break
                    time.sleep(3)
            row.setdefault('catalogue', {'delivered': False, 'key': proof['memoryKey'], 'durableOutbox': proof['noteCatalogueOutbox']})
            check(evidence, label + ': catalogue delivered full tail', row['catalogue'].get('delivered') and row['catalogue'].get('tailRetained'))
            replay = api(base + '/notes', row['payload'])
            retry = (api(base + '/recordings/' + recording_id + '/triage/retry', {})
                     if receipt['status'] == 'completed' else receipt)
            after = api(base + '/cards')
            check(evidence, label + ': completed replay/retry keeps cards stable', replay['id'] == row['savedNoteId'] and
                  retry['status'] == receipt['status'] and {c['id'] for c in after} == {c['id'] for c in cards})
            if receipt.get('sessionId') and receipt['status'] == 'completed':
                after_session = api(base + '/recordings/' + recording_id + '/triage/session')
                check(evidence, label + ': replay does not create another Orchestrator turn',
                      after_session['sessionId'] == row['sessionApi']['sessionId'] and
                      after_session['messageCount'] == row['sessionApi']['messageCount'])
            row.pop('payload')
        except Exception as error:
            row['inspectionError'] = failure_details(error)
            check(evidence, row['project']['name'] + ': inspection completed', False, row['inspectionError'])
            if 'databaseProof' not in row:
                try:
                    row['databaseProof'] = owned_db_evidence(row['project']['id'], recording_id,
                        row.get('receipt', row['initialReceipt']), tail)
                except Exception as database_error:
                    row['databaseProofError'] = failure_details(database_error)
    a, b = evidence['projects']
    check(evidence, 'same recording ID remains project-bound', a['savedNoteId'] != b['savedNoteId'] and
          a['initialReceipt']['id'] != b['initialReceipt']['id'] and
          not ({c['id'] for c in a.get('cards', [])} & {c['id'] for c in b.get('cards', [])}))
    active = api('/api/goals/active')
    # Never store unrelated isolated-user activity in this evidence.
    own_ids = {row['project']['id'] for row in evidence['projects']}
    active_rows = active if isinstance(active, list) else active.get('goals', [])
    matching = [g for g in active_rows if g.get('projectId', g.get('project_id')) in own_ids]
    check(evidence, 'no active goals launched for either synthetic project', not matching)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--execute', action='store_true', help='Required to create synthetic projects and meeting notes')
    parser.add_argument('--timeout', type=int, default=240)
    parser.add_argument('--scenario', choices=list(SCENARIOS), default='original')
    args = parser.parse_args()
    if not args.execute:
        print(json.dumps({'executeRequired': True, 'url': isolated.URL, 'scenario': args.scenario, 'fixture': scenario_fixture(args.scenario)['body'], 'projectCount': 2,
                          'method': 'HTTP backend only; no native/audio claim'}))
        return
    if not 240 <= args.timeout <= 600:
        parser.error('timeout must be 240..600 seconds')
    evidence = {'method': 'synthetic HTTP backend acceptance, NOT native capture', 'url': isolated.URL,
                'startedUtc': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), 'checks': [], 'tokenLogged': False}
    path = evidence_path(args.scenario)
    try:
        execute(evidence, args.timeout, args.scenario)
    except Exception as error:
        evidence['error'] = failure_details(error)
    evidence['passed'] = 'error' not in evidence and bool(evidence['checks']) and all(c['passed'] for c in evidence['checks'])
    save_evidence(path, evidence)
    print(json.dumps({'passed': evidence['passed'], 'evidence': str(path), 'checks': len(evidence['checks'])}))
    raise SystemExit(0 if evidence['passed'] else 1)

if __name__ == '__main__': main()
