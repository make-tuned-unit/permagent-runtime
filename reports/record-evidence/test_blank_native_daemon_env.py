"""Pure environment-contract tests; never launch a daemon or contact Ollama."""

import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import blank_native_daemon as runner


class BlankNativeEnvironmentTests(unittest.TestCase):
    def test_default_denies_ollama_and_does_not_inherit_user_routes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'review-root'
            home = root / 'home'
            inherited = {
                'HOME': '/unrelated/home',
                'PERMAGENT_CONFIG': '/unrelated/config.yaml',
                'PERMAGENT_PATH_ROOT': '/unrelated/root',
                'GOOSE_PROVIDER': 'unrelated-provider',
                'GOOSE_MODEL': 'unrelated-model',
                'OPENAI_API_KEY': 'synthetic-key',
                'ANTHROPIC_API_KEY': 'synthetic-key',
                'HTTP_PROXY': 'http://unrelated.invalid',
                'OLLAMA_HOST': 'http://unrelated.invalid',
                'PERMAGENT_OLLAMA_HOST': 'http://unrelated.invalid',
            }
            with patch.dict(os.environ, inherited):
                env = runner.child_env(root, home)

            denied = f'http://127.0.0.1:{runner.OLLAMA_DENIED_PORT}'
            self.assertEqual(env['OLLAMA_HOST'], denied)
            self.assertEqual(env['PERMAGENT_OLLAMA_HOST'], denied)
            self.assertEqual(env['HOME'], str(home))
            self.assertEqual(env['PERMAGENT_PATH_ROOT'], str(root))
            self.assertEqual(env['PERMAGENT_CONFIG'], str(root / 'config.yaml'))
            self.assertEqual(env['PERMAGENT_SPECTRAL_DB'], str(root / 'spectral/permagent.db'))
            self.assertEqual(env['PERMAGENT_DISABLE_KEYRING'], '1')
            for key in ('GOOSE_PROVIDER', 'GOOSE_MODEL', 'OPENAI_API_KEY',
                        'ANTHROPIC_API_KEY', 'HTTP_PROXY'):
                self.assertNotIn(key, env)
            self.assertFalse(root.exists(), 'Building the environment must not seed config or data')

    def test_explicit_opt_in_only_changes_both_ollama_hosts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'review-root'
            home = root / 'home'
            denied = runner.child_env(root, home)
            allowed = runner.child_env(root, home, use_existing_local_ollama=True)

            changed = {key for key in denied if denied[key] != allowed[key]}
            self.assertEqual(changed, {'OLLAMA_HOST', 'PERMAGENT_OLLAMA_HOST'})
            self.assertEqual(allowed['OLLAMA_HOST'], 'http://127.0.0.1:11434')
            self.assertEqual(allowed['PERMAGENT_OLLAMA_HOST'], 'http://127.0.0.1:11434')
            self.assertFalse(any(key in allowed for key in (
                'GOOSE_PROVIDER', 'GOOSE_MODEL', 'PERMAGENT_MODEL', 'OLLAMA_MODEL',
                'OPENAI_API_KEY', 'ANTHROPIC_API_KEY',
            )))
            self.assertFalse(root.exists(), 'Opt-in must not seed config or select a model')


if __name__ == '__main__':
    unittest.main()
