import tempfile
from pathlib import Path
import unittest
from native_stage_paths import stage_paths, verify_versioned_ledger

class StagePathTests(unittest.TestCase):
    def test_default_paths_remain_unchanged(self):
        target, reports = Path('/owned/target'), Path('/owned/reports')
        self.assertEqual(stage_paths(target, reports, False),
                         (target / 'staged/Permagent World Review.app', reports / 'native-package-verification.json'))
        self.assertEqual(stage_paths(target, reports, True),
                         (target / 'staged/Permagent Record Review.app', reports / 'native-record-package-verification.json'))

    def test_r2_is_independent_from_r1_and_has_its_own_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            stage, report = stage_paths(root / 'target', root / 'reports', True, 'record-r2')
            self.assertEqual(stage, root / 'target/staged/Permagent Record Review (record-r2).app')
            self.assertEqual(report.name, 'native-record-package-verification-record-r2.json')
            self.assertNotEqual((stage, report), stage_paths(root / 'target', root / 'reports', True))
            self.assertFalse(stage.exists())
            self.assertFalse(report.exists())

    def test_rejects_traversal_absolute_unicode_and_nonrecord_modes(self):
        for label in ['', '../r1', '/tmp/app', 'record-../r1', 'record-r2/app', 'record-r2.app',
                      'record--r2', 'record-r2 ', 'record-r２', 'other-r2', 'record-' + 'x' * 40]:
            with self.subTest(label=label), self.assertRaises(ValueError):
                stage_paths(Path('/owned/target'), Path('/owned/reports'), True, label)
        with self.assertRaises(ValueError):
            stage_paths(Path('/owned/target'), Path('/owned/reports'), False, 'record-r2')

    def test_symlink_cannot_redirect_r2_to_old_bundle_or_ledger(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            target, reports = root / 'target', root / 'reports'
            (target / 'staged').mkdir(parents=True)
            reports.mkdir()
            old = target / 'staged/Permagent Record Review.app'
            old.mkdir()
            stage = target / 'staged/Permagent Record Review (record-r2).app'
            stage.symlink_to(old)
            with self.assertRaisesRegex(ValueError, 'symlink'): stage_paths(target, reports, True, 'record-r2')
            stage.unlink()
            old_report = reports / 'native-record-package-verification.json'
            old_report.write_text('{}')
            (reports / 'native-record-package-verification-record-r2.json').symlink_to(old_report)
            with self.assertRaisesRegex(ValueError, 'symlink'): stage_paths(target, reports, True, 'record-r2')

    def test_refresh_requires_exact_version_path_and_compiled_identity_ledger(self):
        stage = Path('/owned/staged/Permagent Record Review (record-r2).app')
        review_id = 'ai.permagent.record-review-20261007'
        good = {'stage_version': 'record-r2', 'staged_app': str(stage), 'record_review': True,
                'identifier': review_id, 'name': 'Permagent Record Review'}
        verify_versioned_ledger(good, 'record-r2', stage, review_id)
        for key, value in [('stage_version', 'record-r1'), ('staged_app', '/old/app'),
                           ('identifier', 'ai.permagent.other'), ('record_review', False), ('name', 'Other')]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                verify_versioned_ledger({**good, key: value}, 'record-r2', stage, review_id)
        # Historical default ledgers do not gain a new mandatory field.
        verify_versioned_ledger({}, None, stage, review_id)

if __name__ == '__main__': unittest.main()
