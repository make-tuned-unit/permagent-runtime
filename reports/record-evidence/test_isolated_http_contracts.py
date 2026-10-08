"""Pure helpers/fixture guards only: no daemon/API/audio operations."""
import json
import urllib.error
from io import BytesIO
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch
import test_isolated_meeting_http as meeting
import test_isolated_whisper_http as whisper

class AcceptanceContractTests(unittest.TestCase):
    def test_existing_synthetic_scripts_satisfy_semantic_cues(self):
        manifest = json.loads((whisper.HERE / 'manifest.json').read_text())
        cues = {cue['id']: cue for cue in manifest['cues']}
        for key in whisper.RULES:
            self.assertTrue(all(whisper.evaluate(key, cues[key]['script']).values()), key)

    def test_lost_negation_or_final_tail_fails(self):
        text = 'I will draft the checklist by Friday. The launch is approved. Send any email.'
        result = whisper.evaluate('mic_open', text)
        self.assertTrue(result['commitment'])
        self.assertFalse(result['launch_negation'])
        self.assertFalse(result['no_email'])
        self.assertFalse(whisper.evaluate('far_tail', 'Final commitment: confirm the venue')['venue_tail'])

    def test_meeting_body_matches_ui_delimiters_and_retains_tail(self):
        self.assertTrue(meeting.BODY.startswith('## Your notes\n'))
        self.assertIn('\n\n## Transcript\n', meeting.BODY)
        self.assertTrue(meeting.BODY.endswith(meeting.TAIL))
        self.assertIn('Do not send any email.', meeting.BODY)

    def test_session_text_extraction_reads_nested_content(self):
        messages = [{'type': 'text', 'text': 'prefix'}, {'type': 'text', 'text': meeting.TAIL}]
        self.assertIn(meeting.TAIL, meeting.text_content(messages))

    def test_api_rejects_external_absolute_urls_before_token_read(self):
        with patch.object(meeting.isolated, 'token', side_effect=AssertionError('must not read token')):
            for path in ['http://127.0.0.1:3001/config', '//example.com/config']:
                with self.assertRaises(ValueError): meeting.api(path)

    def test_sqlite_connection_is_readonly(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / 'spectral').mkdir()
            database = root / 'spectral/permagent.db'
            with sqlite3.connect(database) as db:
                db.execute('CREATE TABLE fixture (value TEXT)')
            with patch.object(meeting.isolated, 'ROOT', root), meeting.db_open() as db:
                self.assertEqual(db.execute('SELECT COUNT(*) FROM fixture').fetchone()[0], 0)
                with self.assertRaises(sqlite3.OperationalError):
                    db.execute("INSERT INTO fixture VALUES ('forbidden')")

    def test_all_scenarios_keep_both_internal_commitments_and_scoped_restrictions(self):
        bodies = set()
        for name in meeting.SCENARIOS:
            fixture = meeting.scenario_fixture(name)
            bodies.add(fixture['body'])
            self.assertIn('checklist', fixture['transcript'])
            self.assertIn('step-free route', fixture['tail'])
            self.assertTrue(fixture['body'].endswith(fixture['tail']))
            self.assertTrue(all(cue in fixture['body'] for cue in fixture['negations']))
        self.assertEqual(len(bodies), 3)
        self.assertEqual(meeting.scenario_fixture('original')['body'], meeting.BODY)

    def test_two_distinct_commitments_rejects_missing_combined_and_extra_tasks(self):
        good = [{'title': 'Draft the internal checklist'}, {'title': 'Confirm step-free route'}]
        self.assertTrue(all(meeting.commitment_checks(good).values()))
        self.assertFalse(meeting.commitment_checks(good[:1])['exactlyTwo'])
        self.assertFalse(meeting.commitment_checks([{'title': 'Checklist and route'}, {'title': 'Unrelated'}])['exactlyTwo'])
        self.assertFalse(meeting.commitment_checks(good + [{'title': 'Buy equipment'}])['exactlyTwo'])
        self.assertFalse(meeting.commitment_checks([{'title': 'Send email'}])['noCancelledOrExternalTask'])

    def test_503_and_transport_evidence_never_include_bearer_or_error_body(self):
        for failure in [urllib.error.HTTPError('http://127.0.0.1:3002/config', 503, 'unavailable', {},
                                               BytesIO(b'private error body')),
                        urllib.error.URLError('private transport detail')]:
            with patch.object(meeting.isolated, 'token', return_value='synthetic-private-bearer'), \
                    patch.object(meeting.urllib.request, 'build_opener') as factory:
                factory.return_value.open.side_effect = failure
                with self.assertRaises(meeting.ApiFailure) as caught: meeting.api('/config')
                detail = meeting.failure_details(caught.exception)
                self.assertTrue(detail['retryable'])
                serialized = json.dumps(detail)
                self.assertNotIn('private', serialized)
                self.assertEqual(detail['path'], '/config')

    def test_evidence_writer_preserves_previous_results_and_rejects_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            with patch.object(meeting, 'REPORTS', root):
                path = meeting.evidence_path('paraphrase-a')
                meeting.save_evidence(path, {'old': 'retained'})
                with self.assertRaises(FileExistsError): meeting.save_evidence(path, {'new': 'must not overwrite'})
                self.assertEqual(json.loads(path.read_text()), {'old': 'retained'})
                link = root / 'isolated-meeting-http-symlink.json'
                link.symlink_to(path)
                with self.assertRaisesRegex(RuntimeError, 'canonical'): meeting.save_evidence(link, {})
                self.assertNotEqual(meeting.evidence_path('original'), meeting.evidence_path('original'))

    def test_database_symlink_rejected_before_readonly_connection(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / 'spectral').mkdir()
            other = root / 'other.db'
            other.write_bytes(b'synthetic')
            (root / 'spectral/permagent.db').symlink_to(other)
            with patch.object(meeting.isolated, 'ROOT', root):
                with self.assertRaisesRegex(RuntimeError, 'noncanonical'): meeting.db_open()

    def test_503_inspection_retains_results_for_both_projects(self):
        projects = []
        def fake_api(path, body=None, expected=200):
            if path == '/config':
                return {'config': {'meeting_writeup_local_only': True,
                                  'PERMAGENT_PACK_LOCAL_PROVIDER': 'ollama', 'PERMAGENT_PACK_LOCAL_MODEL': 'fixture-model'}}
            if path == '/api/projects':
                project = {'id': 'p' + str(len(projects)), 'name': body['name']}
                projects.append(project)
                return project
            if path == '/api/goals/active': return {'goals': []}
            pid = path.split('/')[3]
            receipt = {'id': 't-' + pid, 'noteId': 'n-' + pid, 'status': 'completed',
                       'privacyRoute': 'local_only', 'truncated': False, 'cardIds': [], 'created': False}
            if path.endswith('/columns'): return [{'id': 'c-' + pid, 'position': 0}]
            if path.endswith('/triage'): return receipt
            if path.endswith('/notes'):
                if body is None: raise meeting.ApiFailure('GET', path, 503)
                if expected == 409: return {'httpStatus': 409}
                return {'id': 'n-' + pid, 'triage': receipt}
            raise AssertionError(path)
        evidence = {'checks': []}
        with patch.object(meeting.isolated, 'manifest'), patch.object(meeting, 'api', side_effect=fake_api), \
                patch.object(meeting, 'owned_db_evidence', return_value={'readonlyFallback': True}):
            meeting.execute(evidence, 240, 'paraphrase-b')
        self.assertEqual(len(evidence['projects']), 2)
        self.assertTrue(all(row['inspectionError']['status'] == 503 for row in evidence['projects']))
        self.assertTrue(all(row['databaseProof']['readonlyFallback'] for row in evidence['projects']))
        self.assertEqual(evidence['configuredLocalRoute']['model'], 'fixture-model')
        self.assertGreaterEqual(len(evidence['checks']), 10)

if __name__ == '__main__': unittest.main()
