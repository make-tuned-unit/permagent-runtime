"""--resume-root contract; no daemon, no network, temp directories only."""
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import blank_native_daemon as blank
import isolated_daemon as iso


class ResumeRootTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.target = Path(self.tmp.name).resolve() / 'target'
        self.root = self.target / 'record-qa-runtime'
        self.root.mkdir(parents=True, mode=0o700)
        os.chmod(self.root, 0o700)
        for name in ('home', 'logs', 'secrets'):
            (self.root / name).mkdir(mode=0o700)
        (self.root / 'config.yaml').write_text('{}')
        for p in (patch.object(iso, 'TARGET', self.target), patch.object(iso, 'ROOT', self.root),
                  patch.object(iso, 'ACTIVE', self.root / 'a.json'), patch.object(iso, 'STOP', self.root / 's.json'),
                  patch.object(iso, 'port_free', lambda: None)):
            p.start()
            self.addCleanup(p.stop)

    def test_fresh_path_unchanged_still_refuses_existing_state(self):
        with self.assertRaisesRegex(RuntimeError, 'fresh state'):
            blank.fresh_root()

    def test_non_canonical_path_refuses(self):
        other = self.target / 'other'
        other.mkdir()
        for bad in (str(other), str(self.root) + '/', str(self.root / '..' / 'record-qa-runtime'),
                    'record-qa-runtime', str(Path.home() / '.permagent'), '/Applications/Permagent.app'):
            with self.assertRaises(RuntimeError, msg=bad):
                iso.resume_root_check(bad)

    def test_symlink_to_canonical_root_refuses(self):
        link = self.target / 'link'
        link.symlink_to(self.root)
        with self.assertRaises(RuntimeError):
            iso.resume_root_check(str(link))

    def test_symlinked_component_refuses(self):
        real = Path(self.tmp.name).resolve() / 'real'
        real.mkdir()
        moved = real / 'record-qa-runtime'
        self.root.rename(moved)
        (self.target / 'record-qa-runtime').symlink_to(moved)
        with self.assertRaises(RuntimeError):
            iso.resume_root_check(str(self.root))

    def test_canonical_root_passes_and_skips_wipe(self):
        before = sorted(p.name for p in self.root.iterdir())
        with patch('builtins.print'):
            root, home = blank.fresh_root(str(self.root))
        self.assertEqual((root, home), (self.root, self.root / 'home'))
        self.assertEqual(sorted(p.name for p in self.root.iterdir()), before)
        self.assertTrue((self.root / 'config.yaml').exists())

    def test_live_daemon_refuses(self):
        (self.root / 'blank-native-daemon-1.json').write_text(json.dumps({'daemonPid': os.getppid()}))
        with self.assertRaisesRegex(RuntimeError, 'still alive'):
            iso.resume_root_check(str(self.root))

    def test_occupied_port_refuses(self):
        with patch.object(iso, 'port_free', side_effect=RuntimeError('Port 3002 is occupied')):
            with self.assertRaisesRegex(RuntimeError, 'occupied'):
                iso.resume_root_check(str(self.root))

    def test_sign_libs_independent_of_root(self):
        # sign-libs touches only <profile> dylibs, never the runtime root.
        with patch.object(iso, 'DEBUG', self.target / 'release'):
            with self.assertRaisesRegex(RuntimeError, 'dylibs are missing'):
                iso.libraries()


if __name__ == '__main__':
    unittest.main()
