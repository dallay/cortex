"""Compose packaging contract tests; stdlib only and no Docker services."""

import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import gm01_runner as runner


class FullHarnessPackagingTests(unittest.TestCase):
    def test_lifecycle_and_contract_are_in_hashed_harness(self):
        self.assertIn('gm01_lifecycle.py', runner.HARNESS_FILES)
        self.assertIn('gm01_contract.py', runner.HARNESS_FILES)


class BuildContextTests(unittest.TestCase):
    def test_context_hashes_literal_compile_time_assets_and_excludes_unrelated_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'workspace'
            root.mkdir()
            for name, content in {
                'Cargo.toml': '[workspace]\n',
                'Cargo.lock': 'lock bytes\n',
                'rust-toolchain.toml': '[toolchain]\nchannel = "1.99"\n',
                '.cargo/config.toml': '[build]\n',
                'apps/muninn/Cargo.toml': '[package]\nname="muninn"\n',
                'apps/muninn/src/main.rs': 'const DATA: &str = include_str!("payload.json");\n',
                'apps/muninn/src/payload.json': '{"current":true}\n',
                'crates/muninn/core/Cargo.toml': '[package]\nname="muninn-core"\n',
                'crates/muninn/core/src/lib.rs': 'pub fn current() {}\n',
                'third_party/candle-core/Cargo.toml': '[package]\nname="candle-core"\n',
                '.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/old.json': '{"historical":true}\n',
                'target/debug/muninn': 'Mach-O must not enter the context',
                '.cargo/credentials.toml': 'token = "not for the context"\n',
            }.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)

            hashes = runner.source_hashes(root)

            self.assertIn('apps/muninn/src/payload.json', hashes)
            self.assertEqual(hashes['apps/muninn/src/payload.json'],
                             hashlib.sha256(b'{"current":true}\n').hexdigest())
            self.assertIn('Cargo.lock', hashes)
            self.assertIn('rust-toolchain.toml', hashes)
            self.assertIn('.cargo/config.toml', hashes)
            self.assertNotIn('.cargo/credentials.toml', hashes)
            self.assertNotIn('.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/old.json', hashes)
            self.assertNotIn('target/debug/muninn', hashes)

            reference = root / 'verified-reference/src'
            package = reference / 'neo4j_agent_memory'
            package.mkdir(parents=True)
            (package / '__init__.py').write_text('# verified package\n')
            (reference.parent / 'LICENSE').write_text('Apache License\n')
            tool_root = Path(temporary) / 'tool-root'
            tool_muninn = tool_root / 'dev/muninn'
            tool_muninn.mkdir(parents=True)
            package_hash = hashlib.sha256(b'# verified package\n').hexdigest()
            license_hash = hashlib.sha256(b'Apache License\n').hexdigest()
            integrity = {'agent-memory': {
                'commit': runner.PIN,
                'files': {'src/neo4j_agent_memory/__init__.py': package_hash,
                          'LICENSE': license_hash},
            }}
            (tool_muninn / 'source-integrity.json').write_text(json.dumps(integrity))
            for name in (*runner.HARNESS_FILES, 'requirements-oracle.txt', 'Dockerfile.gm01'):
                (tool_muninn / name).write_text(name + '\n')
            with patch.object(runner, 'ROOT', tool_root):
                context = runner.stage_build_context(
                    root, reference, Path(temporary) / 'context')
            staged = {path.relative_to(context).as_posix() for path in context.rglob('*')
                      if path.is_file()}
            self.assertIn('apps/muninn/src/payload.json', staged)
            self.assertIn('reference/src/neo4j_agent_memory/__init__.py', staged)
            self.assertIn('reference/LICENSE', staged)
            self.assertNotIn('.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/old.json', staged)
            self.assertNotIn('target/debug/muninn', staged)
            self.assertNotIn('.cargo/credentials.toml', staged)
            provenance = json.loads((context / 'build-provenance.json').read_text())
            self.assertEqual(provenance['sources'], hashes)
            self.assertEqual(provenance['reference_license_sha256'],
                             license_hash)
            self.assertIn('gm01_runner.py', provenance['capture_harness_sha256'])

            (root / '.cargo/config.toml').unlink()
            with patch.object(runner, 'ROOT', tool_root):
                empty_config_context = runner.stage_build_context(
                    root, reference, Path(temporary) / 'context-with-empty-cargo-config')
            empty_config = empty_config_context / '.cargo/config.toml'
            self.assertTrue(empty_config.is_file())
            self.assertEqual(empty_config.read_text(), '# No workspace Cargo configuration.\n')

            (root / '.cargo/config').write_text('[build]\n')
            with patch.object(runner, 'ROOT', tool_root):
                legacy_config_context = runner.stage_build_context(
                    root, reference, Path(temporary) / 'context-with-legacy-cargo-config')
            self.assertEqual((legacy_config_context / '.cargo/config').read_text(), '[build]\n')
            self.assertFalse((legacy_config_context / '.cargo/config.toml').exists())

            integrity['agent-memory']['commit'] = '0' * 40
            (tool_muninn / 'source-integrity.json').write_text(json.dumps(integrity))
            invalid_context = Path(temporary) / 'wrong-reference-context'
            with patch.object(runner, 'ROOT', tool_root), \
                 self.assertRaisesRegex(RuntimeError, 'Pin de referencia'):
                runner.stage_build_context(root, reference, invalid_context)
            self.assertFalse(invalid_context.exists())

    def test_cargo_config_with_credentials_is_not_staged(self):
        configs = (
            '[registries.private]\ntoken = "never-copy-this"\n',
            '[source.private]\nregistry = "sparse+https://secret-marker@packages.invalid/index"\n',
            '[registries.private]\nauth = "secret-marker"\n',
        )
        for config in configs:
            with self.subTest(config=config), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / '.cargo').mkdir()
                (root / '.cargo/config.toml').write_text(config)
                with self.assertRaisesRegex(RuntimeError, 'credencial'):
                    runner.source_hashes(root)

    def test_cargo_config_rejects_normalized_secret_keys(self):
        configs = (
            'ANTHROPIC_API_KEY = "never-copy-this"\n',
            '[aws]\nAWS_ACCESS_KEY_ID = "never-copy-this"\n',
            '[nested]\ntoken = "never-copy-this"\n',
            '[client]\nclient_secret = "never-copy-this"\n',
            '[keys]\nprivate_key = "never-copy-this"\n',
        )
        for config in configs:
            with self.subTest(config=config), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / '.cargo').mkdir()
                (root / '.cargo/config.toml').write_text(config)
                with self.assertRaisesRegex(RuntimeError, 'credencial'):
                    runner.source_hashes(root)

    def test_cargo_config_allows_legitimate_auth_settings(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cargo = root / '.cargo'
            cargo.mkdir()
            config = cargo / 'config.toml'
            config.write_text(
                '[registries.private]\nauth-required = true\n'
                'credential-provider = "cargo:token"\n'
                '[build]\ntarget-dir = "target/cargo"\n'
            )

            hashes = runner.source_hashes(root)

            self.assertIn('.cargo/config.toml', hashes)

    def test_cargo_config_symlink_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'workspace'
            cargo = root / '.cargo'
            cargo.mkdir(parents=True)
            external = Path(temporary) / 'external-config.toml'
            external.write_text('[build]\n')
            (cargo / 'config.toml').symlink_to(external)
            with self.assertRaisesRegex(RuntimeError, 'symlink'):
                runner.source_hashes(root)


class ComposeIsolationTests(unittest.TestCase):
    def test_auth_secret_is_a_private_read_only_neo4j_auth_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            private = Path(temporary) / 'private'
            private.mkdir(mode=0o700)
            secret = private / 'neo4j.auth'
            runner.write_auth_file(secret, 'secret-marker')
            self.assertEqual(secret.read_text(), 'neo4j/secret-marker')
            self.assertEqual(secret.stat().st_mode & 0o777, 0o444)
            self.assertEqual(private.stat().st_mode & 0o777, 0o700)

    def test_compose_secret_directory_is_private_cache_scoped_and_always_cleaned(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cache = root / 'dev/muninn/.cache'
            cache.mkdir(parents=True, mode=0o700)
            secret_directory = None

            with self.assertRaisesRegex(RuntimeError, 'test failure'):
                with runner.temporary_compose_secrets(root) as secret_directory:
                    self.assertEqual(secret_directory.parent, cache.resolve())
                    self.assertEqual(secret_directory.stat().st_mode & 0o777, 0o700)
                    runner.write_auth_file(secret_directory / 'reference.auth', 'reference')
                    runner.write_auth_file(secret_directory / 'rust.auth', 'rust')
                    self.assertEqual(
                        {path.name for path in secret_directory.iterdir()},
                        {'reference.auth', 'rust.auth'},
                    )
                    self.assertTrue(all(path.stat().st_mode & 0o777 == 0o444
                                        for path in secret_directory.iterdir()))
                    raise RuntimeError('test failure')

            self.assertIsNotNone(secret_directory)
            self.assertFalse(secret_directory.exists())

    def test_compose_secret_cache_symlink_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cache_parent = root / 'dev/muninn'
            cache_parent.mkdir(parents=True)
            external_cache = root / 'external-cache'
            external_cache.mkdir()
            (cache_parent / '.cache').symlink_to(external_cache, target_is_directory=True)

            with self.assertRaisesRegex(RuntimeError, 'symlink'):
                with runner.temporary_compose_secrets(root):
                    self.fail('La caché symlink no debe aceptarse')

    def test_multi_stage_image_builds_current_linux_binary_and_locked_python_dependencies(self):
        dockerfile = (Path(runner.ROOT) / 'dev/muninn/Dockerfile.gm01').read_text()
        self.assertIn(runner.RUST_BUILDER_IMAGE, dockerfile)
        self.assertIn(runner.PYTHON_IMAGE, dockerfile)
        self.assertIn('cargo +1.99.1 build --locked --release -p muninn --bin muninn',
                      dockerfile)
        self.assertIn('--require-hashes -r /tmp/requirements-oracle.txt', dockerfile)
        self.assertIn('COPY --from=builder /tmp/target/release/muninn /usr/local/bin/muninn',
                      dockerfile)
        self.assertIn('COPY reference/src/ /opt/reference/src/', dockerfile)
        self.assertIn('COPY reference/LICENSE /usr/share/licenses/muninn/agent-memory-LICENSE.txt',
                      dockerfile)
        self.assertIn('USER 1000:1000', dockerfile)
        self.assertNotIn('COPY target/', dockerfile)
        self.assertNotIn('/var/run/docker.sock', dockerfile)
        active_user = 'root'
        for line in dockerfile.splitlines():
            if line.startswith('FROM '):
                active_user = 'root'
            elif line.startswith('USER '):
                active_user = line.removeprefix('USER ')
            elif line.startswith('RUN '):
                self.assertNotEqual(active_user, 'root')

    def test_rendered_stack_has_three_internal_arm64_services_and_no_host_ports(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            context = root / 'context'
            context.mkdir()
            evidence = root / 'new-evidence'
            evidence.mkdir()
            reference_secret = root / 'reference.auth'
            rust_secret = root / 'rust.auth'
            reference_secret.write_text('neo4j/reference-private\n')
            rust_secret.write_text('neo4j/rust-private\n')
            reference_secret.chmod(0o444)
            rust_secret.chmod(0o444)

            document = runner.compose_document(
                context, evidence, reference_secret, rust_secret,
                project='muninn-gm01-a1', uid=501, gid=20,
            )

            self.assertEqual(document.count('\n  reference-db:\n') +
                             document.count('\n  rust-db:\n') +
                             document.count('\n  capture:\n'), 3)
            self.assertIn('neo4j:5.26-community@sha256:414adfe41c6ff32abb752be37f265c77607c166cab298f65a917fc4f05776218', document)
            self.assertIn('platform: linux/arm64', document)
            self.assertIn('internal: true', document)
            self.assertNotIn('\n    ports:', document)
            self.assertNotIn('/var/run/docker.sock', document)
            self.assertIn('NEO4J_AUTH_FILE: /run/secrets/reference_auth', document)
            self.assertIn('NEO4J_AUTH_FILE: /run/secrets/rust_auth', document)
            self.assertIn('read_only: true', document)
            self.assertIn('cap_drop: [ALL]', document)
            self.assertIn('no-new-privileges:true', document)
            self.assertIn('logging:\n      driver: none', document)
            self.assertIn('/data:uid=7474,gid=7474,mode=0700', document)
            self.assertIn('source: "' + str(evidence.resolve()) + '"', document)
            self.assertEqual(document.count('target: /evidence'), 1)
            self.assertNotIn(str(context) + ':', document)
            self.assertNotIn('reference-private', document)
            self.assertNotIn('rust-private', document)

    def test_compose_commands_use_paths_not_secret_values_and_scope_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            compose = root / 'compose.yaml'
            compose.write_text('services: {}\n')
            command = runner.compose_command(compose, 'muninn-gm01-a1',
                                             'up', '--pull', 'never')
            self.assertEqual(command[:5], ['docker', 'compose', '-f', str(compose.resolve()),
                                            '-p'])
            self.assertIn('muninn-gm01-a1', command)
            self.assertEqual(command[-2:], ['--pull', 'never'])
            cleanup = runner.compose_command(compose, 'muninn-gm01-a1',
                                             'down', '--volumes', '--remove-orphans')
            self.assertEqual(cleanup[-3:], ['down', '--volumes', '--remove-orphans'])
            self.assertNotIn('prune', cleanup)
            self.assertNotIn('rm', cleanup)

    def test_evidence_is_reserved_exclusively_and_manifest_never_accepts_h1(self):
        with tempfile.TemporaryDirectory() as temporary:
            evidence = Path(temporary) / 'new'
            runner.reserve_evidence(evidence)
            self.assertEqual(list(evidence.iterdir()), [])
            with self.assertRaises(FileExistsError):
                runner.reserve_evidence(evidence)
            manifest = runner.empty_manifest()
            self.assertEqual(manifest['h1_acceptance'], 'not_established')


if __name__ == '__main__':
    unittest.main()
