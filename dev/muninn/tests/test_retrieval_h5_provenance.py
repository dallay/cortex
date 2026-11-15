import unittest
from copy import deepcopy
from retrieval_h5_provenance import assess_gate_provenance, sha256_bytes, redact


class ProvenanceTests(unittest.TestCase):
    def test_encoder_response_identity_requires_exact_allowlist_and_external_correlation(self):
        from retrieval_h5_provenance import sanitize_encoder_identity, correlate_encoder_identity

        identity = {
            "declared": {
                "artifact_bundle_id": "a" * 64,
                "model_id": "sentence-transformers/all-MiniLM-L6-v2",
                "revision": "b" * 40,
                "space_id": "c" * 64,
            },
            "loaded": {
                "validated_profile": "muninn-minilm-profile-v1",
                "config_sha256": "d" * 64,
            },
        }
        self.assertEqual(identity, sanitize_encoder_identity(identity))
        extra = {**identity, "debug": "private"}
        with self.assertRaises(ValueError):
            sanitize_encoder_identity(extra)
        correlation = correlate_encoder_identity(
            identity, expected_identity=identity, observed_image_id="image-1",
            observed_container_id="container-1", observed_run_id="run-1",
            response_run_id="run-1", response_query_id="query-1", expected_query_id="query-1",
        )
        self.assertEqual("correlated_self_report", correlation["status"])
        self.assertEqual("partial", correlate_encoder_identity(
            identity, expected_identity=None, observed_image_id="image-1",
            observed_container_id="container-1", observed_run_id="run-1",
            response_run_id="run-1", response_query_id="query-1", expected_query_id="query-1",
        )["status"])
        mismatched = deepcopy(identity)
        mismatched["declared"]["artifact_bundle_id"] = "e" * 64
        self.assertEqual("insufficient", correlate_encoder_identity(
            identity, expected_identity=mismatched, observed_image_id="image-1",
            observed_container_id="container-1", observed_run_id="run-1",
            response_run_id="run-1", response_query_id="query-1", expected_query_id="query-1",
        )["status"])
        self.assertEqual("insufficient", correlate_encoder_identity(
            identity, expected_identity=identity, observed_image_id="image-1",
            observed_container_id="container-1", observed_run_id="run-1",
            response_run_id="other-run", response_query_id="query-1", expected_query_id="query-1",
        )["status"])

    def evidence(self):
        keys = {
            "protocol_hash", "review_reference", "operator_corroboration", "corpus_hash",
            "runner_hash", "quality_hash", "bridge_image_id", "bridge_container_id",
            "bridge_binary_hash", "model_bundle_hash", "readonly_mounts", "effective_config",
            "compose_inspect", "encoder_loaded_identity", "run_id", "started_at", "finished_at",
            "gate_results", "encoder_response_identity", "encoder_expected_identity",
            "encoder_response_binding",
        }
        return {key: {"value": "observed", "source": "local", "method": "inspect",
                      "observed_at": "2026-01-01T00:00:00Z"} for key in keys}

    def complete_identity(self):
        return {
            "declared": {"artifact_bundle_id": "a" * 64,
                         "model_id": "sentence-transformers/all-MiniLM-L6-v2",
                         "revision": "b" * 40, "space_id": "c" * 64},
            "loaded": {"validated_profile": "muninn-minilm-profile-v1",
                       "config_sha256": "d" * 64},
        }

    def complete_evidence(self):
        evidence = self.evidence()
        identity = self.complete_identity()
        for key, value in (
            ("encoder_response_identity", deepcopy(identity)),
            ("encoder_expected_identity", deepcopy(identity)),
            ("encoder_response_binding", {"response_run_id": "observed",
                                           "response_query_id": "query-1",
                                           "expected_query_id": "query-1"}),
            ("bridge_image_id", "image-1"), ("bridge_container_id", "container-1"),
            ("run_id", "observed"),
        ):
            evidence[key]["value"] = value
        return evidence

    def test_baseline_is_fixed_and_encoder_configuration_are_required_per_gate(self):
        evidence = self.complete_evidence()
        ann = assess_gate_provenance("ann_exact", evidence)
        self.assertEqual("verified", ann["status"])
        del evidence["encoder_loaded_identity"]
        ann = assess_gate_provenance("ann_exact", evidence)
        self.assertEqual("partial", ann["status"])
        self.assertIn("encoder_loaded_identity", ann["missing_or_conflicting"])

    def test_absolute_gate_requires_labels_rubric_and_adjudication(self):
        result = assess_gate_provenance("absolute_relevance", self.complete_evidence())
        self.assertEqual("partial", result["status"])
        self.assertTrue({"labels_hash", "rubric_hash", "adjudication_evidence"}.issubset(result["missing_or_conflicting"]))

    def test_conflicts_are_insufficient_and_sensitive_values_are_redacted(self):
        evidence = self.evidence()
        evidence["compose_inspect"] = {"value": "conflict", "source": "local", "method": "inspect",
                                       "observed_at": "2026-01-01T00:00:00Z", "conflict": True}
        result = assess_gate_provenance("ann_exact", evidence)
        self.assertEqual("insufficient", result["status"])
        self.assertEqual("sha256:" + __import__("hashlib").sha256(b"data").hexdigest(), sha256_bytes(b"data"))
        self.assertEqual("[REDACTED]", redact({"token": "secret"})["token"])

    def test_collects_docker_inspect_and_local_file_byte_hashes_with_fake_engine(self):
        from tempfile import TemporaryDirectory
        from pathlib import Path
        from retrieval_h5_provenance import collect_observations

        class FakeEngine:
            def __init__(self):
                self.calls = []

            def inspect(self, container_id):
                self.calls.append(container_id)
                return [{"Id": "container-123", "Image": "sha256:image", "Config": {
                    "Image": "bridge:tag", "Labels": {"com.docker.compose.project": "h5"}},
                    "State": {"Status": "running"}, "Mounts": [
                        {"Source": "/host/model", "Destination": "/model", "RW": False}]}]

        with TemporaryDirectory() as directory:
            artifact = Path(directory) / "runner.py"
            artifact.write_bytes(b"observed bytes")
            engine = FakeEngine()
            result = collect_observations(
                engine, container_ids=["container-123"], files={"runner_hash": artifact},
                run_id="run-1", started_at="2026-01-01T00:00:00Z",
                finished_at="2026-01-01T00:00:01Z")

        self.assertEqual(["container-123"], engine.calls)
        self.assertEqual("sha256:image", result["bridge_image_id"]["value"])
        self.assertEqual("container-123", result["bridge_container_id"]["value"])
        self.assertEqual("sha256:" + __import__("hashlib").sha256(b"observed bytes").hexdigest(),
                         result["runner_hash"]["value"])
        self.assertEqual(False, result["readonly_mounts"]["value"][0]["rw"])
        self.assertNotIn("encoder_loaded_identity", result)

    def test_cannot_promote_caller_encoder_or_config_claim_to_local_observation(self):
        from retrieval_h5_provenance import collect_observations

        class FakeEngine:
            def inspect(self, _container_id):
                return [{"Id": "container-123", "Image": "sha256:image", "Config": {},
                         "State": {"Status": "running"}, "Mounts": []}]

        result = collect_observations(FakeEngine(), container_ids=["container-123"], files={},
                                      run_id="run-1", started_at="start", finished_at="end",
                                      caller_claims={"encoder_loaded_identity": "trusted-model",
                                                     "effective_config": {"threshold": 0.2}})
        assessment = assess_gate_provenance("ann_exact", result)
        self.assertEqual("partial", assessment["status"])
        self.assertIn("encoder_loaded_identity", assessment["missing_or_conflicting"])
        self.assertIn("effective_config", assessment["missing_or_conflicting"])

    def test_unidentifiable_execution_is_insufficient_not_merely_partial(self):
        evidence = self.complete_evidence()
        del evidence["bridge_container_id"]
        result = assess_gate_provenance("ann_exact", evidence)
        self.assertEqual("insufficient", result["status"])
        self.assertIn("bridge_container_id", result["missing_or_conflicting"])

    def test_identity_mismatch_and_missing_query_binding_block_verified_provenance(self):
        evidence = self.complete_evidence()
        evidence["encoder_response_identity"]["value"]["declared"]["artifact_bundle_id"] = "e" * 64
        result = assess_gate_provenance("ann_exact", evidence)
        self.assertEqual("insufficient", result["status"])
        self.assertIn("encoder_response_identity", result["missing_or_conflicting"])

        evidence = self.complete_evidence()
        del evidence["encoder_response_binding"]
        result = assess_gate_provenance("ann_exact", evidence)
        self.assertEqual("partial", result["status"])
        self.assertIn("encoder_response_binding", result["missing_or_conflicting"])

    def prior_observations(self):
        from retrieval_h5_provenance import LocalObservations
        # Explicit synthetic prior records, not collected operational evidence.
        keys = ('protocol_hash', 'review_reference', 'operator_corroboration', 'corpus_hash',
                'runner_hash', 'quality_hash', 'bridge_image_id', 'bridge_container_id',
                'bridge_binary_hash', 'model_bundle_hash', 'readonly_mounts', 'effective_config',
                'compose_inspect', 'encoder_expected_identity', 'run_id', 'started_at')
        return LocalObservations({key: deepcopy(self.complete_evidence()[key]) for key in keys})

    def test_preflight_needs_no_future_evidence_and_does_not_authorize_or_mutate(self):
        from retrieval_h5_provenance import assess_preflight_provenance
        observations = self.prior_observations()
        before = deepcopy(observations)
        result = assess_preflight_provenance('ann_exact', observations)
        self.assertEqual('eligible', result['status'])
        self.assertEqual([], result['missing_or_conflicting'])
        self.assertEqual(before, observations)
        self.assertNotIn('PASS', str(result))
        final = assess_gate_provenance('ann_exact', observations)
        self.assertEqual('partial', final['status'])
        for key in ('finished_at', 'gate_results', 'encoder_response_identity',
                    'encoder_response_binding', 'encoder_loaded_identity'):
            self.assertIn(key, final['missing_or_conflicting'])
            self.assertNotIn(key, result['observations'])
        # Future evidence cannot substitute for a missing prior record.
        observations.update({key: value for key, value in self.complete_evidence().items()
                             if key not in observations})
        del observations['started_at']
        self.assertEqual('blocked', assess_preflight_provenance('ann_exact', observations)['status'])

    def test_each_prior_record_and_metadata_is_required_and_conflicts_block(self):
        from retrieval_h5_provenance import assess_preflight_provenance
        for key in self.prior_observations():
            for invalid in ('absent', 'conflict', 'value', 'source', 'method', 'observed_at'):
                observations = self.prior_observations()
                if invalid == 'absent':
                    del observations[key]
                elif invalid == 'conflict':
                    observations[key]['conflict'] = True
                else:
                    del observations[key][invalid]
                with self.subTest(key=key, invalid=invalid):
                    result = assess_preflight_provenance('ann_exact', observations)
                    self.assertEqual('blocked', result['status'])
                    self.assertIn(key, result['missing_or_conflicting'])
        observations = self.prior_observations()
        observations['effective_config']['value'] = {'api_key': 'not-public'}
        self.assertEqual('[REDACTED]', assess_preflight_provenance('ann_exact', observations)
                         ['observations']['effective_config']['value']['api_key'])

    def test_preflight_rejects_plain_claims_and_does_not_weaken_gate_baselines(self):
        from retrieval_h5_provenance import assess_preflight_provenance, LocalObservations
        with self.assertRaises(ValueError):
            assess_preflight_provenance('ann_exact', dict(self.prior_observations()))
        with self.assertRaises(ValueError):
            assess_preflight_provenance('unknown', self.prior_observations())
        self.assertEqual('blocked', assess_preflight_provenance('ann_exact', LocalObservations())['status'])
        observations = self.prior_observations()
        result = assess_preflight_provenance('absolute_relevance', observations)
        self.assertEqual('blocked', result['status'])
        self.assertEqual(['adjudication_evidence', 'labels_hash', 'rubric_hash'],
                         result['missing_or_conflicting'])
        for key in result['missing_or_conflicting']:
            observations[key] = deepcopy(observations['corpus_hash'])
        self.assertEqual('eligible', assess_preflight_provenance('absolute_relevance', observations)['status'])
        # The final assessor still requires every post-response record and full config.
        for key in ('finished_at', 'gate_results', 'encoder_response_identity',
                    'encoder_response_binding', 'encoder_loaded_identity', 'effective_config'):
            evidence = self.complete_evidence()
            del evidence[key]
            final = assess_gate_provenance('ann_exact', evidence)
            self.assertNotEqual('verified', final['status'])
            self.assertIn(key, final['missing_or_conflicting'])

    def test_record_metadata_types_and_conflict_flag_fail_closed_in_both_apis(self):
        from retrieval_h5_provenance import assess_preflight_provenance
        cases = [(field, value) for field in ('source', 'method', 'observed_at')
                 for value in ({'unexpected': 'object'}, True, ['not-a-timestamp'], '', ' \t\n')]
        cases += [('conflict', value) for value in ('true', 'false', 1, 0, None, {}, [])]
        for field, value in cases:
            with self.subTest(field=field, value=value):
                prior = self.prior_observations()
                prior['bridge_container_id'][field] = value
                preflight = assess_preflight_provenance('ann_exact', prior)
                self.assertEqual('blocked', preflight['status'])
                self.assertIn('bridge_container_id', preflight['missing_or_conflicting'])
                final = self.complete_evidence()
                final['bridge_container_id'][field] = value
                assessment = assess_gate_provenance('ann_exact', final)
                self.assertNotEqual('verified', assessment['status'])
                self.assertIn('bridge_container_id', assessment['missing_or_conflicting'])

    def test_valid_record_metadata_preserves_values_and_optional_boolean_conflict(self):
        from retrieval_h5_provenance import assess_preflight_provenance
        for conflict in ('absent', False, True):
            for value in ('observed', {'limit': 10}, ['synthetic']):
                prior = self.prior_observations()
                # Config values remain polymorphic; timestamps receive no new format policy.
                prior['effective_config'].update(value=value, source=' local ', method='inspect',
                                                 observed_at='synthetic-clock-label')
                if conflict != 'absent':
                    prior['effective_config']['conflict'] = conflict
                before = deepcopy(prior)
                self.assertEqual('blocked' if conflict is True else 'eligible',
                                 assess_preflight_provenance('ann_exact', prior)['status'])
                self.assertEqual(before, prior)
                final = self.complete_evidence()
                final['effective_config'] = deepcopy(prior['effective_config'])
                self.assertEqual('insufficient' if conflict is True else 'verified',
                                 assess_gate_provenance('ann_exact', final)['status'])
