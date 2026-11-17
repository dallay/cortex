"""Local-only diagnostic execution; never C1 acceptance."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

import retrieval_h5_local as local


class LocalTests(unittest.TestCase):
    def test_stage_records_real_bytes_and_rejects_existing_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / 'input'
            source.write_bytes(b'original')
            script = root / 'export.py'
            script.write_text("import hashlib,json,sys\nfrom pathlib import Path\n"
                              "print(json.dumps({'input_sha256':hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest()}))")
            output = root / 'attempt'
            record = local.run_stage(output, [sys.executable, str(script), str(source)],
                                     [source, script], source)
            self.assertEqual(record['input_sha256'], local.file_record(source)['sha256'])
            self.assertEqual(json.loads((output / 'stdout.json').read_bytes())['input_sha256'],
                             record['input_sha256'])
            self.assertEqual(record['network_policy'], 'sandbox-exec deny network*')
            with self.assertRaises(FileExistsError):
                local.run_stage(output, [sys.executable], [source], source)

    def test_valid_output_or_stage_receipt_drift_blocks_final_publication(self):
        for name in ('stdout.json', 'stage.json'):
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                source = root / 'input'
                source.write_bytes(b'original')
                program = ('import json,hashlib,sys; from pathlib import Path; '
                           'print(json.dumps({"input_sha256":hashlib.sha256(Path(sys.argv[1]).read_bytes()).hexdigest()}))')
                stage = local.run_stage(root / 'rust', [sys.executable, '-c', program, str(source)],
                                        [source], source)
                comparison = root / 'comparison.json'
                comparison.write_text('{}')
                receipt = {'stages': [stage], 'comparison': local.file_record(comparison)}
                original = (root / 'rust' / name).read_bytes()
                (root / 'rust' / name).write_bytes(original + b' ')
                with self.assertRaisesRegex(ValueError, 'evidence changed'):
                    local.verify_stages([stage])
                with self.assertRaisesRegex(ValueError, 'evidence changed'):
                    local.publish_receipt(root, receipt)
                self.assertFalse((root / 'local-diagnostic.json').exists())

    def test_output_channels_are_bounded_without_success_receipt(self):
        for channel in ('stdout', 'stderr'):
            with self.subTest(channel=channel), tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                source = root / 'input'
                source.write_bytes(b'original')
                program = ('import sys; sys.' + channel + '.write("x" * 131072)')
                with patch.object(local, 'CHANNEL_LIMIT', 4096):
                    with self.assertRaisesRegex(ValueError, 'channel limit'):
                        local.run_stage(root / 'attempt', [sys.executable, '-c', program], [source], source)
                self.assertFalse((root / 'attempt/stage.json').exists())
                self.assertFalse((root / 'local-diagnostic.json').exists())
                self.assertLessEqual((root / 'attempt' / ('stdout.json' if channel == 'stdout' else 'stderr.log')).stat().st_size, 4096)

    def test_hashing_does_not_read_whole_file(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve() / 'file'
            path.write_bytes(b'x' * 200000)
            with patch.object(Path, 'read_bytes', side_effect=AssertionError('unbounded read')):
                self.assertEqual(local.file_record(path)['size'], 200000)

    def test_bundle_receipt_bytes_must_match_pinned_id_before_loading(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            (root / '_muninn_artifacts.json').write_bytes(b'{}')
            lock = Path(__file__).resolve().parents[1] / 'fixtures/artifact-lock.json'
            with self.assertRaisesRegex(ValueError, 'bundle receipt'):
                local.verify_model(root, lock)

    def test_network_is_denied_in_child(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / 'input'
            source.write_bytes(b'original')
            program = ('import socket; s=socket.socket(); s.bind(("127.0.0.1", 0))')
            with self.assertRaisesRegex(ValueError, 'exporter exit'):
                local.run_stage(root / 'attempt', [sys.executable, '-c', program], [source], source)
            self.assertFalse((root / 'attempt/stage.json').exists())

    def test_bad_consumption_does_not_produce_success_record(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / 'input'
            source.write_bytes(b'original')
            output = root / 'attempt'
            with self.assertRaisesRegex(ValueError, 'consumption'):
                local.run_stage(output, [sys.executable, '-c', 'print("{}")'], [source], source)
            self.assertFalse((output / 'stage.json').exists())

    def test_changed_material_fails_before_success(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            source = root / 'input'
            source.write_bytes(b'original')
            output = root / 'attempt'
            program = ('import hashlib,json,sys; from pathlib import Path; p=Path(sys.argv[1]); '
                       'h=hashlib.sha256(p.read_bytes()).hexdigest(); p.write_bytes(b"changed"); '
                       'print(json.dumps({"input_sha256":h}))')
            with self.assertRaisesRegex(ValueError, 'material changed'):
                local.run_stage(output, [sys.executable, '-c', program, str(source)], [source], source)
            self.assertFalse((output / 'stage.json').exists())
