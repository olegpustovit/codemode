import runpy
import unittest
from pathlib import Path

probe = runpy.run_path(str(Path(__file__).with_name('probe-integrations')))


class ProbeTests(unittest.TestCase):
    def test_one_click_requires_registration_pkce_and_secure_endpoints(self):
        metadata = {'registration_endpoint': 'https://provider.example/register',
                    'authorization_endpoint': 'https://provider.example/authorize',
                    'token_endpoint': 'https://provider.example/token',
                    'code_challenge_methods_supported': ['S256']}
        probe['one_click'](metadata)
        for field in ['registration_endpoint', 'code_challenge_methods_supported', 'token_endpoint']:
            with self.subTest(field=field), self.assertRaises(ValueError):
                probe['one_click']({k: v for k, v in metadata.items() if k != field})
        probe['one_click']({**metadata, 'registration_endpoint': None, 'client_id_metadata_document_supported': True})
        with self.assertRaises(ValueError):
            probe['one_click']({**metadata, 'token_endpoint': 'http://insecure.example/token'})

    def test_probe_sends_only_a_protocol_initialize_without_credentials(self):
        import io
        from unittest.mock import patch
        class Response(io.BytesIO):
            status = 200
            headers = {'content-type': 'application/json'}
        with patch('urllib.request.urlopen', return_value=Response(b'{"result": {}}')) as send:
            status, _, _ = probe['request']('https://provider.example/mcp', initialize=True)
            self.assertEqual(status, 200)
            request = send.call_args.args[0]
            import json
            payload = json.loads(request.data)
            self.assertEqual(payload['method'], 'initialize')
            self.assertEqual(request.full_url, 'https://provider.example/mcp')
            self.assertFalse(request.has_header('Authorization'))
            self.assertEqual(send.call_args.kwargs['timeout'], 8)
