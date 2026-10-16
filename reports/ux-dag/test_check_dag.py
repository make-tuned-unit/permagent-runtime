import tempfile
import unittest
from pathlib import Path
from check_dag import validate

class DagTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.base = Path(self.temp.name)
        (self.base / 'receipt.md').write_text('Verified by a focused test')
        self.doc = {'nodes': [
            {'id': 'A', 'depends_on': [], 'status': 'verified', 'acceptance': ['a'], 'evidence': ['receipt.md']},
            {'id': 'B', 'depends_on': ['A'], 'status': 'pending', 'acceptance': ['b'], 'evidence': []},
        ]}
    def tearDown(self):
        self.temp.cleanup()
    def test_dependency_order(self):
        self.assertEqual(validate(self.doc, self.base), ['A', 'B'])
    def test_cycle(self):
        self.doc['nodes'][0]['depends_on'] = ['B']
        with self.assertRaisesRegex(ValueError, 'Cycle'):
            validate(self.doc, self.base)
    def test_unknown_dependency(self):
        self.doc['nodes'][1]['depends_on'] = ['missing']
        with self.assertRaisesRegex(ValueError, 'Unknown'):
            validate(self.doc, self.base)
    def test_no_completion_without_evidence(self):
        self.doc['nodes'][1]['status'] = 'verified'
        with self.assertRaisesRegex(ValueError, 'No verification'):
            validate(self.doc, self.base)
    def test_no_completion_before_dependency(self):
        self.doc['nodes'][0]['status'] = 'pending'
        self.doc['nodes'][1].update(status='verified', evidence=['receipt.md'])
        with self.assertRaisesRegex(ValueError, 'before'):
            validate(self.doc, self.base)
    def test_missing_receipt(self):
        self.doc['nodes'][0]['evidence'] = ['absent.md']
        with self.assertRaisesRegex(ValueError, 'Missing evidence'):
            validate(self.doc, self.base)

if __name__ == '__main__':
    unittest.main()
