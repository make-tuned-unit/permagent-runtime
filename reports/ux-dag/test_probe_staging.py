import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from probe_staging import main, probe, request


class ProbeTests(unittest.TestCase):
    def setUp(self):
        self.state = {'projectId': '00000000-0000-4000-8000-000000000001',
                      'personId': '00000000-0000-4000-8000-000000000002',
                      'token': 'test-only-token'}
        self.project = {'name': 'UX HTTP fixture test', 'status': 'paused'}
        self.person = {'entity_uuid': self.state['personId'], 'display_name': 'UX fixture test'}
        self.calls = []
        self.now = 0

    def call(self, path, token, method='GET', body=None):
        self.calls.append((method, path, body))
        if method == 'PATCH':
            if path.endswith('/fields'):
                self.person.update(body['fields'])
            else:
                self.project.update(body)
        if path == '/api/people/directory':
            return [self.person]
        if path == '/api/projects/' + self.state['projectId']:
            return self.project
        return {}

    def pause(self, duration):
        self.now += duration

    def run_probe(self, call=None):
        return probe(self.state, 3, call=call or self.call,
                     clock=lambda: self.now, pause=self.pause)

    def test_probes_all_surfaces_and_verifies_owned_record_writes(self):
        result = self.run_probe()
        self.assertEqual(result['read_requests'], 12)
        self.assertEqual(result['elapsed_seconds'], 3)
        self.assertTrue(result['persistence_verified'])
        writes = [(path, body) for method, path, body in self.calls if method == 'PATCH']
        self.assertEqual(writes, [
            ('/api/projects/' + self.state['projectId'], {'status': 'active'}),
            ('/api/people/' + self.state['personId'] + '/fields',
             {'fields': {'notes': 'UX browser wiring verified'}}),
        ])

    def test_foreign_project_is_rejected_before_writes(self):
        self.project['name'] = 'Real project'
        with self.assertRaisesRegex(RuntimeError, 'Refusing a project'):
            self.run_probe()
        self.assertFalse(any(method == 'PATCH' for method, _, _ in self.calls))

    def test_foreign_person_is_rejected_before_writes(self):
        self.person['display_name'] = 'Real person'
        with self.assertRaisesRegex(RuntimeError, 'Refusing a person'):
            self.run_probe()
        self.assertFalse(any(method == 'PATCH' for method, _, _ in self.calls))

    def test_read_timeout_fails_without_completion_writes(self):
        def timeout(path, *args, **kwargs):
            if path.endswith('/stack/detected'):
                raise TimeoutError('stalled read')
            return self.call(path, *args, **kwargs)
        with self.assertRaises(TimeoutError):
            self.run_probe(timeout)
        self.assertFalse(any(method == 'PATCH' for method, _, _ in self.calls))

    def test_polling_does_not_start_reads_after_deadline(self):
        def slow_read(path, token, method='GET', body=None):
            result = self.call(path, token, method, body)
            self.now += 0.6
            return result
        self.run_probe(slow_read)
        first_write = next(i for i, (method, _, _) in enumerate(self.calls)
                           if method == 'PATCH')
        polling = [path for method, path, _ in self.calls[2:first_write]
                   if method == 'GET']
        # Two identity reads take 1.2 seconds before the three-second polling
        # window starts. Five 0.6-second reads then reach the deadline.
        self.assertEqual(len(polling), 5)

    def test_unpersisted_edit_is_not_success(self):
        def dropped(path, token, method='GET', body=None):
            if method == 'PATCH':
                return {}
            return self.call(path, token)
        with self.assertRaisesRegex(RuntimeError, 'did not persist'):
            self.run_probe(dropped)

    def test_failed_probe_never_releases_the_daemon_acceptance_gate(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root) / 'ux-browser-test'
            directory.mkdir()
            state = directory / 'state.json'
            state.write_text(json.dumps(self.state))
            state.chmod(0o600)
            with patch('probe_staging.REVIEW_ROOT', Path(root).resolve()), \
                    patch('sys.argv', ['probe_staging.py', str(directory)]), \
                    patch('probe_staging.probe', side_effect=TimeoutError('stalled runtime')):
                with self.assertRaises(TimeoutError):
                    main()
            self.assertFalse((directory / 'finish').exists())

    def test_nonprivate_credential_is_rejected_before_any_request(self):
        with tempfile.TemporaryDirectory() as root:
            directory = Path(root) / 'ux-browser-test'
            directory.mkdir()
            state = directory / 'state.json'
            state.write_text(json.dumps(self.state))
            state.chmod(0o644)
            with patch('probe_staging.REVIEW_ROOT', Path(root).resolve()), \
                    patch('sys.argv', ['probe_staging.py', str(directory)]), \
                    patch('probe_staging.probe') as run:
                with self.assertRaisesRegex(RuntimeError, 'private'):
                    main()
                run.assert_not_called()

    def test_missing_token_is_rejected_before_any_request(self):
        state = {**self.state, 'token': ''}
        with self.assertRaisesRegex(RuntimeError, 'bearer token'):
            probe(state, 1, call=self.call, clock=lambda: self.now,
                  pause=self.pause)
        self.assertEqual(self.calls, [])

    def test_symlink_review_directory_is_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / 'ux-browser-real'
            target.mkdir()
            link = Path(root) / 'ux-browser-link'
            link.symlink_to(target, target_is_directory=True)
            with patch('probe_staging.REVIEW_ROOT', Path(root).resolve()), \
                    patch('sys.argv', ['probe_staging.py', str(link)]), \
                    patch('probe_staging.probe') as run:
                with self.assertRaisesRegex(RuntimeError, 'symlink'):
                    main()
                run.assert_not_called()

    @patch('probe_staging.http.client.HTTPConnection')
    def test_requests_are_timeout_bounded_on_staging_only(self, connection):
        response = connection.return_value.getresponse.return_value
        response.status, response.read.return_value = 200, b'{}'
        self.assertEqual(request('/api/people/directory', 'test-only-token'), {})
        connection.assert_called_once_with('127.0.0.1', 3002, timeout=3)
        connection.return_value.close.assert_called_once()

    @patch('probe_staging.http.client.HTTPConnection')
    def test_redirect_or_error_is_rejected_and_connection_closed(self, connection):
        response = connection.return_value.getresponse.return_value
        response.status, response.read.return_value = 302, b''
        with self.assertRaisesRegex(RuntimeError, 'HTTP 302') as raised:
            request('/api/people/directory', 'test-only-token')
        self.assertNotIn('test-only-token', str(raised.exception))
        connection.return_value.close.assert_called_once()

    @patch('probe_staging.http.client.HTTPConnection')
    def test_timeout_identifies_surface_without_echoing_credentials(self, connection):
        connection.return_value.getresponse.side_effect = TimeoutError('test-only-token')
        with self.assertRaisesRegex(RuntimeError, 'GET /api/people/directory: TimeoutError') as raised:
            request('/api/people/directory', 'test-only-token')
        self.assertNotIn('test-only-token', str(raised.exception))
        connection.return_value.close.assert_called_once()

    @patch('probe_staging.http.client.HTTPConnection')
    def test_error_response_body_is_not_echoed(self, connection):
        response = connection.return_value.getresponse.return_value
        response.status, response.read.return_value = 500, b'test-only-token private detail'
        with self.assertRaisesRegex(RuntimeError, 'HTTP 500') as raised:
            request('/api/people/directory', 'test-only-token')
        self.assertNotIn('test-only-token', str(raised.exception))
        self.assertNotIn('private detail', str(raised.exception))


if __name__ == '__main__':
    unittest.main()
