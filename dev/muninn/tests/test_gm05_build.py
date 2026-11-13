"""Build invocation is mocked; source staging and hashes use real temporary files."""
import hashlib
import importlib
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


class BuildTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='gm05-build-test-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name) / 'source'
        self.root.mkdir()
        for name, text in {
            'Cargo.toml': '[workspace]\nmembers = ["apps/muninn"]\n',
            'Cargo.lock': 'lock', 'rust-toolchain.toml': '[toolchain]\nchannel = "1.99"\n',
            'apps/muninn/Cargo.toml': '[package]\nname = "muninn"\nversion = "0.1.0"\n',
            'apps/muninn/src/main.rs': 'fn main() {} // dirty bytes',
            'apps/muninn/src/new.rs': '// untracked source',
            'apps/muninn/src/resource.rs': 'const DATA: &str = include_str!("../../../dev/fixtures/asset.json");',
            'dev/fixtures/asset.json': '{"embedded": true}',
            'crates/demo/data/schema.json': '{"resource": true}',
            'third_party/demo/build.rs': '// build input',
            'third_party/demo/data.bin': 'embedded resource',
        }.items():
            self.put(name, text)
        self.build = importlib.import_module('gm05_build')
        self.commands = []

    def put(self, name, text):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        return path

    def invoke(self, command, **kwargs):
        self.commands.append((command, kwargs))
        if '--version' in command or '-Vv' in command:
            return SimpleNamespace(returncode=0, stdout='trusted toolchain version\n')
        snapshot = Path(kwargs['cwd'])
        self.assertNotEqual(snapshot, self.root)
        self.assertNotIn(str(self.root), ' '.join(command))
        for name in ('apps/muninn/src/new.rs', 'dev/fixtures/asset.json', 'crates/demo/data/schema.json',
                     'third_party/demo/data.bin'):
            self.assertEqual((snapshot / name).read_bytes(), (self.root / name).read_bytes())
        target = Path(command[command.index('--target-dir') + 1])
        self.assertFalse(target.exists())
        binary = target / 'release/muninn'
        binary.parent.mkdir(parents=True)
        binary.write_bytes(b'new executable')
        binary.chmod(0o700)
        return SimpleNamespace(returncode=0, stdout='')

    def test_builds_current_snapshot_records_provenance_and_removes_scratch(self):
        with mock.patch.object(self.build.subprocess, 'run', side_effect=self.invoke):
            with self.build.build_candidate(self.root) as artifact:
                path = artifact.path
                self.assertEqual(path.read_bytes(), b'new executable')
                manifest = artifact.manifest
                sources = manifest['sources']
                self.assertEqual(sources['apps/muninn/src/main.rs'], hashlib.sha256(
                    (self.root / 'apps/muninn/src/main.rs').read_bytes()).hexdigest())
                self.assertEqual(manifest['source_manifest_sha256'], hashlib.sha256(
                    json.dumps(sources, sort_keys=True, separators=(',', ':')).encode()).hexdigest())
                self.assertEqual(manifest['binary_sha256'], hashlib.sha256(b'new executable').hexdigest())
                self.assertEqual(manifest['toolchain']['rustc'], 'trusted toolchain version')
                command = manifest['build_command']
                self.assertEqual(command[:8], ['cargo', 'build', '--locked', '--release',
                                              '-p', 'muninn', '--bin', 'muninn'])
                artifact.verify()
                self.assertEqual(Path(self.commands[-1][1]['env']['CARGO_HOME']).parent,
                                 path.parents[2])
                self.assertNotIn('RUSTFLAGS', self.commands[-1][1]['env'])
                path.write_bytes(b'tampered')
                with self.assertRaises(RuntimeError):
                    artifact.verify()
        self.assertFalse(path.exists())
        self.assertTrue(all(call[1]['cwd'] != self.root for call in self.commands))

    def test_unsafe_symlinks_and_external_manifest_paths_fail_before_build(self):
        for mode in ('file', 'directory', 'manifest'):
            with self.subTest(mode=mode):
                if mode == 'file':
                    path = self.root / 'apps/muninn/src/escape.rs'
                    path.symlink_to('/not-readable-external')
                elif mode == 'directory':
                    path = self.root / 'third_party/escape'
                    path.symlink_to('/not-readable-external', target_is_directory=True)
                else:
                    self.put('apps/muninn/Cargo.toml',
                             '[dependencies]\nexternal = { path = "../../../outside" }\n')
                with mock.patch.object(self.build.subprocess, 'run') as run:
                    with self.assertRaises(RuntimeError):
                        with self.build.build_candidate(self.root):
                            self.fail('unsafe snapshot accepted')
                    run.assert_not_called()
                if mode != 'manifest':
                    # TemporaryDirectory owns this test-only symlink.
                    path.unlink()

    def test_sensitive_include_is_rejected_without_reading_its_bytes(self):
        self.put('apps/muninn/src/main.rs',
                 'const DATA: &str = include_str!("../../../dev/fixtures/.env");')
        sensitive = self.put('dev/fixtures/.env', 'synthetic forbidden bytes')
        original = Path.read_bytes
        def guarded(path):
            if path.resolve() == sensitive.resolve():
                raise AssertionError('sensitive bytes were read')
            return original(path)
        with mock.patch.object(Path, 'read_bytes', guarded), \
             mock.patch.object(self.build.subprocess, 'run') as run:
            with self.assertRaises(RuntimeError):
                with self.build.build_candidate(self.root):
                    self.fail('sensitive input accepted')
            run.assert_not_called()

    def test_cargo_config_credentials_are_rejected_before_build(self):
        self.put('.cargo/config.toml', '[registry]\ntoken = "synthetic-test-value"\n')
        with mock.patch.object(self.build.subprocess, 'run') as run:
            with self.assertRaises(RuntimeError):
                with self.build.build_candidate(self.root):
                    self.fail('credential config accepted')
            run.assert_not_called()

    def test_failure_missing_output_or_symlink_output_never_returns_artifact(self):
        for mode in ('failed', 'missing', 'symlink'):
            with self.subTest(mode=mode):
                def invoke(command, **kwargs):
                    if 'build' not in command:
                        return SimpleNamespace(returncode=0, stdout='version')
                    if mode == 'symlink':
                        target = Path(command[command.index('--target-dir') + 1])
                        (target / 'release').mkdir(parents=True)
                        (target / 'release/muninn').symlink_to(self.root / 'Cargo.lock')
                    return SimpleNamespace(returncode=1 if mode == 'failed' else 0, stdout='')
                with mock.patch.object(self.build.subprocess, 'run', side_effect=invoke):
                    with self.assertRaises(RuntimeError):
                        with self.build.build_candidate(self.root):
                            self.fail('invalid output accepted')


if __name__ == '__main__':
    unittest.main()
