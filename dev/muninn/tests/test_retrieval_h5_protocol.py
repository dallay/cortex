import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from retrieval_h5_protocol import canonical_protocol_hash, validate_protocol, corroborate_approval


class ProtocolTests(unittest.TestCase):
    @staticmethod
    def pending(rationale="Awaiting explicit product approval"):
        return {"status": "pending", "value": None, "rationale": rationale}

    @staticmethod
    def approved(value):
        return {"status": "approved", "value": value, "rationale": "Structural test value only"}

    def structurally_complete_protocol(self):
        protocol = self.protocol()
        identity = {"declared": {"artifact_bundle_id": "a" * 64,
                    "model_id": "sentence-transformers/all-MiniLM-L6-v2", "revision": "b" * 40,
                    "space_id": "c" * 64}, "loaded": {"validated_profile": "muninn-minilm-profile-v1",
                    "config_sha256": "d" * 64}}
        values = {
            "metrics_and_k": {"k_by_metric": {metric: 10 for metric in
                ("ExactTopKRecall@K", "OrderedTopKMatch@K", "Recall@K", "nDCG@K")}},
            "acceptance_rules_and_tolerances": {gate: {"criterion": "structural only", "threshold": 0.5}
                for gate in ("ann_exact", "absolute_relevance")},
            "run_count_and_aggregation": {"run_count": 1, "aggregation": "structural only",
                "tie_policy": "structural only", "undefined_policy": "structural only"},
            "corpus_and_query_candidate_scope": {"corpus_id": "structural-only", "version": "test-version",
                "queries": [{"query_id": "q1", "candidate_ids": ["c1"]}]},
            "labels_rubric_and_adjudication": {"labels_version": "test-labels", "rubric": "structural only",
                "adjudication": "structural only"},
            "ann_exact_comparison_scope": {"retrieval_path": "structural only", "candidate_scope": "structural only"},
            "encoder_identity": identity,
            "provenance": {key: "structural only" for key in ("encoder_model", "build_bridge", "dependencies",
                "corpus_labels", "retrieval_configuration", "runtime_environment")},
        }
        protocol["run_inputs"] = {name: self.approved(value) for name, value in values.items()}
        return protocol

    def protocol(self):
        return {
            "schema_version": "h5-acceptance/v1",
            "protocol_id": "example",
            "protocol_version": "draft",
            "repository_review": {
                "commit": "abc123", "review_reference": "review-1",
                "approved_at": "2026-01-01T00:00:00Z",
                "approved_gates": ["ann_exact", "absolute_relevance"],
            },
            "run_inputs": {
                "metrics_and_k": None, "acceptance_rules_and_tolerances": None,
                "run_count_and_aggregation": None, "corpus_and_query_candidate_scope": None,
                "labels_rubric_and_adjudication": None, "ann_exact_comparison_scope": None,
                "encoder_identity": None,
            },
            "gates": {"ann_exact": True, "absolute_relevance": True},
        }

    @staticmethod
    def schema_result(protocol):
        schema_path = Path(__file__).parents[1] / "fixtures/retrieval-h5-protocol.schema.json"
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json", encoding="utf-8") as output:
            json.dump(protocol, output, allow_nan=False)
            output.flush()
            return subprocess.run(["jsonschema", "-i", output.name, str(schema_path)], capture_output=True)

    def test_incomplete_example_is_not_confirmatory_or_hash_approved(self):
        protocol = self.protocol()
        validated = validate_protocol(protocol)
        self.assertEqual({"ann_exact", "absolute_relevance"}, set(validated["gates"]))
        self.assertFalse(validated["confirmatory_ready"])
        self.assertTrue(validated["missing_fields"])
        self.assertEqual(canonical_protocol_hash(protocol), validated["sha256"])

    def test_unknown_and_evidence_downgrade_fields_are_rejected(self):
        for field in ("unexpected", "evidence_requirement"):
            protocol = self.protocol()
            protocol[field] = "weaken baseline"
            with self.subTest(field=field), self.assertRaises(ValueError):
                validate_protocol(protocol)

    def test_protocol_requires_a_strict_expected_encoder_identity_for_confirmatory_readiness(self):
        protocol = self.protocol()
        self.assertIn("run_inputs.encoder_identity", validate_protocol(protocol)["missing_fields"])
        identity = {
            "declared": {"artifact_bundle_id": "a" * 64,
                         "model_id": "sentence-transformers/all-MiniLM-L6-v2",
                         "revision": "b" * 40, "space_id": "c" * 64},
            "loaded": {"validated_profile": "muninn-minilm-profile-v1",
                       "config_sha256": "d" * 64},
        }
        protocol["run_inputs"]["encoder_identity"] = self.approved(identity)
        validated = validate_protocol(protocol)
        self.assertNotIn("run_inputs.encoder_identity", validated["missing_fields"])
        protocol["run_inputs"]["encoder_identity"]["value"]["extra"] = "caller substitution"
        self.assertTrue(validate_protocol(protocol)["malformed"])

    def test_approval_requires_prior_operator_corroboration_of_exact_hash(self):
        protocol = self.structurally_complete_protocol()
        digest = canonical_protocol_hash(protocol)
        record = {"protocol_sha256": digest, "commit": "abc123", "review_reference": "review-1",
                  "corroborated_at": "2026-01-02T00:00:00Z", "corroborated_by": "operator-1"}
        self.assertFalse(corroborate_approval(protocol, expected_sha256=digest, operator_record=None))
        self.assertFalse(corroborate_approval(protocol, expected_sha256="0" * 64, operator_record=record))
        self.assertTrue(corroborate_approval(protocol, expected_sha256=digest, operator_record=record))

    def test_approved_at_requires_timezone_but_pending_sentinel_remains_schema_valid(self):
        root = Path(__file__).parents[1]
        fixture = json.loads((root / "fixtures/retrieval-h5-protocol.example.json").read_text())
        self.assertEqual("PENDING", fixture["repository_review"]["approved_at"])
        self.assertTrue(any(item["path"] == "repository_review.approved_at"
                            for item in validate_protocol(fixture)["pending"]))
        self.assertEqual(0, self.schema_result(fixture).returncode)

        for timestamp in ("2026-01-01T00:00:00", "not-a-timestamp", "2026-99-99T25:61:61Z"):
            protocol = self.structurally_complete_protocol()
            protocol["repository_review"]["approved_at"] = timestamp
            with self.subTest(timestamp=timestamp):
                self.assertIn("repository_review.approved_at",
                              [item["path"] for item in validate_protocol(protocol)["malformed"]])
                self.assertNotEqual(0, self.schema_result(protocol).returncode)

        protocol = self.structurally_complete_protocol()
        protocol["repository_review"]["approved_at"] = "pending whatever"
        self.assertNotEqual(0, self.schema_result(protocol).returncode)

        protocol = self.structurally_complete_protocol()
        protocol["repository_review"]["approved_at"] = "2026-01-01T00:00:00+00:00"
        self.assertFalse(any(item["path"] == "repository_review.approved_at"
                             for item in validate_protocol(protocol)["malformed"]))
        self.assertEqual(0, self.schema_result(protocol).returncode)

    def test_metadata_uses_only_the_exact_pending_sentinel(self):
        root = Path(__file__).parents[1]
        fixture = json.loads((root / "fixtures/retrieval-h5-protocol.example.json").read_text())
        expected_paths = {
            "protocol_id",
            "protocol_version",
            "repository_review.commit",
            "repository_review.review_reference",
            "repository_review.approved_at",
        }
        validation = validate_protocol(fixture)
        self.assertFalse(validation["confirmatory_ready"])
        self.assertEqual(expected_paths, {item["path"] for item in validation["pending"]
                                           if item["path"] in expected_paths})
        self.assertEqual(0, self.schema_result(fixture).returncode)

        protocol = self.structurally_complete_protocol()
        self.assertEqual(0, self.schema_result(protocol).returncode)
        for path in ("protocol_id", "protocol_version", "repository_review.commit",
                     "repository_review.review_reference"):
            candidate = copy.deepcopy(protocol)
            if path.startswith("repository_review."):
                candidate["repository_review"][path.rsplit(".", 1)[1]] = "PENDING"
            else:
                candidate[path] = "PENDING"
            self.assertEqual(0, self.schema_result(candidate).returncode, path)

        for path in ("protocol_id", "protocol_version", "repository_review.commit",
                     "repository_review.review_reference"):
            candidate = copy.deepcopy(protocol)
            if path.startswith("repository_review."):
                candidate["repository_review"][path.rsplit(".", 1)[1]] = "pending whatever"
            else:
                candidate[path] = "pending whatever"
            self.assertNotEqual(0, self.schema_result(candidate).returncode, path)

        candidate = copy.deepcopy(fixture)
        candidate["protocol_id"] = " PENDING"
        self.assertTrue(any(item["path"] == "protocol_id" for item in validate_protocol(candidate)["pending"]))
        self.assertNotEqual(0, self.schema_result(candidate).returncode)

    def test_corroborate_approval_fails_closed_for_naive_aware_timestamps(self):
        protocol = self.structurally_complete_protocol()
        digest = canonical_protocol_hash(protocol)
        record = {"protocol_sha256": digest, "commit": "abc123", "review_reference": "review-1",
                  "corroborated_at": "2026-01-02T00:00:00Z", "corroborated_by": "operator-1"}
        protocol["repository_review"]["approved_at"] = "2026-01-01T00:00:00"
        self.assertFalse(corroborate_approval(protocol, expected_sha256=canonical_protocol_hash(protocol),
                                              operator_record=record))

        protocol["repository_review"]["approved_at"] = "2026-01-01T00:00:00Z"
        record["protocol_sha256"] = canonical_protocol_hash(protocol)
        record["corroborated_at"] = "2026-01-02T00:00:00"
        self.assertFalse(corroborate_approval(protocol, expected_sha256=canonical_protocol_hash(protocol),
                                              operator_record=record))

    def test_approved_gates_rejects_non_string_json_items_as_value_error(self):
        for invalid_gate in ([], {}):
            protocol = self.protocol()
            protocol["repository_review"]["approved_gates"] = [invalid_gate]
            with self.subTest(invalid_gate=invalid_gate), self.assertRaises(ValueError):
                validate_protocol(protocol)

    def test_metric_k_allowlist_matches_gate_combination_in_python_and_schema(self):
        protocol = self.structurally_complete_protocol()
        protocol["gates"] = {"ann_exact": True, "absolute_relevance": False}
        del protocol["run_inputs"]["labels_rubric_and_adjudication"]
        metrics = protocol["run_inputs"]["metrics_and_k"]["value"]["k_by_metric"]
        del metrics["Recall@K"]
        del metrics["nDCG@K"]
        metrics["Recall@K"] = 10
        validation = validate_protocol(protocol)
        self.assertTrue(any(item["path"] == "run_inputs.metrics_and_k.value.k_by_metric.Recall@K"
                            for item in validation["malformed"]))
        self.assertNotEqual(0, self.schema_result(protocol).returncode)

        absolute_only = self.structurally_complete_protocol()
        absolute_only["gates"] = {"ann_exact": False, "absolute_relevance": True}
        del absolute_only["run_inputs"]["ann_exact_comparison_scope"]
        metrics = absolute_only["run_inputs"]["metrics_and_k"]["value"]["k_by_metric"]
        del metrics["ExactTopKRecall@K"]
        del metrics["OrderedTopKMatch@K"]
        metrics["OrderedTopKMatch@K"] = 10
        validation = validate_protocol(absolute_only)
        self.assertTrue(any(item["path"] == "run_inputs.metrics_and_k.value.k_by_metric.OrderedTopKMatch@K"
                            for item in validation["malformed"]))
        self.assertNotEqual(0, self.schema_result(absolute_only).returncode)

        both_disabled = self.structurally_complete_protocol()
        both_disabled["gates"] = {"ann_exact": False, "absolute_relevance": False}
        del both_disabled["run_inputs"]["labels_rubric_and_adjudication"]
        del both_disabled["run_inputs"]["ann_exact_comparison_scope"]
        both_disabled["run_inputs"]["metrics_and_k"]["value"]["k_by_metric"] = {"Recall@K": 10}
        validation = validate_protocol(both_disabled)
        self.assertTrue(any(item["path"] == "run_inputs.metrics_and_k.value.k_by_metric.Recall@K"
                            for item in validation["malformed"]))
        self.assertNotEqual(0, self.schema_result(both_disabled).returncode)

    def test_acceptance_rules_match_enabled_gates_in_python_and_schema(self):
        cases = (
            ({"ann_exact": True, "absolute_relevance": True},
             {"ann_exact", "absolute_relevance"}, set()),
            ({"ann_exact": True, "absolute_relevance": False}, {"ann_exact"},
             {"absolute_relevance"}),
            ({"ann_exact": False, "absolute_relevance": True}, {"absolute_relevance"},
             {"ann_exact"}),
        )
        for gates, required, rejected in cases:
            protocol = self.structurally_complete_protocol()
            protocol["gates"] = gates
            if not gates["ann_exact"]:
                del protocol["run_inputs"]["ann_exact_comparison_scope"]
                metrics = protocol["run_inputs"]["metrics_and_k"]["value"]["k_by_metric"]
                del metrics["ExactTopKRecall@K"]
                del metrics["OrderedTopKMatch@K"]
            if not gates["absolute_relevance"]:
                del protocol["run_inputs"]["labels_rubric_and_adjudication"]
                metrics = protocol["run_inputs"]["metrics_and_k"]["value"]["k_by_metric"]
                del metrics["Recall@K"]
                del metrics["nDCG@K"]
            rules = {gate: {"criterion": "structural sample", "threshold": 0.5}
                     for gate in required}
            protocol["run_inputs"]["acceptance_rules_and_tolerances"] = self.approved(rules)
            with self.subTest(gates=gates, rules=rules):
                validation = validate_protocol(protocol)
                self.assertFalse(validation["malformed"])
                self.assertFalse(validation["missing"])
                self.assertEqual(0, self.schema_result(protocol).returncode)

            for gate in required:
                incomplete = copy.deepcopy(protocol)
                del incomplete["run_inputs"]["acceptance_rules_and_tolerances"]["value"][gate]
                with self.subTest(gates=gates, missing_rule=gate):
                    validation = validate_protocol(incomplete)
                    self.assertTrue(validation["missing"])
                    self.assertNotEqual(0, self.schema_result(incomplete).returncode)

            for gate in rejected:
                invalid = copy.deepcopy(protocol)
                invalid["run_inputs"]["acceptance_rules_and_tolerances"]["value"][gate] = {
                    "criterion": "structural sample", "threshold": 0.5}
                with self.subTest(gates=gates, disabled_rule=gate):
                    self.assertTrue(validate_protocol(invalid)["malformed"])
                    self.assertNotEqual(0, self.schema_result(invalid).returncode)

        disabled = self.structurally_complete_protocol()
        disabled["gates"] = {"ann_exact": False, "absolute_relevance": False}
        del disabled["run_inputs"]["ann_exact_comparison_scope"]
        del disabled["run_inputs"]["labels_rubric_and_adjudication"]
        disabled["run_inputs"]["metrics_and_k"] = self.pending()
        disabled["run_inputs"]["acceptance_rules_and_tolerances"] = self.pending()
        validation = validate_protocol(disabled)
        self.assertFalse(validation["confirmatory_ready"])
        self.assertFalse(validation["malformed"])
        self.assertTrue(any(item["path"] == "run_inputs.acceptance_rules_and_tolerances"
                            for item in validation["pending"]))
        self.assertEqual(0, self.schema_result(disabled).returncode)

        disabled["run_inputs"]["acceptance_rules_and_tolerances"] = self.approved({})
        validation = validate_protocol(disabled)
        self.assertTrue(any(item["path"] == "run_inputs.acceptance_rules_and_tolerances.value"
                            for item in validation["malformed"]))
        self.assertNotEqual(0, self.schema_result(disabled).returncode)

    def test_decision_envelopes_require_explicit_status_value_and_rationale(self):
        protocol = self.protocol()
        protocol["run_inputs"]["metrics_and_k"] = self.pending()
        validated = validate_protocol(protocol)
        self.assertIn("run_inputs.metrics_and_k", [item["path"] for item in validated["pending"]])
        self.assertFalse(validated["confirmatory_ready"])

        protocol["run_inputs"]["metrics_and_k"] = {
            "status": "approved", "value": {"k_by_metric": {"ExactTopKRecall@K": 10}},
            "rationale": "Approved by product",
        }
        validated = validate_protocol(protocol)
        self.assertIn("run_inputs.metrics_and_k.value.k_by_metric.OrderedTopKMatch@K",
                      [item["path"] for item in validated["missing"]])

    def test_decision_envelope_rejects_empty_placeholder_wrong_type_and_unknown_keys(self):
        invalid = [
            {},
            {"status": "pending", "value": "not-null", "rationale": "waiting"},
            {"status": "pending", "value": None, "rationale": ""},
            {"status": "rejected", "value": None, "rationale": "waiting"},
            {"status": "approved", "value": {}, "rationale": "approved"},
            {"status": "approved", "value": {"k_by_metric": "10"}, "rationale": "approved"},
            {"status": "approved", "value": {"k_by_metric": {"ExactTopKRecall@K": 10,
                "OrderedTopKMatch@K": 10, "Recall@K": 10, "nDCG@K": 10}},
                "rationale": "approved", "extra": True},
            {"status": "approved", "value": {"k_by_metric": {"ExactTopKRecall@K": 10,
                "OrderedTopKMatch@K": 10, "Recall@K": 10, "nDCG@K": 10}},
                "rationale": "PENDING"},
        ]
        for decision in invalid:
            protocol = self.protocol()
            protocol["run_inputs"]["metrics_and_k"] = decision
            with self.subTest(decision=decision):
                self.assertFalse(validate_protocol(protocol)["confirmatory_ready"])

    def test_missing_pending_and_malformed_paths_are_separate_and_sorted(self):
        protocol = self.protocol()
        protocol["run_inputs"]["metrics_and_k"] = self.pending()
        protocol["run_inputs"]["provenance"] = {
            "status": "approved", "value": {"encoder_model": "real", "unexpected": "extra"},
            "rationale": "Structural test",
        }
        first = validate_protocol(protocol)
        second = validate_protocol(protocol)
        self.assertEqual(first["missing"], second["missing"])
        self.assertEqual(first["pending"], second["pending"])
        self.assertEqual(first["malformed"], second["malformed"])
        self.assertEqual(sorted(first["missing"], key=lambda item: (item["path"], item["reason"])),
                         first["missing"])
        self.assertIn("run_inputs.metrics_and_k", [item["path"] for item in first["pending"]])
        self.assertIn("run_inputs.provenance.value.unexpected",
                      [item["path"] for item in first["malformed"]])
        self.assertFalse(first["confirmatory_ready"])

    def test_finding_report_is_deterministic_and_blocks_operator_corroboration(self):
        protocol = self.protocol()
        first = validate_protocol(protocol)
        second = validate_protocol(protocol)
        self.assertEqual(first["missing"], second["missing"])
        self.assertEqual(first["pending"], second["pending"])
        self.assertEqual(first["malformed"], second["malformed"])
        digest = canonical_protocol_hash(protocol)
        record = {"protocol_sha256": digest, "commit": "abc123", "review_reference": "review-1",
                  "corroborated_at": "2026-01-02T00:00:00Z", "corroborated_by": "operator-1"}
        self.assertFalse(corroborate_approval(protocol, expected_sha256=digest, operator_record=record))

    def test_json_schema_matches_pending_template_and_strict_approved_shapes(self):
        root = Path(__file__).parents[1]
        schema_path = root / "fixtures/retrieval-h5-protocol.schema.json"
        fixture = json.loads((root / "fixtures/retrieval-h5-protocol.example.json").read_text())
        template_result = validate_protocol(fixture)
        self.assertFalse(template_result["confirmatory_ready"])
        self.assertTrue(template_result["pending"])
        self.assertEqual(0, subprocess.run(["jsonschema", "-i", str(root / "fixtures/retrieval-h5-protocol.example.json"),
                                            str(schema_path)], capture_output=True).returncode)
        def schema_result(instance):
            with tempfile.NamedTemporaryFile(mode="w", suffix=".json", encoding="utf-8") as output:
                json.dump(instance, output)
                output.flush()
                return subprocess.run(["jsonschema", "-i", output.name, str(schema_path)], capture_output=True)
        malformed = copy.deepcopy(fixture)
        malformed["run_inputs"]["metrics_and_k"] = {"status": "approved", "value": {}, "rationale": "test"}
        self.assertNotEqual(0, schema_result(malformed).returncode)
        approved = self.structurally_complete_protocol()
        approved["repository_review"]["approved_at"] = "2026-01-01T00:00:00Z"
        approved_result = schema_result(approved)
        self.assertEqual(0, approved_result.returncode, approved_result.stderr.decode())
        approved["run_inputs"]["metrics_and_k"]["value"]["k_by_metric"]["extra"] = 10
        self.assertNotEqual(0, schema_result(approved).returncode)

    def test_gate_specific_fields_are_required_only_for_enabled_gate(self):
        protocol = self.structurally_complete_protocol()
        protocol["gates"] = {"ann_exact": True, "absolute_relevance": False}
        del protocol["run_inputs"]["labels_rubric_and_adjudication"]
        del protocol["run_inputs"]["acceptance_rules_and_tolerances"]["value"]["absolute_relevance"]
        del protocol["run_inputs"]["metrics_and_k"]["value"]["k_by_metric"]["Recall@K"]
        del protocol["run_inputs"]["metrics_and_k"]["value"]["k_by_metric"]["nDCG@K"]
        validated = validate_protocol(protocol)
        self.assertFalse(any(item["path"].startswith("run_inputs.labels_rubric")
                             for category in ("missing", "pending", "malformed") for item in validated[category]))
        schema_path = Path(__file__).parents[1] / "fixtures/retrieval-h5-protocol.schema.json"
        with tempfile.NamedTemporaryFile(mode="w", suffix=".json", encoding="utf-8") as output:
            json.dump(protocol, output)
            output.flush()
            result = subprocess.run(["jsonschema", "-i", output.name, str(schema_path)], capture_output=True)
        self.assertEqual(0, result.returncode, result.stderr.decode())

    def v2_protocol(self):
        protocol = self.structurally_complete_protocol()
        protocol['schema_version'] = 'h5-acceptance/v2'
        protocol['gates']['absolute_relevance'] = False
        inputs = protocol['run_inputs']
        del inputs['labels_rubric_and_adjudication']
        inputs['metrics_and_k']['value']['k_by_metric'] = {
            'ExactTopKRecall@K': 10, 'OrderedTopKMatch@K': 10}
        inputs['acceptance_rules_and_tolerances']['value'] = {'ann_exact': {
            'criterion': 'Informative synthetic description', 'threshold': 1.0,
            'policy_id': 'ann-exact-per-query-equality-v1'}}
        inputs['run_count_and_aggregation']['value'] = {
            'run_count': 1, 'aggregation': 'per-query-governing-macro-descriptive-v1',
            'tie_policy': 'observed-ranking-no-retrospective-tiebreak-v1',
            'undefined_policy': 'required-undefined-blocks-pass-v1'}
        inputs['corpus_and_query_candidate_scope']['value']['corpus_hash'] = 'sha256:' + 'a' * 64
        inputs['ann_exact_comparison_scope']['value'] = {
            'retrieval_path': '/h5/diagnostic/search', 'candidate_scope': 'same-query-candidates-v1',
            'search_config': {'limit': 10, 'threshold': 0.0}}
        return protocol

    def test_v2_closed_contract_cross_validation(self):
        protocol = self.v2_protocol()
        self.assertTrue(validate_protocol(protocol)['confirmatory_ready'])
        self.assertEqual(0, self.schema_result(protocol).returncode)
        inputs = protocol['run_inputs']
        for name in inputs:
            for mutation in ('missing', 'pending', 'extra'):
                candidate = copy.deepcopy(protocol)
                if mutation == 'missing':
                    del candidate['run_inputs'][name]
                elif mutation == 'pending':
                    candidate['run_inputs'][name] = self.pending()
                else:
                    candidate['run_inputs'][name]['value']['extra'] = True
                with self.subTest(name=name, mutation=mutation):
                    validation = validate_protocol(candidate)
                    self.assertFalse(validation['confirmatory_ready'])
                    self.assertTrue(validation[{'missing': 'missing', 'pending': 'pending',
                                                'extra': 'malformed'}[mutation]])
                    self.assertEqual(mutation == 'pending', self.schema_result(candidate).returncode == 0)

    def test_v2_exact_tokens_and_numeric_domains_match_schema(self):
        cases = {
            'metrics_and_k.k_by_metric.ExactTopKRecall@K': (10.0, [True, 9, 10.5, '10']),
            'metrics_and_k.k_by_metric.OrderedTopKMatch@K': (10, [False, 11]),
            'acceptance_rules_and_tolerances.ann_exact.policy_id': (
                'ann-exact-per-query-equality-v1', ['ANN-exact-per-query-equality-v1', 'other']),
            'acceptance_rules_and_tolerances.ann_exact.threshold': (1, [True, 0.99, '1']),
            'run_count_and_aggregation.run_count': (1.0, [True, 2, 1.5]),
            'run_count_and_aggregation.aggregation': ('per-query-governing-macro-descriptive-v1', ['macro']),
            'run_count_and_aggregation.tie_policy': ('observed-ranking-no-retrospective-tiebreak-v1', ['observed']),
            'run_count_and_aggregation.undefined_policy': ('required-undefined-blocks-pass-v1', ['skip']),
            'corpus_and_query_candidate_scope.corpus_hash': ('sha256:' + 'b' * 64, ['b' * 64, 'sha256:' + 'B' * 64]),
            'ann_exact_comparison_scope.retrieval_path': ('/h5/diagnostic/search', ['/search']),
            'ann_exact_comparison_scope.candidate_scope': ('same-query-candidates-v1', ['all']),
            'ann_exact_comparison_scope.search_config.limit': (100.0, [True, 9, 101, 10.5]),
            'ann_exact_comparison_scope.search_config.threshold': (1, [False, -0.1, 1.1, '0']),
        }
        for path, (valid, invalid) in cases.items():
            if path.endswith(('threshold', 'limit', 'run_count', '@K')):
                invalid += [float('nan'), float('inf'), -float('inf')]
            for value in [valid] + invalid:
                candidate = self.v2_protocol()
                name, *keys = path.split('.')
                target = candidate['run_inputs'][name]['value']
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = value
                with self.subTest(path=path, value=value):
                    expected = value == valid and type(value) is not bool
                    self.assertEqual(expected, validate_protocol(candidate)['confirmatory_ready'])
                    if type(value) is float and not __import__('math').isfinite(value):
                        # Non-finite values have no JSON representation. Do not
                        # claim the permissive CLI decoder enforces JSON syntax.
                        with self.assertRaises(ValueError):
                            self.schema_result(candidate)
                    else:
                        self.assertEqual(expected, self.schema_result(candidate).returncode == 0)
            candidate = self.v2_protocol()
            name, *keys = path.split('.')
            target = candidate['run_inputs'][name]['value']
            for key in keys[:-1]:
                target = target[key]
            del target[keys[-1]]
            self.assertTrue(validate_protocol(candidate)['missing'], path)
            self.assertNotEqual(0, self.schema_result(candidate).returncode, path)

    def test_v2_hash_corroboration_covers_every_leaf_and_version(self):
        protocol = self.v2_protocol()
        digest = canonical_protocol_hash(protocol)
        record = {'protocol_sha256': digest, 'commit': 'abc123', 'review_reference': 'review-1',
                  'corroborated_at': '2026-01-02T00:00:00Z', 'corroborated_by': 'synthetic'}
        self.assertTrue(corroborate_approval(protocol, expected_sha256=digest, operator_record=record))
        def leaves(value, path=()):
            if isinstance(value, dict):
                for key, item in value.items():
                    yield from leaves(item, path + (key,))
            elif isinstance(value, list):
                for key, item in enumerate(value):
                    yield from leaves(item, path + (key,))
            else:
                yield path
        for path in leaves(protocol):
            candidate = copy.deepcopy(protocol)
            target = candidate
            for key in path[:-1]:
                target = target[key]
            target[path[-1]] = 'h5-acceptance/v1' if path == ('schema_version',) else 'altered'
            with self.subTest(path=path):
                self.assertNotEqual(digest, canonical_protocol_hash(candidate))
                try:
                    approved = corroborate_approval(candidate, expected_sha256=digest, operator_record=record)
                except ValueError:
                    approved = False
                self.assertFalse(approved)

    def test_v2_gate_matrix_is_closed_without_changing_v1(self):
        for ann in (False, True):
            for relevance in (False, True):
                protocol = self.v2_protocol()
                protocol['gates'] = {'ann_exact': ann, 'absolute_relevance': relevance}
                if relevance:
                    protocol['run_inputs']['labels_rubric_and_adjudication'] = self.approved({
                        'labels_version': 'synthetic', 'rubric': 'synthetic', 'adjudication': 'synthetic'})
                with self.subTest(ann=ann, relevance=relevance):
                    validation = validate_protocol(protocol)
                    self.assertEqual(ann and not relevance, validation['confirmatory_ready'])
                    if not ann or relevance:
                        self.assertTrue(any(item['path'].startswith('gates.')
                                            for item in validation['malformed']))
                    self.assertEqual(ann and not relevance, self.schema_result(protocol).returncode == 0)

    def test_v2_corpus_hash_rejects_trailing_newline_in_both_validators(self):
        protocol = self.v2_protocol()
        protocol['run_inputs']['corpus_and_query_candidate_scope']['value']['corpus_hash'] += '\n'
        self.assertTrue(validate_protocol(protocol)['malformed'])
        self.assertNotEqual(0, self.schema_result(protocol).returncode)

    def test_v2_bounded_fields_reject_huge_integers_without_exception(self):
        for path in ('metrics_and_k.k_by_metric.ExactTopKRecall@K',
                     'metrics_and_k.k_by_metric.OrderedTopKMatch@K',
                     'acceptance_rules_and_tolerances.ann_exact.threshold',
                     'run_count_and_aggregation.run_count',
                     'ann_exact_comparison_scope.search_config.limit',
                     'ann_exact_comparison_scope.search_config.threshold'):
            for value in (10**400, -(10**400)):
                protocol = self.v2_protocol()
                name, *keys = path.split('.')
                target = protocol['run_inputs'][name]['value']
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = value
                with self.subTest(path=path, sign=value > 0):
                    validation = validate_protocol(protocol)
                    self.assertFalse(validation['confirmatory_ready'])
                    self.assertIn('run_inputs.' + name + '.value.' + '.'.join(keys),
                                  [item['path'] for item in validation['malformed']])
                    self.assertNotEqual(0, self.schema_result(protocol).returncode)
