"""Runner GM-01: stdlib tests only; no build, pull, or service startup."""

import asyncio
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from urllib.error import HTTPError
from unittest.mock import Mock, patch

import gm01_runner as runner
from gm01_capture import RustBridgeAdapter
from gm01_differential import CaptureError


class ProvenanceTests(unittest.TestCase):
    def test_binary_identity_requires_elf64_little_endian_aarch64(self):
        header = bytearray(20)
        header[:4] = b'\x7fELF'
        header[4] = 2
        header[5] = 1
        header[18:20] = (183).to_bytes(2, 'little')
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / 'muninn'
            binary.write_bytes(header)
            self.assertEqual(runner.verify_linux_arm64_binary(binary),
                             'ELF64 little-endian AArch64')
            binary.write_bytes(b'\xcf\xfa\xed\xfe' + bytes(32))
            with self.assertRaisesRegex(RuntimeError, 'Linux arm64 ELF'):
                runner.verify_linux_arm64_binary(binary)

    def test_artifact_verification_rejects_binary_tampering(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / 'muninn'
            header = bytearray(20)
            header[:4] = b'\x7fELF'
            header[4] = 2
            header[5] = 1
            header[18:20] = (183).to_bytes(2, 'little')
            binary.write_bytes(header)
            manifest = {'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest()}
            artifact = runner.BuiltArtifact(binary, manifest)
            artifact.verify()
            binary.write_bytes(b'changed')
            with self.assertRaisesRegex(RuntimeError, 'binario'):
                artifact.verify()

    def test_service_environment_is_allowlist(self):
        with patch.dict(os.environ, {'NAM_BACKEND': 'nams', 'NEO4J_URI': 'external',
                                     'HTTPS_PROXY': 'external', 'OPENAI_API_KEY': 'secret'}):
            env = runner.clean_env()
        self.assertFalse(any(k.startswith(('NAM', 'NEO4J', 'OPENAI')) or 'PROXY' in k
                             for k in env))

    def test_evidence_directory_is_exclusive(self):
        with tempfile.TemporaryDirectory() as root:
            target = Path(root) / 'new'
            runner.reserve_evidence(target)
            with self.assertRaises(FileExistsError):
                runner.reserve_evidence(target)


class IsolationTests(unittest.TestCase):
    def test_database_descriptor_has_only_internal_dns_and_no_docker_lifecycle(self):
        database = runner.Database('bolt://rust-db:7687', 'private')
        self.assertEqual(database.uri, 'bolt://rust-db:7687')
        self.assertTrue(database.active)
        with patch.object(runner.subprocess, 'run') as command:
            database.close()
            command.assert_not_called()
        self.assertFalse(database.active)

    def test_fixture_cleanup_runs_even_when_process_termination_fails(self):
        fixture = runner.OwnedFixture(Mock(), Path('/private'),
                                      runner.Database('bolt://rust-db:7687', 'private'))
        fixture.process = Mock()
        fixture.process.terminate.side_effect = RuntimeError('stop')
        fixture.database = Mock()
        fixture.output = None
        with self.assertRaises(RuntimeError):
            fixture.close()
        fixture.database.close.assert_called_once_with()
        self.assertFalse(fixture.active)

    def test_reference_environment_restored_after_failure(self):
        with patch.dict(os.environ, {'NAM_BACKEND': 'nams', 'HTTPS_PROXY': 'external'}):
            previous = dict(os.environ)
            with self.assertRaises(RuntimeError):
                with runner.reference_environment(Path('/private')):
                    self.assertNotIn('NAM_BACKEND', os.environ)
                    self.assertNotIn('HTTPS_PROXY', os.environ)
                    raise RuntimeError('reference import')
            self.assertEqual(dict(os.environ), previous)

    def test_cli_rejects_external_binary_and_boolean_bypass(self):
        with patch.object(runner.argparse.ArgumentParser, 'error', side_effect=ValueError):
            with self.assertRaises(ValueError):
                runner.main(['--reference', '/ref', '--evidence', '/new', '--binary', '/bin/external'])
            with self.assertRaises(ValueError):
                runner.main(['--reference', '/ref', '--evidence', '/new', '--allow-writes'])

    def test_embedder_rejects_all_requests(self):
        for method, value in [('embed', 'text'), ('embed_batch', ['text'])]:
            with self.assertRaisesRegex(RuntimeError, 'prohíbe'):
                asyncio.run(getattr(runner.RejectEmbedder(), method)(value))

    def test_third_party_build_inputs_are_hashed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'Cargo.lock').write_text('lock')
            (root / 'Cargo.toml').write_text('workspace')
            (root / 'third_party/candle-core/src').mkdir(parents=True)
            path = root / 'third_party/candle-core/src/lib.rs'
            path.write_text('original')
            before = runner.source_hashes(root)
            path.write_text('changed')
            self.assertNotEqual(before, runner.source_hashes(root))


class CapabilityTests(unittest.TestCase):
    def test_generic_client_rejected_without_call(self):
        client = Mock()
        with self.assertRaises(CaptureError):
            asyncio.run(RustBridgeAdapter(client).add_message('s', 'user', 'c', {}))
        client.call.assert_not_called()

    def test_only_live_owned_fixture_can_write(self):
        fixture = runner.OwnedFixture(
            Mock(), Path('/private'), runner.Database('bolt://rust-db:7687', 'private'))
        fixture.process = Mock()
        fixture.process.poll.return_value = None
        fixture.call = Mock(return_value={'id': 'message'})
        adapter = RustBridgeAdapter(fixture)
        with self.assertRaises(CaptureError):
            asyncio.run(adapter.add_message('s', 'user', 'c', {}))
        fixture.active = True
        asyncio.run(adapter.add_message('s', 'user', 'c', {'x': 1}))
        options = fixture.call.call_args.kwargs
        self.assertIs(options['generate_embedding'], False)
        fixture.active = False
        with self.assertRaises(CaptureError):
            asyncio.run(adapter.add_message('s', 'user', 'c', {}))


class OwnedFixtureHttpTests(unittest.TestCase):
    class Response:
        def __init__(self, status, payload):
            self.status = status
            self.payload = payload

        def __enter__(self):
            return self

        def __exit__(self, *args):
            return False

        def read(self):
            return self.payload

    def live_fixture(self):
        fixture = runner.OwnedFixture(
            Mock(), Path('/private'), runner.Database('bolt://rust-db:7687', 'private'))
        fixture.active = True
        fixture.process = Mock()
        fixture.process.poll.return_value = None
        fixture.base_url = 'http://bridge'
        return fixture

    def test_add_message_accepts_the_raw_wire_message_json_response(self):
        message = {
            'id': 'message-1', 'conversation_id': 'conversation-1', 'role': 'user',
            'content': 'hello', 'timestamp': '2026-11-02T12:00:00+00:00',
            'embedding': None, 'metadata': {'source': 'gm01'},
        }
        fixture = self.live_fixture()
        response = self.Response(200, json.dumps(message).encode())
        with patch.object(runner, 'urlopen', return_value=response) as urlopen:
            result = fixture.call('add_message', session_id='session-a', role='user',
                                  content='hello', metadata={'source': 'gm01'},
                                  generate_embedding=False)
        self.assertEqual(result, message)
        request = urlopen.call_args.args[0]
        self.assertEqual(request.full_url, 'http://bridge/add_message')
        self.assertEqual(request.get_method(), 'POST')
        self.assertEqual(json.loads(request.data)['generate_embedding'], False)

    def test_request_handles_no_content_without_decoding_json(self):
        fixture = self.live_fixture()
        response = self.Response(204, b'')
        with patch.object(runner, 'urlopen', return_value=response):
            self.assertEqual(fixture._request('clear_session', {'session_id': 's'}), (204, None))

    def test_get_conversation_returns_raw_wire_json_from_http_request(self):
        conversation = {
            'id': 'conversation-1', 'session_id': 'session-a', 'title': None,
            'created_at': '2026-11-02T12:00:00+00:00', 'updated_at': None,
            'messages': [{
                'id': 'message-1', 'conversation_id': 'conversation-1', 'role': 'user',
                'content': 'hello', 'timestamp': '2026-11-02T12:00:00+00:00',
                'embedding': None, 'metadata': {'source': 'gm01'},
            }],
        }
        fixture = self.live_fixture()
        response = self.Response(200, json.dumps(conversation).encode())
        with patch.object(runner, 'urlopen', return_value=response) as urlopen:
            result = asyncio.run(RustBridgeAdapter(fixture).get_conversation('session-a'))
        self.assertEqual(result, {
            'id': 'conversation-1', 'session_id': 'session-a',
            'messages': [{
                'id': 'message-1', 'conversation_id': 'conversation-1', 'role': 'user',
                'content': 'hello', 'metadata': {'source': 'gm01'},
                'timestamp': '2026-11-02T12:00:00+00:00',
            }],
        })
        request = urlopen.call_args.args[0]
        self.assertEqual(request.full_url, 'http://bridge/get_conversation')
        self.assertEqual(json.loads(request.data), {'session_id': 'session-a'})

    def test_json_call_rejects_empty_or_non_object_success_responses(self):
        for status, body in ((204, b''), (200, b''), (200, b'null'), (200, b'[]')):
            with self.subTest(status=status, body=body):
                fixture = self.live_fixture()
                response = self.Response(status, body)
                with patch.object(runner, 'urlopen', return_value=response):
                    with self.assertRaisesRegex(RuntimeError, '^Bridge devolvió error$'):
                        fixture.call('get_conversation', session_id='session-a')

    def test_http_error_is_kept_as_a_sanitized_bridge_error(self):
        fixture = self.live_fixture()
        error = HTTPError('http://bridge/get_conversation', 503, 'unavailable', {}, None)
        with patch.object(runner, 'urlopen', side_effect=error):
            with self.assertRaisesRegex(RuntimeError, '^Bridge devolvió error$'):
                fixture.call('get_conversation', session_id='session-a')


if __name__ == '__main__':
    unittest.main()
