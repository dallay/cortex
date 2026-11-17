"""Host-orchestrator regressions; no Docker daemon calls are made."""

import asyncio
import ast
import json
from pathlib import Path
import re
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import gm01_runner as runner


class ReviewTests(unittest.TestCase):
    NEO4J_IMAGE_ID = 'sha256:' + 'b' * 64

    @staticmethod
    def _stage_name(command):
        if command[:3] == ['docker', 'image', 'inspect']:
            return ('inspect_neo4j' if command[-1] == runner.NEO4J_IMAGE
                    else 'inspect_capture')
        return next((name for name in ('config', 'build', 'up', 'down')
                     if name in command), None)

    def test_reference_custom_contract_static_without_imports(self):
        settings = runner.reference_options('bolt://reference-db:7687', 'private')
        self.assertEqual(settings['embedding']['provider'], 'custom')
        source = runner.ROOT / 'dev/muninn/.cache/sources/agent-memory/src/neo4j_agent_memory'
        tree = ast.parse((source / 'config/settings.py').read_text())
        enum = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == 'EmbeddingProvider')
        self.assertIn('custom', [n.value.value for n in enum.body if isinstance(n, ast.Assign)])
        module = ast.parse((source / '__init__.py').read_text())
        client = next(n for n in module.body if isinstance(n, ast.ClassDef) and n.name == 'MemoryClient')
        connect = next(n for n in client.body if isinstance(n, ast.AsyncFunctionDef) and n.name == '_connect_bolt')
        branch = next(n for n in connect.body if isinstance(n, ast.If) and '_embedder_override' in ast.unparse(n.test))
        self.assertNotIn('_create_embedder()', ast.unparse(branch.body))
        self.assertIn('_create_embedder()', ast.unparse(branch.orelse))

    def _run_host_with_mock_compose(self, root, fail_stage=None,
                                   image_inspect_output='sha256:' + 'a' * 64,
                                   neo4j_inspect_returncode=0,
                                   neo4j_inspect_output=NEO4J_IMAGE_ID):
        evidence = root / 'evidence'
        context = root / 'context'
        context.mkdir()
        (context / 'build-provenance.json').write_text(json.dumps({
            'sources': runner.source_hashes(runner.ROOT),
        }))
        commands = []
        secrets_seen = []
        secret_directories = {}
        self.secret_directories = secret_directories
        self.secrets_seen = secrets_seen

        def fake_run(command, **options):
            commands.append(command)
            if command[:3] == ['docker', 'image', 'inspect']:
                if command[-1] == runner.NEO4J_IMAGE:
                    return SimpleNamespace(returncode=neo4j_inspect_returncode,
                                           stdout=neo4j_inspect_output)
                return SimpleNamespace(returncode=0, stdout=image_inspect_output)
            compose_path = Path(command[command.index('-f') + 1])
            compose_text = compose_path.read_text()
            for secret_path in re.findall(r'    file: "([^"]+)"', compose_text):
                secret_file = Path(secret_path)
                secrets_seen.append(secret_file.read_text().strip())
                self.assertEqual(secret_file.stat().st_mode & 0o777, 0o444)
                secret_directories[secret_file.parent] = (
                    secret_file.parent.stat().st_mode & 0o777
                )
            stage = next((name for name in ('config', 'build', 'up', 'down')
                          if name in command), None)
            if stage == 'up':
                manifest = runner.empty_manifest()
                manifest.update(status='captured', comparison='pass', capture_marker='preserve')
                runner.save(evidence / 'manifest.json', manifest)
            if (fail_stage == stage
                    or isinstance(fail_stage, (tuple, list, set)) and stage in fail_stage):
                if isinstance(fail_stage, (tuple, list, set)) and stage == 'down':
                    raise ValueError('cleanup secret-marker')
                return SimpleNamespace(returncode=1)
            return SimpleNamespace(returncode=0)

        with patch.object(runner, 'prepare_reference', return_value=Path('/verified/src')), \
             patch.object(runner, 'stage_build_context', side_effect=lambda *args: context), \
             patch.object(runner.subprocess, 'run', side_effect=fake_run):
            self.last_commands = commands
            asyncio.run(runner.run(Path('/reference'), evidence))
        return evidence, commands, secrets_seen

    def test_run_uses_private_checkout_cache_for_compose_secrets_and_always_cleans_it(self):
        cache = (runner.ROOT / 'dev/muninn/.cache').resolve()
        for fail_stage in (None, 'up'):
            with self.subTest(fail_stage=fail_stage), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                if fail_stage is None:
                    evidence, _, _ = self._run_host_with_mock_compose(root)
                else:
                    with self.assertRaises(RuntimeError):
                        self._run_host_with_mock_compose(root, fail_stage=fail_stage)
                    evidence = root / 'evidence'

                self.assertEqual(len(self.secret_directories), 1)
                manifest_text = (evidence / 'manifest.json').read_text()
                evidence_text = '\n'.join(path.read_text(errors='replace')
                                           for path in evidence.rglob('*') if path.is_file())
                for directory, mode in self.secret_directories.items():
                    with self.subTest(directory=directory):
                        self.assertEqual(directory.resolve().parent, cache)
                        self.assertEqual(mode, 0o700)
                        self.assertFalse(directory.exists())
                        self.assertNotIn(str(directory), manifest_text)
                        self.assertNotIn(str(directory), evidence_text)
                self.assertFalse(any(value.removeprefix('neo4j/') in evidence_text
                                     for value in self.secrets_seen))

    def test_neo4j_preflight_runs_before_build_and_records_id_in_final_manifest(self):
        with tempfile.TemporaryDirectory() as temporary:
            evidence, commands, _ = self._run_host_with_mock_compose(Path(temporary))

            self.assertEqual([self._stage_name(command) for command in commands],
                             ['config', 'inspect_neo4j', 'build', 'inspect_capture', 'up', 'down'])
            self.assertEqual(commands[1], ['docker', 'image', 'inspect', '--format',
                                           '{{.Id}}', runner.NEO4J_IMAGE])
            self.assertEqual(commands[2][-3:], ['build', '--pull', 'capture'])
            self.assertIn('--pull', commands[4])
            self.assertEqual(commands[4][commands[4].index('--pull') + 1], 'never')
            manifest = json.loads((evidence / 'manifest.json').read_text())
            self.assertEqual(manifest['neo4j_image'], {
                'reference': runner.NEO4J_IMAGE,
                'id': self.NEO4J_IMAGE_ID,
            })

    def test_missing_or_malformed_neo4j_id_blocks_before_build_and_up(self):
        cases = (
            ('missing', 1, ''),
            ('malformed', 0, 'not-an-image-id secret-marker'),
        )
        for label, returncode, output in cases:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                with self.assertRaises(RuntimeError):
                    self._run_host_with_mock_compose(
                        root, neo4j_inspect_returncode=returncode,
                        neo4j_inspect_output=output)

                commands = self.last_commands
                manifest = json.loads((root / 'evidence/manifest.json').read_text())
                stages = [self._stage_name(command) for command in commands]

                self.assertEqual(stages, ['config', 'inspect_neo4j', 'down'])
                self.assertNotIn('build', stages)
                self.assertNotIn('up', stages)
                self.assertEqual(manifest['status'], 'blocked')
                self.assertEqual(manifest['stage'], 'inspect_neo4j_image')
                self.assertEqual(manifest['error_type'], 'RuntimeError')
                self.assertNotIn('neo4j_image', manifest)
                self.assertNotIn('secret-marker', json.dumps(manifest))

    def test_neo4j_id_is_preserved_in_blocked_manifest_after_up_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(RuntimeError):
                self._run_host_with_mock_compose(root, fail_stage='up')

            manifest = json.loads((root / 'evidence/manifest.json').read_text())
            self.assertEqual(manifest['status'], 'blocked')
            self.assertEqual(manifest['neo4j_image'], {
                'reference': runner.NEO4J_IMAGE,
                'id': self.NEO4J_IMAGE_ID,
            })

    def test_compose_host_run_orders_build_capture_and_scoped_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            evidence, commands, secret_contents = self._run_host_with_mock_compose(Path(temporary))
            self.assertEqual([self._stage_name(command) for command in commands],
                             ['config', 'inspect_neo4j', 'build', 'inspect_capture', 'up', 'down'])
            self.assertEqual(commands[2][-3:], ['build', '--pull', 'capture'])
            self.assertEqual(commands[3][:3], ['docker', 'image', 'inspect'])
            self.assertRegex(commands[3][-1], r'^muninn-gm01-capture:[a-f0-9]+$')
            self.assertIn('--pull', commands[4])
            self.assertEqual(commands[4][commands[4].index('--pull') + 1], 'never')
            self.assertIn('--no-build', commands[4])
            self.assertEqual(commands[-1][-3:], ['down', '--volumes', '--remove-orphans'])
            self.assertTrue(all('muninn-gm01-' in command[command.index('-p') + 1]
                                for command in commands if '-p' in command))
            self.assertFalse(any('prune' in command or 'docker.sock' in command
                                 for command in commands))
            self.assertEqual(len(secret_contents), 8)
            self.assertTrue(all(value.startswith('neo4j/') for value in secret_contents))
            serialized_commands = json.dumps(commands)
            self.assertFalse(any(value.removeprefix('neo4j/') in serialized_commands
                                  for value in secret_contents))
            manifest = json.loads((evidence / 'manifest.json').read_text())
            self.assertEqual(manifest['h1_acceptance'], 'not_established')
            self.assertEqual(manifest['capture_image'], {
                'reference': commands[3][-1], 'id': 'sha256:' + 'a' * 64,
            })
            self.assertEqual(manifest['neo4j_image'], {
                'reference': runner.NEO4J_IMAGE, 'id': self.NEO4J_IMAGE_ID,
            })
            self.assertEqual(manifest['capture_marker'], 'preserve')
            self.assertEqual(manifest['comparison'], 'pass')
            self.assertNotIn('password', json.dumps(manifest).lower())

    def test_malformed_capture_image_id_blocks_without_persisting_inspect_output(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(RuntimeError):
                self._run_host_with_mock_compose(
                    root, image_inspect_output='malformed-image-id secret-marker')

            manifest = json.loads((root / 'evidence/manifest.json').read_text())
            commands = self.last_commands

            self.assertEqual(manifest['status'], 'blocked')
            self.assertEqual(manifest['stage'], 'inspect_capture_image')
            self.assertEqual(manifest['error_type'], 'RuntimeError')
            self.assertNotIn('capture_image', manifest)
            self.assertNotIn('secret-marker', json.dumps(manifest))
            self.assertEqual([self._stage_name(command) for command in commands],
                              ['config', 'inspect_neo4j', 'build', 'inspect_capture', 'down'])

    def test_build_failure_still_runs_only_project_scoped_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence = root / 'evidence'
            context = root / 'context'
            context.mkdir()
            (context / 'build-provenance.json').write_text(json.dumps({
                'sources': runner.source_hashes(runner.ROOT),
            }))
            commands = []

            def fake_run(command, **options):
                commands.append(command)
                if command[:3] == ['docker', 'image', 'inspect']:
                    return SimpleNamespace(returncode=0, stdout=self.NEO4J_IMAGE_ID)
                stage = next((name for name in ('config', 'build', 'up') if name in command), None)
                return SimpleNamespace(returncode=1 if stage == 'build' else 0)

            with patch.object(runner, 'prepare_reference', return_value=Path('/verified/src')), \
                 patch.object(runner, 'stage_build_context', side_effect=lambda *args: context), \
                 patch.object(runner.subprocess, 'run', side_effect=fake_run):
                with self.assertRaises(RuntimeError):
                    asyncio.run(runner.run(Path('/reference'), evidence))

            self.assertEqual([self._stage_name(command) for command in commands],
                             ['config', 'inspect_neo4j', 'build', 'down'])
            self.assertEqual(commands[2][-3:], ['build', '--pull', 'capture'])
            self.assertFalse(any(command[-1].startswith('muninn-gm01-capture:')
                                 for command in commands))
            self.assertEqual(commands[-1][-3:], ['down', '--volumes', '--remove-orphans'])
            self.assertNotIn('prune', json.dumps(commands))
            manifest = json.loads((evidence / 'manifest.json').read_text())
            self.assertEqual(manifest['status'], 'blocked')
            self.assertEqual(manifest['error_type'], 'RuntimeError')
            self.assertEqual(manifest['h1_acceptance'], 'not_established')
            self.assertEqual(manifest['neo4j_image'], {
                'reference': runner.NEO4J_IMAGE, 'id': self.NEO4J_IMAGE_ID,
            })
            self.assertNotIn('Build de imagen fallido', json.dumps(manifest))

    def test_cleanup_failure_marks_container_manifest_blocked(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(RuntimeError):
                self._run_host_with_mock_compose(root, fail_stage='down')
            manifest = json.loads((root / 'evidence/manifest.json').read_text())
            self.assertEqual(manifest['status'], 'blocked')
            self.assertEqual(manifest['orchestration_stage'], 'compose_cleanup')
            self.assertEqual(manifest['orchestration_error_type'], 'RuntimeError')
            self.assertEqual(manifest['cleanup_status'], 'failed')
            self.assertEqual(manifest['cleanup_error_type'], 'RuntimeError')
            self.assertEqual(manifest['cleanup_warning'],
                             'La limpieza de recursos puede estar incompleta.')
            self.assertEqual(manifest['capture_marker'], 'preserve')
            self.assertEqual(manifest['capture_image']['id'], 'sha256:' + 'a' * 64)
            self.assertEqual(manifest['neo4j_image']['id'], self.NEO4J_IMAGE_ID)
            self.assertEqual(manifest['h1_acceptance'], 'not_established')
            self.assertEqual(self.last_commands[-1][-3:],
                             ['down', '--volumes', '--remove-orphans'])
            self.assertNotIn('cleanup secret-marker', json.dumps(manifest))
            self.assertNotIn('Limpieza Compose fallida', json.dumps(manifest))

    def test_up_failure_blocks_and_preserves_capture_and_inspected_image(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(RuntimeError):
                self._run_host_with_mock_compose(root, fail_stage='up')

            manifest = json.loads((root / 'evidence/manifest.json').read_text())

            self.assertEqual(manifest['status'], 'blocked')
            self.assertEqual(manifest['orchestration_stage'], 'compose_up')
            self.assertEqual(manifest['orchestration_error_type'], 'RuntimeError')
            self.assertEqual(manifest['capture_marker'], 'preserve')
            self.assertEqual(manifest['capture_image']['id'], 'sha256:' + 'a' * 64)
            self.assertNotIn('Captura Compose fallida', json.dumps(manifest))
            self.assertNotIn('secret-marker', json.dumps(manifest))

    def test_primary_up_and_cleanup_failures_preserve_both_diagnostics(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(RuntimeError):
                self._run_host_with_mock_compose(root, fail_stage=('up', 'down'))

            manifest = json.loads((root / 'evidence/manifest.json').read_text())

            self.assertEqual(manifest['status'], 'blocked')
            self.assertEqual(manifest['orchestration_stage'], 'compose_up')
            self.assertEqual(manifest['orchestration_error_type'], 'RuntimeError')
            self.assertEqual(manifest['cleanup_status'], 'failed')
            self.assertEqual(manifest['cleanup_error_type'], 'ValueError')
            self.assertEqual(manifest['cleanup_warning'],
                             'La limpieza de recursos puede estar incompleta.')
            self.assertEqual(manifest['capture_marker'], 'preserve')
            self.assertEqual(manifest['h1_acceptance'], 'not_established')
            self.assertNotIn('secret-marker', json.dumps(manifest))

    def test_image_and_network_invariants_are_fixed(self):
        self.assertEqual(runner.NEO4J_IMAGE,
                         'neo4j:5.26-community@sha256:414adfe41c6ff32abb752be37f265c77607c166cab298f65a917fc4f05776218')
        self.assertIn('@sha256:', runner.RUST_BUILDER_IMAGE)
        self.assertIn('@sha256:', runner.PYTHON_IMAGE)


class ComposeFailureDiagnosticTests(unittest.TestCase):
    NEO4J_IMAGE_ID = 'sha256:' + 'b' * 64
    CAPTURE_IMAGE_ID = 'sha256:' + 'a' * 64

    @staticmethod
    def _container(service, status, exit_code, oom_killed, health=None):
        state = {
            'Status': status,
            'ExitCode': exit_code,
            'OOMKilled': oom_killed,
            'Error': 'state-error-secret-marker',
        }
        if health is not None:
            state['Health'] = {'Status': health, 'Log': ['health-secret-marker']}
        return {
            'Id': 'must-not-be-recorded',
            'Config': {
                'Labels': {'com.docker.compose.service': service},
                'Env': ['environment-secret-marker'],
            },
            'State': state,
            'Mounts': [{'Source': 'mount-secret-marker'}],
        }

    def _run_failed_up(self, root, ids, records=(), ps_returncode=0,
                       inspect_returncode=0):
        evidence = root / 'evidence'
        context = root / 'context'
        context.mkdir()
        (context / 'build-provenance.json').write_text(json.dumps({
            'sources': runner.source_hashes(runner.ROOT),
        }))
        events = []
        docker_calls = []
        project_names = []

        def fake_run(command, **options):
            if command[:3] == ['docker', 'image', 'inspect']:
                if command[-1] == runner.NEO4J_IMAGE:
                    return SimpleNamespace(returncode=0, stdout=self.NEO4J_IMAGE_ID)
                return SimpleNamespace(returncode=0, stdout=self.CAPTURE_IMAGE_ID)
            if command[:2] == ['docker', 'ps']:
                events.append('ps')
                docker_calls.append(command)
                return SimpleNamespace(returncode=ps_returncode, stdout='\n'.join(ids))
            if command[:2] == ['docker', 'inspect']:
                events.append('inspect')
                docker_calls.append(command)
                return SimpleNamespace(returncode=inspect_returncode,
                                       stdout=json.dumps(list(records)))

            project_names.append(command[command.index('-p') + 1])
            stage = next((name for name in ('config', 'build', 'up', 'down')
                          if name in command), None)
            events.append(stage)
            return SimpleNamespace(returncode=1 if stage == 'up' else 0)

        with patch.object(runner, 'prepare_reference', return_value=Path('/verified/src')), \
             patch.object(runner, 'stage_build_context', side_effect=lambda *args: context), \
             patch.object(runner.subprocess, 'run', side_effect=fake_run):
            with self.assertRaisesRegex(RuntimeError, 'GM-01 bloqueado'):
                asyncio.run(runner.run(Path('/reference'), evidence))

        manifest = json.loads((evidence / 'manifest.json').read_text())
        self.assertEqual(len(set(project_names)), 1)
        return manifest, events, docker_calls, project_names[0]

    def assert_diagnostic_order_and_project_scope(self, events, docker_calls, project):
        self.assertEqual(events[events.index('up'):],
                         ['up', 'ps', 'inspect', 'down'] if 'inspect' in events
                         else ['up', 'ps', 'down'])
        self.assertEqual(docker_calls[0], [
            'docker', 'ps', '-aq', '--filter',
            f'label=com.docker.compose.project={project}',
        ])
        self.assertRegex(project, r'^muninn-gm01-[a-f0-9]+$')

    def test_failed_capture_records_only_safe_state_for_exact_project_ids(self):
        db_reference = '1' * 64
        db_rust = '2' * 64
        capture = '3' * 64
        records = [
            self._container('reference-db', 'running', 0, False, 'healthy'),
            self._container('rust-db', 'running', 0, False, 'healthy'),
            self._container('capture', 'exited', 1, False),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            manifest, events, docker_calls, project = self._run_failed_up(
                Path(temporary), [db_reference, db_rust, capture], records)

        self.assert_diagnostic_order_and_project_scope(events, docker_calls, project)
        self.assertEqual(docker_calls[1], ['docker', 'inspect', db_reference, db_rust, capture])
        self.assertEqual(manifest['status'], 'blocked')
        self.assertEqual(manifest['stage'], 'compose_up')
        self.assertEqual(manifest['capture_image']['id'], self.CAPTURE_IMAGE_ID)
        self.assertEqual(manifest['neo4j_image']['id'], self.NEO4J_IMAGE_ID)
        self.assertEqual(manifest['compose_diagnostics'], {
            'status': 'containers_found',
            'services': [
                {'service': 'reference-db', 'state': 'running', 'exit_code': 0,
                 'oom_killed': False, 'health_status': 'healthy'},
                {'service': 'rust-db', 'state': 'running', 'exit_code': 0,
                 'oom_killed': False, 'health_status': 'healthy'},
                {'service': 'capture', 'state': 'exited', 'exit_code': 1,
                 'oom_killed': False, 'health_status': None},
            ],
        })
        serialized = json.dumps(manifest)
        for secret in ('secret-marker', 'mount-secret-marker', 'environment-secret-marker',
                       'must-not-be-recorded', db_reference, db_rust, capture):
            self.assertNotIn(secret, serialized)

    def test_unhealthy_database_records_no_capture_service_when_not_created(self):
        db_reference = '4' * 64
        db_rust = '5' * 64
        records = [
            self._container('reference-db', 'running', 0, False, 'healthy'),
            self._container('rust-db', 'exited', 1, True, 'unhealthy'),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            manifest, events, docker_calls, project = self._run_failed_up(
                Path(temporary), [db_reference, db_rust], records)

        self.assert_diagnostic_order_and_project_scope(events, docker_calls, project)
        self.assertEqual(docker_calls[1], ['docker', 'inspect', db_reference, db_rust])
        self.assertEqual([item['service'] for item in
                          manifest['compose_diagnostics']['services']],
                         ['reference-db', 'rust-db'])
        self.assertEqual(manifest['compose_diagnostics']['services'][1]['health_status'],
                         'unhealthy')
        self.assertEqual(manifest['compose_diagnostics']['services'][1]['oom_killed'], True)
        self.assertEqual(manifest['status'], 'blocked')

    def test_empty_project_resource_list_is_recorded_as_no_containers(self):
        with tempfile.TemporaryDirectory() as temporary:
            manifest, events, docker_calls, project = self._run_failed_up(Path(temporary), [])

        self.assertEqual(events[events.index('up'):], ['up', 'ps', 'down'])
        self.assertEqual(docker_calls, [[
            'docker', 'ps', '-aq', '--filter',
            f'label=com.docker.compose.project={project}',
        ]])
        self.assertEqual(manifest['compose_diagnostics'], {
            'status': 'no_containers', 'services': [],
        })
        self.assertEqual(manifest['status'], 'blocked')

    def test_malformed_container_id_records_only_diagnostic_error_type_and_cleans_up(self):
        with tempfile.TemporaryDirectory() as temporary:
            manifest, events, docker_calls, project = self._run_failed_up(
                Path(temporary), ['invalid-id secret-marker'])

        self.assertEqual(events[events.index('up'):], ['up', 'ps', 'down'])
        self.assertEqual(docker_calls, [[
            'docker', 'ps', '-aq', '--filter',
            f'label=com.docker.compose.project={project}',
        ]])
        self.assertEqual(manifest['compose_diagnostics'], {
            'diagnostic_error_type': 'ValueError',
        })
        self.assertEqual(manifest['status'], 'blocked')
        self.assertNotIn('secret-marker', json.dumps(manifest))

    def test_project_list_command_failure_records_only_type_and_still_cleans_up(self):
        with tempfile.TemporaryDirectory() as temporary:
            manifest, events, docker_calls, project = self._run_failed_up(
                Path(temporary), [], ps_returncode=1)

        self.assertEqual(events[events.index('up'):], ['up', 'ps', 'down'])
        self.assertEqual(docker_calls, [[
            'docker', 'ps', '-aq', '--filter',
            f'label=com.docker.compose.project={project}',
        ]])
        self.assertEqual(manifest['compose_diagnostics'], {
            'diagnostic_error_type': 'RuntimeError',
        })
        self.assertEqual(manifest['status'], 'blocked')

    def test_inspect_command_failure_records_only_type_and_still_cleans_up(self):
        container_id = '6' * 64
        with tempfile.TemporaryDirectory() as temporary:
            manifest, events, docker_calls, project = self._run_failed_up(
                Path(temporary), [container_id], inspect_returncode=1)

        self.assertEqual(events[events.index('up'):], ['up', 'ps', 'inspect', 'down'])
        self.assertEqual(docker_calls, [
            ['docker', 'ps', '-aq', '--filter',
             f'label=com.docker.compose.project={project}'],
            ['docker', 'inspect', container_id],
        ])
        self.assertEqual(manifest['compose_diagnostics'], {
            'diagnostic_error_type': 'RuntimeError',
        })
        self.assertEqual(manifest['status'], 'blocked')

    def test_invalid_project_name_is_rejected_before_docker_commands(self):
        with patch.object(runner.subprocess, 'run') as command:
            with self.assertRaises(ValueError):
                runner.inspect_compose_project('other-project')
        command.assert_not_called()

if __name__ == '__main__':
    unittest.main()
