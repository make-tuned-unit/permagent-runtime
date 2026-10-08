import io
import os
from pathlib import Path
import tempfile
import unittest
from contextlib import redirect_stdout
from unittest.mock import patch
import isolated_browser as browser

class BrowserHelperTests(unittest.TestCase):
    def test_pairing_uses_legitimate_api_and_returns_only_private_file_path(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            output = io.StringIO()
            code = 'a' * 32
            with patch.object(browser.isolated, 'ROOT', root), patch.object(browser.isolated, 'manifest'), \
                    patch.object(browser.isolated, 'token', return_value='synthetic-bearer'), \
                    patch.object(browser.isolated, 'request', return_value={'claim_code': code}) as request, \
                    patch.object(browser.socket, 'create_connection'), redirect_stdout(output):
                browser.pair_link()
            path = Path(output.getvalue().strip())
            self.assertEqual(path.parent, root / 'browser-pairing')
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(path.parent.stat().st_mode & 0o777, 0o700)
            self.assertEqual(path.read_text(), browser.BROWSER_URL + '#claim=' + code + '\n')
            self.assertNotIn(code, output.getvalue())
            self.assertNotIn('synthetic-bearer', output.getvalue())
            self.assertEqual(request.call_args.args[0], 'http://127.0.0.1:3002/api/devices/pair')

    def test_clean_frontend_environment_cannot_inherit_daemon_or_debug_redirect(self):
        with patch.dict(os.environ, {'VITE_DAEMON_URL': 'http://127.0.0.1:3001',
                                      'DEBUG': '*', 'OPENAI_API_KEY': 'synthetic'}):
            env = browser.clean_env()
        self.assertNotIn('VITE_DAEMON_URL', env)
        self.assertNotIn('DEBUG', env)
        self.assertNotIn('OPENAI_API_KEY', env)
        self.assertEqual(env['HOME'], os.environ['HOME'])

    def test_pinned_node_does_not_follow_inherited_path(self):
        with patch.dict(os.environ, {'PATH': '/untrusted/path', 'NODE_OPTIONS': '--inspect'}):
            command = browser.frontend_command()
            env = browser.clean_env()
        self.assertEqual(command[0], '/Users/j/.local/share/mise/installs/node/24.19.0/bin/node')
        self.assertTrue(Path(command[0]).is_file())
        self.assertEqual(command[0], str(Path(command[0]).resolve()))
        self.assertTrue(env['PATH'].startswith(str(browser.NODE.parent) + ':'))
        self.assertNotIn('/untrusted/path', env['PATH'])
        self.assertEqual(env['NODE_OPTIONS'], '--max-old-space-size=1024')

    def test_prerequisite_errors_distinguish_node_from_config(self):
        with patch.object(browser, 'NODE', Path('/missing/node')):
            with self.assertRaisesRegex(RuntimeError, 'Trusted Node executable missing'):
                browser.frontend_command()
        with patch.object(browser, 'CONFIG', Path('/missing/vite.config.mjs')):
            with self.assertRaisesRegex(RuntimeError, 'Isolated Vite config missing'):
                browser.frontend_command()

    def test_moving_node_alias_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory).resolve() / 'node-version'
            executable.write_text('fixture')
            executable.chmod(0o700)
            alias = executable.with_name('node')
            alias.symlink_to(executable)
            with patch.object(browser, 'NODE', alias):
                with self.assertRaisesRegex(RuntimeError, 'redirected'):
                    browser.frontend_command()

if __name__ == '__main__': unittest.main()
