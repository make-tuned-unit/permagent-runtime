import hashlib
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch
import repair_old_note_associations as repair


class RepairTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve() / 'record-qa-runtime'
        (self.root / 'spectral').mkdir(parents=True)
        (self.root / 'brain').mkdir()
        self.targets = repair.original_payloads()
        with sqlite3.connect(self.root / 'spectral/permagent.db') as db:
            db.executescript('''CREATE TABLE meeting_triage(id TEXT, status TEXT, attempt_count INT, card_ids_json TEXT, note_id TEXT, project_id TEXT, recording_id TEXT, body_hash TEXT, session_id TEXT);
                CREATE TABLE project_notes(id TEXT, project_id TEXT, body TEXT);
                CREATE TABLE cards(id TEXT, project_id TEXT, title TEXT);
                CREATE TABLE messages(id INT, session_id TEXT, content_json TEXT);
                CREATE TABLE project_memories(project_id TEXT, memory_id TEXT);
                CREATE TABLE brain_feed_outbox(memory_key TEXT, status TEXT, attempts INT);''')
            for target in self.targets:
                prior, payload = target['baseline'], target['payload']
                vals = [prior[k] for k in ['id','status','attempt_count','card_ids_json','note_id','project_id']]
                vals += [payload['recordingId'], hashlib.sha256(payload['body'].strip().encode()).hexdigest(), 's-' + prior['id']]
                db.execute('INSERT INTO meeting_triage VALUES (?,?,?,?,?,?,?,?,?)', vals)
                db.execute('INSERT INTO project_notes VALUES (?,?,?)', (prior['note_id'], prior['project_id'], 'enhanced original note'))
                for card in json.loads(prior['card_ids_json']):
                    db.execute('INSERT INTO cards VALUES (?,?,?)',(card, prior['project_id'], 'original task'))
        with sqlite3.connect(self.root / 'brain/memory.db') as db:
            db.execute('CREATE TABLE memories(id TEXT, key TEXT, content TEXT)')
            for target in self.targets:
                prior = target['baseline']
                db.execute('INSERT INTO memories VALUES (?,?,?)',('mem-' + prior['note_id'], f"note:{prior['project_id']}:{prior['note_id']}", 'enhanced original note'))
        self.patch = patch.object(repair.isolated, 'ROOT', self.root)
        self.patch.start(); self.addCleanup(self.patch.stop)

    def test_only_three_original_payloads_verified_and_snapshot_is_readonly(self):
        with patch.object(repair.http, 'api', side_effect=AssertionError('No API permitted')):
            before = repair.snapshot(self.targets)
            self.assertEqual(len(before), 3)
            self.assertEqual(sum(len(row['cardIds']) for row in before), 4)
            self.assertTrue(all(row['associationCount'] == 0 for row in before))
            self.assertEqual(before, repair.snapshot(self.targets))

    def test_changed_original_body_hash_or_extra_note_is_rejected(self):
        with sqlite3.connect(self.root / 'spectral/permagent.db') as db:
            db.execute("UPDATE meeting_triage SET body_hash='wrong'")
        with self.assertRaisesRegex(RuntimeError, 'body hash'):
            repair.snapshot(self.targets)

    def test_repair_links_do_not_change_stable_state_but_card_edits_do(self):
        before = repair.snapshot(self.targets)
        with sqlite3.connect(self.root / 'spectral/permagent.db') as db:
            for target in self.targets:
                p = target['baseline']
                db.execute('INSERT INTO project_memories VALUES (?,?)', (p['project_id'], 'mem-' + p['note_id']))
        after = repair.snapshot(self.targets)
        self.assertTrue(all(row['associationCount'] == 1 for row in after))
        self.assertEqual([r['stableStateSha256'] for r in before], [r['stableStateSha256'] for r in after])
        with sqlite3.connect(self.root / 'spectral/permagent.db') as db:
            db.execute("UPDATE cards SET title='unexpected changed task'")
        changed = repair.snapshot(self.targets)
        self.assertNotEqual([r['stableStateSha256'] for r in after], [r['stableStateSha256'] for r in changed])


if __name__ == '__main__': unittest.main()
