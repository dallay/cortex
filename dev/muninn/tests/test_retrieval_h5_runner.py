import csv
import hashlib
import io
import json
import unittest
from copy import deepcopy
from pathlib import Path

from retrieval_h5_runner import (HttpBridge, run_evaluation, serialize_protocol_gate_metrics,
                                 integrate_protocol_gate_metrics, validate_endpoint, runner_connection)
from retrieval_quality import corpus_hash


class FakeBridge:
    def __init__(self, *, identity=None, duplicate_uuid=False, unknown_alias=False,
                 duplicate_alias=False, invalid_rank=False, malformed_insert=False,
                 malformed_search=False, transport_error=False,
                 permute_ann=False, empty_exact=False, underfill=None,
                 tie_scores=False):
        self.writes = []
        self.searches = []
        self.diagnostic_searches = []
        self.identity = identity
        self.duplicate_uuid = duplicate_uuid
        self.unknown_alias = unknown_alias
        self.duplicate_alias = duplicate_alias
        self.invalid_rank = invalid_rank
        self.malformed_insert = malformed_insert
        self.malformed_search = malformed_search
        self.transport_error = transport_error
        self.permute_ann = permute_ann
        self.empty_exact = empty_exact
        self.underfill = underfill
        self.tie_scores = tie_scores

    def post(self, route, payload):
        if self.transport_error:
            raise OSError("simulated bridge transport error")
        if route == "/add_message":
            self.writes.append(deepcopy(payload))
            if self.malformed_insert:
                return {"status": "ok"}
            if self.duplicate_uuid and len(self.writes) > 1:
                return {"id": "duplicate-uuid-constant"}
            return {"id": f"uuid-{len(self.writes)}"}
        if route == "/search_messages":
            self.searches.append(deepcopy(payload))
            # Model Neo4j ANN: select the global top-k before the bridge filters by session.
            global_hits = [f"uuid-{i+1}" for i, item in enumerate(self.writes)][:payload["limit"]]
            ids = [raw_id for raw_id in global_hits
                   if self.writes[int(raw_id.removeprefix("uuid-"))-1]["session_id"] == payload["session_id"]]
            return [{"id": raw_id, "score": 0.9-i*.1} for i, raw_id in enumerate(ids)]
        if route == "/h5/diagnostic/search":
            self.diagnostic_searches.append(deepcopy(payload))
            if self.malformed_search:
                return {"ann": [], "exact": []}
            payload = dict(payload)
            ann = self.post("/search_messages", payload)
            aliases = [payload["candidate_aliases"][hit["id"]] for hit in ann if hit["id"] in payload["candidate_aliases"]]
            if self.unknown_alias:
                aliases.append("unknown-candidate-id")
            if self.duplicate_alias and aliases:
                aliases.append(aliases[0])
            if self.underfill is not None:
                aliases = aliases[:self.underfill]
            ann_rows = []
            for rank, alias in enumerate(aliases, 1):
                r = rank if not self.invalid_rank else (rank + 1 if rank == 1 else rank)
                score = 0.95 if self.tie_scores else (1.0 - (rank * 0.01))
                ann_rows.append({"alias": alias, "score": score, "rank": r})
            exact_rows = deepcopy(ann_rows)
            if self.permute_ann and len(ann_rows) >= 2:
                ann_rows[0], ann_rows[1] = ann_rows[1], ann_rows[0]
                ann_rows[0]["rank"] = 1
                ann_rows[1]["rank"] = 2
            if self.empty_exact:
                exact_rows = []
            encoder_identity = self.identity or {
                "declared": {
                    "artifact_bundle_id": "ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1",
                    "model_id": "sentence-transformers/all-MiniLM-L6-v2",
                    "revision": "1" * 40,
                    "space_id": "2" * 64,
                },
                "loaded": {
                    "validated_profile": "muninn-minilm-profile-v1",
                    "config_sha256": "3" * 64,
                },
            }
            return {
                "ann": ann_rows,
                "exact": exact_rows,
                "encoder_identity": deepcopy(encoder_identity),
            }
        raise AssertionError(route)


class RunnerTests(unittest.TestCase):
    def setUp(self):
        root = Path(__file__).parents[1]
        self.corpus = json.loads((root / "fixtures/retrieval-pilot-v1-corpus.json").read_text())
        template = (root / "fixtures/retrieval-pilot-v1-annotation-template.csv").read_text()
        rows = list(csv.DictReader(io.StringIO(template)))
        for row in rows:
            row.update(reviewer_id="reviewer-k7m2", grade="2")
        out = io.StringIO(newline="")
        writer = csv.DictWriter(out, fieldnames=rows[0].keys(), lineterminator="\n")
        writer.writeheader(); writer.writerows(rows)
        self.csv_text = out.getvalue()
        self.bridge = FakeBridge()
        self.kwargs = dict(
            corpus=self.corpus, annotation_csv=self.csv_text,
            raw_csv_sha256=hashlib.sha256(self.csv_text.encode()).hexdigest(),
            bridge_identity="sha256:" + "a"*64, model_bundle_id="ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1",
            source_build_identity="git:abc123", bridge=self.bridge, limit=5, threshold=0.0,
        )

    def test_validates_all_inputs_before_http(self):
        args = dict(self.kwargs, corpus={})
        with self.assertRaises(ValueError): run_evaluation(**args)
        self.assertEqual(self.bridge.writes, [])
        args = dict(self.kwargs, annotation_csv="bad")
        with self.assertRaises(ValueError): run_evaluation(**args)
        self.assertEqual(self.bridge.writes, [])
        for invalid in (dict(limit=101), dict(threshold=1.1), dict(bridge_identity="")):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                run_evaluation(**dict(self.kwargs, **invalid))
            self.assertEqual(self.bridge.writes, [])

    def test_diagnostic_response_requires_identity_in_the_same_allowlisted_response(self):
        from retrieval_h5_runner import sanitize_diagnostic

        response = {"ann": [], "exact": []}
        with self.assertRaises(ValueError):
            sanitize_diagnostic(response, set())

    def test_protocol_metric_serialization_requires_explicit_k_and_does_not_emit_acceptance(self):
        report = serialize_protocol_gate_metrics(
            [{"query_id": "q", "ann": [{"id": "x"}], "exact": [{"id": "x"}]}],
            [{"query_id": "q", "candidates": {"x": 2}, "ranked": [{"id": "x", "score": 1}]}],
            k=1,
        )
        self.assertEqual(report["ann_exact"]["per_query"][0]["metrics"]["ordered_top_k_match"], 1)
        self.assertNotIn("status", report["ann_exact"])
        with self.assertRaises(ValueError):
            serialize_protocol_gate_metrics([], [], k=0)

    def test_writes_candidates_maps_ids_searches_queries_and_emits_safe_metrics(self):
        report = run_evaluation(**self.kwargs)
        self.assertEqual(len(self.bridge.writes), 12)
        self.assertEqual(len(self.bridge.searches), 6)
        self.assertEqual(len(self.bridge.diagnostic_searches), 6)
        self.assertEqual(len(report["ann_exact_diagnostic"]["queries"]), 6)
        self.assertTrue(all(item["session_id"].startswith("muninn-h5-retrieval-pilot-v1-") for item in self.bridge.writes))
        self.assertTrue(all(item["limit"] == 12 and item["threshold"] == 0.0 for item in self.bridge.searches))
        self.assertEqual(len(self.bridge.diagnostic_searches), len(self.bridge.searches))
        self.assertTrue(all(set(item["candidate_aliases"].values()) ==
                            {candidate["id"] for candidate in self.corpus["queries"][i]["candidates"]}
                            for i, item in enumerate(self.bridge.diagnostic_searches)))
        self.assertEqual(report["ks"], [1, 3, 5])
        self.assertTrue(all(len(item["metrics"]["ordered_ids"]) == 2 for item in report["per_query"]))
        self.assertEqual(len(set(hit for item in report["per_query"] for hit in item["metrics"]["ordered_ids"])), 12)
        encoded = json.dumps(report)
        self.assertNotIn("uuid-", encoded)
        self.assertNotIn(self.corpus["queries"][0]["text"], encoded)
        self.assertNotIn(self.corpus["queries"][0]["candidates"][0]["text"], encoded)
        self.assertNotIn("embedding", encoded)
        self.assertEqual(report["state"], "measured_not_accepted")
        self.assertTrue(all(query["identity_provenance"]["status"] == "partial"
                            for query in report["ann_exact_diagnostic"]["queries"]))
        self.assertTrue(all(query["identity_provenance"]["attestation"] is False
                            for query in report["ann_exact_diagnostic"]["queries"]))
        self.assertEqual(report["provenance"]["corpus_hash"], corpus_hash(self.corpus))
        self.assertEqual(report["annotation_provenance"]["raw_csv_sha256"], self.kwargs["raw_csv_sha256"])
        self.assertEqual(report["provenance"]["identity_verification"]["bridge_identity"], "caller_supplied_not_verified")
        self.assertTrue(report["provenance"]["current_source"].startswith("sha256:"))
        self.assertNotIn("content", report["ann_exact_diagnostic"])
        self.assertNotIn("vector", report["ann_exact_diagnostic"])
        for query, result in zip(self.corpus["queries"], report["per_query"]):
            expected = {candidate["id"] for candidate in query["candidates"]}
            self.assertEqual(set(result["metrics"]["ordered_ids"]), expected)

    def test_rejects_bad_result_id_without_hiding_content(self):
        class Bad(FakeBridge):
            def post(self, route, payload):
                if route == "/add_message": return {"id": "valid-uuid"}
                return [{"id": "outside-query", "score": .8}]
        with self.assertRaises(ValueError): run_evaluation(**dict(self.kwargs, bridge=Bad()))

    def test_rejects_duplicate_search_results(self):
        class Duplicate(FakeBridge):
            def post(self, route, payload):
                result = super().post(route, payload)
                if route == "/search_messages":
                    return result + result[:1]
                return result
        with self.assertRaisesRegex(ValueError, "unknown or duplicate candidate"):
            run_evaluation(**dict(self.kwargs, bridge=Duplicate()))

    def test_endpoint_must_be_loopback(self):
        for endpoint in ("http://example.com:8080", "http://192.168.1.5:8080", "https://127.0.0.1:8080"):
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError): validate_endpoint(endpoint)
        self.assertEqual(validate_endpoint("http://127.0.0.1:18080"), "http://127.0.0.1:18080")

    def test_internal_alias_requires_exact_runner_and_project_markers(self):
        marked = {"MUNINN_H5_DISPOSABLE": "muninn-h5-v1",
                  "MUNINN_H5_COMPOSE_PROJECT": "muninn-h5-pilot-v1",
                  "MUNINN_H5_INTERNAL_RUNNER": "muninn-h5-v1",
                  "MUNINN_BRIDGE_TOKEN": "test-token"}
        self.assertEqual(runner_connection("http://bridge:8080", marked)[0], "http://bridge:8080")
        for key, value in (("MUNINN_H5_INTERNAL_RUNNER", ""),
                           ("MUNINN_H5_DISPOSABLE", "wrong"),
                           ("MUNINN_H5_COMPOSE_PROJECT", "wrong")):
            with self.subTest(key=key), self.assertRaises(ValueError):
                runner_connection("http://bridge:8080", dict(marked, **{key: value}))

    def test_http_bridge_accepts_only_explicit_internal_alias_mode(self):
        with self.assertRaises(ValueError):
            HttpBridge("http://bridge:8080", "test-token")
        bridge = HttpBridge("http://bridge:8080", "test-token", allow_internal=True)
        self.assertEqual(bridge.endpoint, "http://bridge:8080")
        with self.assertRaises(ValueError):
            HttpBridge("http://example.com:8080", "test-token", allow_internal=True)

    def test_internal_runner_still_rejects_other_remote_hosts(self):
        marked = {"MUNINN_H5_DISPOSABLE": "muninn-h5-v1",
                  "MUNINN_H5_COMPOSE_PROJECT": "muninn-h5-pilot-v1",
                  "MUNINN_H5_INTERNAL_RUNNER": "muninn-h5-v1",
                  "MUNINN_BRIDGE_TOKEN": "test-token"}
        for endpoint in ("http://example.com:8080", "http://192.168.1.2:8080", "http://bridge:8081"):
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError):
                runner_connection(endpoint, marked)

    def test_token_is_read_from_file(self):
        from tempfile import TemporaryDirectory
        with TemporaryDirectory() as directory:
            token_path = Path(directory) / "token"
            token_path.write_text("file-token\n")
            env = {"MUNINN_H5_DISPOSABLE": "muninn-h5-v1",
                   "MUNINN_H5_COMPOSE_PROJECT": "muninn-h5-pilot-v1",
                   "MUNINN_H5_INTERNAL_RUNNER": "muninn-h5-v1",
                   "MUNINN_BRIDGE_TOKEN_FILE": str(token_path)}
            endpoint, token = runner_connection("http://bridge:8080", env)
            self.assertEqual(endpoint, "http://bridge:8080")
            self.assertEqual(token, "file-token")

    def test_h5_compose_stages_bundle_in_owned_disposable_volume(self):
        compose = (Path(__file__).parents[1] / "compose.h5.yaml").read_text()
        self.assertIn("create_host_path: false", compose)
        self.assertIn("cp -R --no-preserve=mode,ownership /source/. /artifacts/", compose)
        self.assertIn("find /artifacts -type d -exec chmod 0500", compose)
        self.assertIn("find /artifacts -type f -exec chmod 0400", compose)
        self.assertLess(compose.index("find /artifacts -type f -exec chmod 0400"), compose.index("chown -R 7474:7474 /artifacts"))
        self.assertIn("artifacts:/artifacts:ro", compose)
        self.assertIn("name: muninn-h5-pilot-v1", compose)
        self.assertIn("  artifacts: {}", compose)
        self.assertIn("profiles: [evaluation]", compose)
        self.assertNotIn("\n    ports:", compose)
        self.assertNotIn("ports:", compose)
        self.assertIn("/tmp:rw", compose)
        self.assertIn("runner-output:/out", compose)
        initializer = compose.split("\n  runner-output-init:\n", 1)[1].split("\n  runner:\n", 1)[0]
        self.assertIn("profiles: [evaluation]", initializer)
        self.assertIn("image: ${MUNINN_H5_BRIDGE_IMAGE", initializer)
        self.assertIn("network_mode: none", initializer)
        self.assertIn("user: \"0:0\"", initializer)
        self.assertIn("cap_drop: [ALL]", initializer)
        self.assertIn("cap_add: [CHOWN, FOWNER]", initializer)
        self.assertIn("chown 7474:7474 /out", initializer)
        self.assertIn("chmod 0700 /out", initializer)
        self.assertEqual(initializer.count("runner-output:/out"), 1)
        self.assertNotIn("/bridge", initializer)
        self.assertIn("credentials:/run/secrets:ro", compose)
        runner = compose.split("\n  runner:\n", 1)[1].split("\nnetworks:", 1)[0]
        self.assertIn("networks: [h5]", runner)
        self.assertIn("http://bridge:8080", runner)
        self.assertIn("read_only: true", runner)
        self.assertIn("cap_drop: [ALL]", runner)
        self.assertIn("no-new-privileges:true", runner)
        self.assertIn('"/tmp:rw,noexec,nosuid,size=16m"', runner)
        self.assertIn("MUNINN_BRIDGE_TOKEN_FILE: /run/secrets/bridge_token", runner)
        self.assertIn("runner-output-init:\n        condition: service_completed_successfully", runner)
        self.assertNotIn("MUNINN_BRIDGE_TOKEN:", runner)
        for pathvar in ("MUNINN_H5_CORPUS_FILE", "MUNINN_H5_ANNOTATIONS_FILE",
                        "MUNINN_H5_RUNNER_FILE", "MUNINN_H5_QUALITY_MODULE_FILE"):
            self.assertIn("${" + pathvar + ":?", runner)
        self.assertIn("target: /runner/retrieval_h5_runner.py", runner)
        self.assertIn("target: /runner/retrieval_quality.py", runner)
        self.assertIn("network_mode: none", compose)
        self.assertIn("  credentials:\n    image:", compose)
        credentials = compose.split("  credentials:\n", 1)[1].split("networks:\n", 1)[0]
        self.assertIn("network_mode: none", credentials)
        self.assertIn("target: /neo4j_auth", credentials)
        self.assertIn("target: /bridge_token", credentials)
        self.assertIn("- credentials:/credentials", credentials)
        self.assertIn("cap_add: [CHOWN, DAC_OVERRIDE, FOWNER]", credentials)
        self.assertIn("chown 7474:7474 /credentials/neo4j_auth /credentials/bridge_token", credentials)
        self.assertIn("chmod 0400 /credentials/neo4j_auth /credentials/bridge_token", credentials)
        self.assertIn("credentials:/run/secrets:ro", compose)
        self.assertIn("condition: service_completed_successfully", compose)
        self.assertNotIn("\nsecrets:\n", compose)
        self.assertIn("/data:uid=7474,gid=7474,mode=0700", compose)
        self.assertIn("/logs:uid=7474,gid=7474,mode=0700", compose)
        self.assertIn("${MUNINN_H5_NEO4J_AUTH_FILE:?Set dedicated H5 Neo4j auth file}", compose)
        self.assertIn("${MUNINN_H5_BRIDGE_TOKEN_FILE:?Set dedicated H5 bridge token file}", compose)
        self.assertNotIn(".cache/neo4j.auth", compose)
        self.assertNotIn(".cache/bridge.token", compose)
        bridge = compose.split("\n  bridge:\n", 1)[1].split("\n  artifacts:\n", 1)[0]
        self.assertIn("credentials:\n        condition: service_completed_successfully", bridge)
        neo4j = compose.split("\n  neo4j:\n", 1)[1].split("\n  bridge:\n", 1)[0]
        self.assertIn("credentials:\n        condition: service_completed_successfully", neo4j)


class PilotCompatibilityTests(unittest.TestCase):
    def test_cli_preserves_annotation_requirement_and_descriptive_pilot_report(self):
        from contextlib import redirect_stderr
        from tempfile import TemporaryDirectory
        from unittest.mock import patch
        from retrieval_h5_runner import main

        corpus = json.loads((Path(__file__).parents[1] / "fixtures/retrieval-pilot-v1-corpus.json").read_text())
        with TemporaryDirectory() as directory:
            root = Path(directory)
            corpus_path = root / "corpus.json"
            annotations_path = root / "annotations.csv"
            output_path = root / "report.json"
            corpus_path.write_text(json.dumps(corpus))
            annotations_path.write_text("required pilot annotations")
            args = ["--corpus", str(corpus_path), "--annotations", str(annotations_path),
                    "--raw-csv-sha256", "0" * 64, "--bridge-identity", "bridge",
                    "--model-bundle-id", "bundle", "--source-build-identity", "build",
                    "--endpoint", "http://127.0.0.1:18080", "--limit", "5",
                    "--threshold", "0", "--output", str(output_path)]
            report = {"state": "measured_not_accepted", "acceptance": "descriptive_only"}
            with patch("retrieval_h5_runner.runner_connection", return_value=("http://127.0.0.1:18080", "token")), \
                 patch("retrieval_h5_runner.HttpBridge"), \
                 patch("retrieval_h5_runner.run_evaluation", return_value=report) as evaluate:
                self.assertEqual(0, main(args))
            self.assertEqual(report, json.loads(output_path.read_text()))
            evaluate.assert_called_once()
            self.assertEqual(corpus, evaluate.call_args.kwargs["corpus"])
            self.assertEqual("required pilot annotations", evaluate.call_args.kwargs["annotation_csv"])

            output_path.unlink()
            args.remove("--annotations")
            args.remove(str(annotations_path))
            stderr = io.StringIO()
            with redirect_stderr(stderr), self.assertRaises(SystemExit) as missing_annotations:
                main(args)
            self.assertEqual(2, missing_annotations.exception.code)
            self.assertIn("required", stderr.getvalue())
            self.assertFalse(output_path.exists())

    def test_cli_write_oserror_returns_two_without_claiming_persistence(self):
        from contextlib import redirect_stderr
        from tempfile import TemporaryDirectory
        from unittest.mock import patch
        from retrieval_h5_runner import main

        corpus = json.loads((Path(__file__).parents[1] / "fixtures/retrieval-pilot-v1-corpus.json").read_text())
        with TemporaryDirectory() as directory:
            root = Path(directory)
            corpus_path = root / "corpus.json"
            annotations_path = root / "annotations.csv"
            output_path = root / "report.json"
            corpus_path.write_text(json.dumps(corpus))
            annotations_path.write_text("required pilot annotations")
            args = ["--corpus", str(corpus_path), "--annotations", str(annotations_path),
                    "--raw-csv-sha256", "0" * 64, "--bridge-identity", "bridge",
                    "--model-bundle-id", "bundle", "--source-build-identity", "build",
                    "--endpoint", "http://127.0.0.1:18080", "--limit", "5",
                    "--threshold", "0", "--output", str(output_path)]
            stderr = io.StringIO()
            with patch("retrieval_h5_runner.runner_connection", return_value=("http://127.0.0.1:18080", "token")), \
                 patch("retrieval_h5_runner.HttpBridge"), \
                 patch("retrieval_h5_runner.run_evaluation", return_value={
                     "state": "measured_not_accepted", "acceptance": "descriptive_only"}), \
                 patch("pathlib.Path.write_text", side_effect=OSError("simulated output failure")), \
                 redirect_stderr(stderr):
                self.assertEqual(2, main(args))
            self.assertIn("H5 runner refused: simulated output failure", stderr.getvalue())

    def test_protocol_preflight_remains_diagnostic_without_pass(self):
        from retrieval_h5_runner import protocol_preflight

        class NeverBridge:
            def post(self, *_args, **_kwargs):
                raise AssertionError("protocol_preflight must not call the bridge")

        report = protocol_preflight(ProtocolPreflightTests().complete_protocol(), NeverBridge())
        self.assertFalse(report["measurement_started"])
        self.assertNotIn("PASS", json.dumps(report))
        for gate in report["gates"].values():
            self.assertEqual("blocked", gate["status"])
            self.assertEqual("not_evaluated", gate["results"]["status"])


if __name__ == "__main__": unittest.main()

class ProtocolPreflightTests(unittest.TestCase):
    def encoder_identity(self):
        return {
            "declared": {"artifact_bundle_id": "a" * 64,
                         "model_id": "sentence-transformers/all-MiniLM-L6-v2",
                         "revision": "b" * 40, "space_id": "c" * 64},
            "loaded": {"validated_profile": "muninn-minilm-profile-v1",
                       "config_sha256": "d" * 64},
        }

    def complete_protocol(self):
        def approved(value):
            return {"status": "approved", "value": value, "rationale": "Structural runner test only"}
        protocol = {
            "schema_version": "h5-acceptance/v1", "protocol_id": "test-run", "protocol_version": "1",
            "repository_review": {"commit": "abc", "review_reference": "review-1",
                                  "approved_at": "2026-01-01T00:00:00Z",
                                  "approved_gates": ["ann_exact", "absolute_relevance"]},
            "run_inputs": {
                "metrics_and_k": approved({"k_by_metric": {metric: 10 for metric in
                    ("ExactTopKRecall@K", "OrderedTopKMatch@K", "Recall@K", "nDCG@K")}}),
                "acceptance_rules_and_tolerances": approved({gate: {"criterion": "structural only", "threshold": 0.5}
                    for gate in ("ann_exact", "absolute_relevance")}),
                "run_count_and_aggregation": approved({"run_count": 1, "aggregation": "structural only",
                    "tie_policy": "structural only", "undefined_policy": "structural only"}),
                "corpus_and_query_candidate_scope": approved({"corpus_id": "test-corpus", "version": "test-version",
                    "queries": [{"query_id": "q1", "candidate_ids": ["c1"]}]}),
                "labels_rubric_and_adjudication": approved({"labels_version": "test-labels", "rubric": "structural only",
                    "adjudication": "structural only"}),
                "ann_exact_comparison_scope": approved({"retrieval_path": "structural only", "candidate_scope": "structural only"}),
                "provenance": approved({key: "structural only" for key in ("encoder_model", "build_bridge", "dependencies",
                    "corpus_labels", "retrieval_configuration", "runtime_environment")}),
            },
            "gates": {"ann_exact": True, "absolute_relevance": True},
        }
        protocol["run_inputs"]["encoder_identity"] = approved(self.encoder_identity())
        return protocol

    def test_incomplete_run_protocol_blocks_before_bridge_and_returns_independent_gate_states(self):
        from retrieval_h5_runner import protocol_preflight
        from retrieval_h5_protocol import validate_protocol
        class NeverBridge:
            def post(self, *_args, **_kwargs):
                raise AssertionError("blocked preflight must not contact the bridge")
        protocol = {
            "schema_version": "h5-acceptance/v1", "protocol_id": "draft", "protocol_version": "draft",
            "repository_review": {"commit": "pending", "review_reference": "pending",
                                  "approved_at": "2026-01-01T00:00:00Z", "approved_gates": []},
            "run_inputs": {key: {"status": "pending", "value": None, "rationale": "Awaiting approval"} for key in ("metrics_and_k", "acceptance_rules_and_tolerances",
                "run_count_and_aggregation", "corpus_and_query_candidate_scope",
                "labels_rubric_and_adjudication", "ann_exact_comparison_scope", "encoder_identity", "provenance")},
            "gates": {"ann_exact": True, "absolute_relevance": True},
        }
        result = protocol_preflight(protocol, NeverBridge())
        self.assertEqual("blocked", result["gates"]["ann_exact"]["status"])
        self.assertEqual("blocked", result["gates"]["absolute_relevance"]["status"])
        self.assertNotIn("overall_status", result)
        self.assertFalse(validate_protocol(protocol)["confirmatory_ready"])
        self.assertTrue(result["protocol"]["findings"]["pending"])
        self.assertEqual(result["protocol"]["findings"]["pending"],
                         result["gates"]["ann_exact"]["protocol"]["findings"]["pending"])

    def test_preflight_reports_local_provenance_per_gate_and_never_claims_acceptance(self):
        from retrieval_h5_runner import protocol_preflight
        from retrieval_h5_provenance import collect_observations

        class FakeEngine:
            def inspect(self, container_id):
                return [{"Id": container_id, "Image": "sha256:image", "Config": {"Labels": {}},
                         "State": {"Status": "running"}, "Mounts": []}]

        observations = collect_observations(
            FakeEngine(), container_ids=["bridge-container"], files={}, run_id="run-1",
            started_at="2026-01-01T00:00:00Z", finished_at="2026-01-01T00:00:01Z",
            caller_claims={"encoder_loaded_identity": "caller-model"})
        result = protocol_preflight(self.complete_protocol(), provenance_observations=observations,
                                    run_id="run-1")
        self.assertEqual("partial", result["gates"]["ann_exact"]["provenance"]["status"])
        self.assertEqual("partial", result["gates"]["absolute_relevance"]["provenance"]["status"])
        self.assertEqual("blocked", result["gates"]["ann_exact"]["status"])
        self.assertEqual("blocked", result["gates"]["absolute_relevance"]["status"])
        self.assertFalse(result["measurement_started"])
        self.assertNotIn("overall_status", result)
        self.assertIn("encoder_loaded_identity",
                      result["gates"]["ann_exact"]["provenance"]["missing_or_conflicting"])
        self.assertFalse(result["gates"]["ann_exact"]["protocol"]["approved"])

    def test_protocol_gate_declaration_is_not_operator_corroboration(self):
        from retrieval_h5_runner import protocol_preflight
        result = protocol_preflight(self.complete_protocol())
        for gate in result["gates"].values():
            self.assertFalse(gate["protocol"]["approved"])

    def test_protocol_expected_identity_overrides_and_conflicts_with_caller_claim(self):
        from retrieval_h5_runner import protocol_preflight
        from retrieval_h5_provenance import LocalObservations

        observations = LocalObservations({
            "encoder_expected_identity": {
                "value": {**self.encoder_identity(), "unexpected": "caller claim"},
                "source": "caller", "method": "claim", "observed_at": "2026-01-01T00:00:00Z",
            },
        })
        result = protocol_preflight(self.complete_protocol(), provenance_observations=observations)
        self.assertEqual(self.encoder_identity(),
                         result["gates"]["ann_exact"]["provenance"]["observations"][
                             "encoder_expected_identity"]["value"])
        self.assertIn("encoder_expected_identity",
                      result["gates"]["ann_exact"]["provenance"]["missing_or_conflicting"])

    def test_preflight_marks_protocol_hash_conflict_as_insufficient(self):
        from retrieval_h5_runner import protocol_preflight
        from retrieval_h5_provenance import LocalObservations
        observations = LocalObservations({
            "protocol_hash": {"value": "sha256:wrong", "source": "local filesystem",
                              "method": "SHA-256", "observed_at": "2026-01-01T00:00:00Z"},
            "bridge_container_id": {"value": "container", "source": "Docker Engine",
                                     "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "run_id": {"value": "run-1", "source": "runner", "method": "correlation",
                       "observed_at": "2026-01-01T00:00:00Z"},
        })
        result = protocol_preflight(self.complete_protocol(), provenance_observations=observations)
        gate = result["gates"]["ann_exact"]
        self.assertEqual("insufficient", gate["provenance"]["status"])
        self.assertIn("protocol_hash", gate["provenance"]["missing_or_conflicting"])

    def test_protocol_report_does_not_mutate_historical_descriptive_report(self):
        import copy
        from retrieval_h5_runner import protocol_preflight

        historical = {"state": "measured_not_accepted", "acceptance": "descriptive_only",
                      "per_query": [{"query_id": "query-01", "metrics": {"recall": 1.0}}]}
        original = copy.deepcopy(historical)
        protocol = self.complete_protocol()
        result = protocol_preflight(protocol, provenance_observations={}, historical_report=historical)
        self.assertEqual(original, historical)
        self.assertEqual("measured_not_accepted", historical["state"])
        self.assertNotIn("historical_report", result)
        self.assertEqual({"ann_exact", "absolute_relevance"}, set(result["gates"]))

    def test_integrated_gate_report_serializes_supplied_metrics_and_preserves_blocking_states(self):
        from copy import deepcopy
        from retrieval_h5_runner import protocol_preflight

        report = protocol_preflight(self.complete_protocol())
        historical = {"state": "measured_not_accepted", "acceptance": "descriptive_only"}
        report["legacy_historical_report"] = deepcopy(historical)
        untouched = deepcopy(report)
        no_entries = integrate_protocol_gate_metrics(report, ann_queries=[], quality_queries=[], k=2)
        self.assertEqual(no_entries["gates"]["ann_exact"]["status"], "blocked")
        self.assertEqual(no_entries["gates"]["ann_exact"]["results"]["status"], "not_evaluated")
        self.assertEqual(no_entries["gates"]["absolute_relevance"]["results"]["status"], "not_evaluated")
        one_gate = integrate_protocol_gate_metrics(
            report,
            ann_queries=[{"query_id": "q1", "ann": [{"id": "a"}], "exact": [{"id": "a"}]}],
            k=1,
        )
        self.assertIn("per_query", one_gate["gates"]["ann_exact"]["results"])
        self.assertEqual(one_gate["gates"]["absolute_relevance"]["status"], "blocked")
        self.assertEqual(one_gate["gates"]["absolute_relevance"]["results"]["status"], "not_evaluated")

        evaluated = integrate_protocol_gate_metrics(
            report,
            ann_queries=[{"query_id": "q1", "ann": [{"id": "a"}], "exact": [{"id": "a"}]}],
            quality_queries=[{"query_id": "q1", "candidates": {"a": 2},
                              "ranked": [{"id": "a", "score": 1}]}],
            k=1,
        )
        ann_gate = evaluated["gates"]["ann_exact"]
        quality_gate = evaluated["gates"]["absolute_relevance"]
        self.assertEqual(ann_gate["results"]["k"], 1)
        self.assertEqual(ann_gate["results"]["per_query"][0]["metrics"]["exact_top_k_recall"], 1.0)
        self.assertEqual(quality_gate["results"]["per_query"][0]["metrics"]["recall_at_k"], {"1": 1.0})
        self.assertEqual(quality_gate["results"]["per_query"][0]["metrics"]["ndcg_at_k"], {"1": 1.0})
        self.assertEqual(quality_gate["results"]["per_query"][0]["metrics"]["mrr_classification"], "descriptive_only")
        self.assertEqual(ann_gate["status"], "blocked")
        self.assertEqual(quality_gate["status"], "blocked")
        self.assertNotIn("overall_status", evaluated)
        self.assertEqual(report, untouched)
        self.assertEqual(evaluated["legacy_historical_report"], historical)


class ProtocolLocalPreflightTests(unittest.TestCase):
    def fixture(self):
        from test_retrieval_h5_protocol import ProtocolTests
        from retrieval_h5_protocol import canonical_protocol_hash
        from copy import deepcopy
        corpus = {'schema_version': '1.0', 'corpus_id': 'synthetic-u9', 'queries': [
            {'id': f'q{i}', 'text': f'Synthetic need {i}', 'candidates': [
                {'id': f'c{j}', 'text': f'Synthetic candidate {j}'} for j in range(10)]}
            for i in range(2)]}
        protocol = ProtocolTests().v2_protocol()
        inputs = protocol['run_inputs']
        inputs['corpus_and_query_candidate_scope']['value'] = {
            'corpus_id': corpus['corpus_id'], 'version': 'synthetic-version',
            'corpus_hash': corpus_hash(corpus), 'queries': [
                {'query_id': q['id'], 'candidate_ids': [c['id'] for c in q['candidates']]}
                for q in corpus['queries']]}
        inputs['ann_exact_comparison_scope']['value']['search_config']['limit'] = 20
        record = {'protocol_sha256': canonical_protocol_hash(protocol), 'commit': 'abc123',
                  'review_reference': 'review-1', 'corroborated_at': '2026-01-02T00:00:00Z',
                  'corroborated_by': 'synthetic-operator'}
        return deepcopy(protocol), deepcopy(corpus), record

    def check_local(self, protocol, corpus, record, *, run_id='synthetic-run', override=None):
        from retrieval_h5_runner import _validate_protocol_run_inputs
        from unittest.mock import patch
        from contextlib import ExitStack
        with ExitStack() as stack:
            for name in ('run_evaluation', 'annotations_from_csv', 'compose_report',
                         'serialize_gate_metrics', 'gate_result', 'HttpBridge.post'):
                stack.enter_context(patch('retrieval_h5_runner.' + name,
                                          side_effect=AssertionError('unexpected effect')))
            for name in ('pathlib.Path.mkdir', 'pathlib.Path.write_text', 'builtins.open'):
                stack.enter_context(patch(name, side_effect=AssertionError('unexpected filesystem effect')))
            return _validate_protocol_run_inputs(protocol=protocol, corpus=corpus,
                operator_record=record, run_id=run_id, search_config_override=override)

    def test_local_positive_preserves_inputs_and_counts_shared_ids_per_pair(self):
        from copy import deepcopy
        protocol, corpus, record = self.fixture()
        before = deepcopy((protocol, corpus, record))
        result = self.check_local(protocol, corpus, record, override={'limit': 20.0, 'threshold': 0})
        self.assertEqual([], result['findings'])
        self.assertEqual(20, len(result['pairs']))
        self.assertEqual(before, (protocol, corpus, record))
        self.assertTrue(result['protocol']['operator_corroborated'])
        self.assertTrue(result['protocol']['confirmatory_ready'])

    def test_local_rejections_are_safe_deterministic_and_without_effects(self):
        from copy import deepcopy
        from retrieval_h5_protocol import canonical_protocol_hash
        cases = ('absent', 'invalid', 'pending', 'v1', 'gates', 'hash', 'commit', 'review',
                 'altered_text', 'missing_query', 'extra_query', 'duplicate_query',
                 'duplicate_candidate', 'under_ten', 'scope_duplicate', 'scope_missing',
                 'limit', 'run', 'override_extra', 'override_bool', 'override_huge', 'override_nan')
        for case in cases:
            protocol, corpus, record = self.fixture()
            override = None
            run_id = 'synthetic-run'
            if case == 'absent': protocol = None
            elif case == 'invalid': protocol = {'sensitive-arbitrary-key': 'private'}
            elif case == 'pending': protocol['run_inputs']['encoder_identity'] = {
                'status': 'pending', 'value': None, 'rationale': 'Awaiting synthetic review'}
            elif case == 'v1': protocol['schema_version'] = 'h5-acceptance/v1'
            elif case == 'gates': protocol['gates']['absolute_relevance'] = True
            elif case in ('hash', 'commit', 'review'):
                record[{'hash': 'protocol_sha256', 'commit': 'commit', 'review': 'review_reference'}[case]] = 'different'
            elif case == 'altered_text': corpus['queries'][0]['text'] = 'private altered text'
            elif case == 'missing_query': corpus['queries'].pop()
            elif case == 'extra_query':
                query = deepcopy(corpus['queries'][0]); query['id'] = 'q-extra'; corpus['queries'].append(query)
            elif case == 'duplicate_query': corpus['queries'].append(deepcopy(corpus['queries'][0]))
            elif case == 'duplicate_candidate':
                corpus['queries'][0]['candidates'].append(deepcopy(corpus['queries'][0]['candidates'][0]))
            elif case == 'under_ten': corpus['queries'][0]['candidates'].pop()
            elif case.startswith('scope_'):
                ids = protocol['run_inputs']['corpus_and_query_candidate_scope']['value']['queries'][0]['candidate_ids']
                ids.append(ids[0]) if case == 'scope_duplicate' else ids.pop()
            elif case == 'limit':
                protocol['run_inputs']['ann_exact_comparison_scope']['value']['search_config']['limit'] = 10
            elif case == 'run': run_id = 'PENDING'
            else:
                override = {'limit': 20, 'threshold': 0}
                if case == 'override_extra': override['private-key'] = 'private'
                elif case == 'override_bool': override['threshold'] = False
                elif case == 'override_huge': override['limit'] = 10**400
                else: override['threshold'] = float('nan')
            # Scope/limit changes are corroborated synthetically to reach that boundary.
            if case in ('scope_duplicate', 'scope_missing', 'limit'):
                record['protocol_sha256'] = canonical_protocol_hash(protocol)
            with self.subTest(case=case):
                result = self.check_local(protocol, corpus, record, run_id=run_id, override=override)
                self.assertTrue(result['findings'])
                self.assertEqual(result['findings'], self.check_local(
                    protocol, corpus, record, run_id=run_id, override=override)['findings'])
                self.assertNotIn('private', json.dumps(result))
                if case == 'pending':
                    self.assertTrue(result['protocol']['findings']['pending'])


class ProtocolEvidencePreflightTests(unittest.TestCase):
    def fixture(self):
        from copy import deepcopy
        from retrieval_h5_provenance import LocalObservations
        from test_retrieval_h5_provenance import ProvenanceTests
        protocol, corpus, operator = ProtocolLocalPreflightTests().fixture()
        records = ProvenanceTests().prior_observations()
        values = {'protocol_hash': operator['protocol_sha256'], 'review_reference': 'review-1',
                  'operator_corroboration': deepcopy(operator), 'corpus_hash': corpus_hash(corpus),
                  'run_id': 'synthetic-run', 'bridge_container_id': 'synthetic-container',
                  'encoder_expected_identity': deepcopy(protocol['run_inputs']['encoder_identity']['value']),
                  'corpus_freeze_review': {'corpus_id': 'synthetic-u9', 'version': 'synthetic-version',
                      'corpus_hash': corpus_hash(corpus), 'status': 'frozen', 'review_reference': 'corpus-review'},
                  'storage_scope': {'run_id': 'synthetic-run', 'bridge_container_id': 'synthetic-container',
                      'storage_id': 'synthetic-storage', 'isolation': 'run-exclusive-disposable-v1',
                      'observed_candidates': [], 'planned_candidates': [
                          {'query_id': f'q{i}', 'candidate_id': f'c{j}'} for i in range(2) for j in range(10)]},
                  'effective_config': {'run_id': 'synthetic-run', 'retrieval_path': '/h5/diagnostic/search',
                      'candidate_scope': 'same-query-candidates-v1', 'search_config': {'limit': 20.0, 'threshold': 0}}}
        records.update({key: {'value': value, 'source': 'explicit synthetic acquisition',
                             'method': 'offline fake', 'observed_at': 'synthetic-clock'}
                        for key, value in values.items()})
        return protocol, corpus, operator, LocalObservations(records)

    @staticmethod
    def install_pilot_and_filesystem_spies(stack, patch):
        for name in ('run_evaluation', 'annotations_from_csv', 'compose_report', 'protocol_preflight'):
            stack.enter_context(patch('retrieval_h5_runner.' + name,
                                      side_effect=AssertionError('unexpected pilot effect')))
        for name in ('builtins.open', 'pathlib.Path.open', 'pathlib.Path.read_text',
                     'pathlib.Path.read_bytes', 'pathlib.Path.mkdir', 'pathlib.Path.write_text',
                     'pathlib.Path.write_bytes'):
            stack.enter_context(patch(name, side_effect=AssertionError('unexpected filesystem effect')))

    @staticmethod
    def install_no_effect_spies(stack, patch):
        for name in ('run_evaluation', 'annotations_from_csv', 'compose_report',
                     'serialize_gate_metrics', 'serialize_protocol_gate_metrics', 'gate_result',
                     'integrate_protocol_gate_metrics', 'protocol_preflight'):
            stack.enter_context(patch('retrieval_h5_runner.' + name,
                                      side_effect=AssertionError('unexpected pilot, metric, or gate effect')))
        for name in ('builtins.open', 'pathlib.Path.open', 'pathlib.Path.read_text',
                     'pathlib.Path.read_bytes', 'pathlib.Path.mkdir', 'pathlib.Path.write_text',
                     'pathlib.Path.write_bytes'):
            stack.enter_context(patch(name, side_effect=AssertionError('unexpected filesystem effect')))

    def invoke(self, protocol, corpus, operator, records, *, error=None):
        from contextlib import ExitStack
        from copy import deepcopy
        from tempfile import TemporaryDirectory
        from unittest.mock import Mock, patch
        from retrieval_h5_runner import run_protocol_evaluation
        before = deepcopy((protocol, corpus, operator, records))
        def acquire(**kwargs):
            self.assertEqual(protocol, kwargs['protocol'])
            kwargs['protocol'].clear(); kwargs['corpus'].clear()
            if error: raise error('private-secret')
            return records
        evidence = Mock(); evidence.before_run.side_effect = acquire
        from retrieval_h5_provenance import LocalObservations
        evidence.after_response.return_value = LocalObservations({})
        evidence.after_run.return_value = LocalObservations({})
        expected_identity = (
            protocol.get('run_inputs', {}).get('encoder_identity', {}).get('value')
            if type(protocol) is dict and type(protocol.get('run_inputs')) is dict
            and type(protocol['run_inputs'].get('encoder_identity')) is dict else {}
        )
        bridge = FakeBridge(identity=expected_identity)
        with TemporaryDirectory() as directory:
            sentinel = Path(directory) / 'sentinel'; sentinel.write_bytes(b'unchanged')
            with ExitStack() as stack:
                self.install_pilot_and_filesystem_spies(stack, patch)
                result = run_protocol_evaluation(protocol=protocol, corpus=corpus,
                    operator_record=operator, run_id='synthetic-run', bridge=bridge, evidence=evidence)
            self.assertEqual(b'unchanged', sentinel.read_bytes())
        if result['preflight']['status'] == 'blocked':
            self.assertEqual(0, len(bridge.writes))
            self.assertEqual(0, len(bridge.diagnostic_searches))
            evidence.after_response.assert_not_called(); evidence.after_run.assert_not_called()
            self.assertFalse(result['measurement_started'])
            self.assertEqual({'status': 'blocked', 'findings': result['preflight']['findings']}, result['preflight'])
            self.assertTrue(result['preflight']['findings'])
            self.assertEqual({'status': 'blocked', 'results': {}}, result['gates']['ann_exact'])
        else:
            self.assertEqual(20, len(bridge.writes))
            self.assertEqual(2, len(bridge.diagnostic_searches))
            self.assertEqual(2, evidence.after_response.call_count)
            evidence.after_run.assert_called_once_with(run_id='synthetic-run')
            self.assertTrue(result['measurement_started'])
            self.assertEqual({'status': 'eligible', 'findings': []}, result['preflight'])
            self.assertEqual('blocked', result['gates']['ann_exact']['status'])
        self.assertEqual(before, (protocol, corpus, operator, records))
        self.assertEqual({'status': 'not_evaluated', 'results': {}}, result['gates']['absolute_relevance'])
        self.assertEqual({'preflight', 'gates', 'measurement_started', 'protocol', 'corpus', 'run_id'}, set(result))
        self.assertNotIn('private-secret', json.dumps(result))
        return result, evidence.before_run.call_count

    def test_eligible_stops_and_local_rejection_skips_acquisition(self):
        args = self.fixture()
        result, calls = self.invoke(*args)
        self.assertEqual(1, calls)
        self.assertEqual({'status': 'eligible', 'findings': []}, result['preflight'])
        self.assertEqual('blocked', result['gates']['ann_exact']['status'])
        result, calls = self.invoke(None, *args[1:])
        self.assertEqual(0, calls); self.assertEqual('blocked', result['preflight']['status'])

    def test_closed_records_and_bindings_fail_closed(self):
        from copy import deepcopy
        names = ('corpus_freeze_review', 'storage_scope', 'effective_config', 'operator_corroboration')
        cases = []
        for name in names:
            cases += [(name, 'absent', None), (name, 'extra', None), (name, 'value', []),
                      (name, 'conflict', 'true'), (name, 'source', True)]
            for field in self.fixture()[3][name]['value']:
                cases += [(name, 'missing:' + field, None), (name, 'field:' + field, 'pending')]
        for name in ('protocol_hash', 'review_reference', 'corpus_hash', 'encoder_expected_identity', 'run_id'):
            cases.append((name, 'value', 'private-secret'))
        cases += [('storage_scope', 'field:observed_candidates', ['private-secret']),
                  ('storage_scope', 'field:planned_candidates', [{'query_id': 'q0', 'candidate_id': 'c0'}] * 20),
                  ('effective_config', 'field:search_config', {'limit': True, 'threshold': 0})]
        for name, mode, value in cases:
            with self.subTest(name=name, mode=mode):
                protocol, corpus, operator, records = self.fixture()
                if mode == 'absent': records.pop(name)
                elif mode == 'extra': records[name]['value']['private-secret'] = 'private-secret'
                elif mode.startswith('missing:'): records[name]['value'].pop(mode.split(':')[1])
                elif mode.startswith('field:'): records[name]['value'][mode.split(':')[1]] = value
                else: records[name][mode] = value
                result, calls = self.invoke(protocol, corpus, operator, records)
                self.assertEqual(1, calls); self.assertEqual('blocked', result['preflight']['status'])
                findings = result['preflight']['findings']; self.assertTrue(findings)
                self.assertEqual(findings, sorted(findings, key=lambda f: (f['path'], f['reason'])))
                self.assertEqual({'status': 'blocked', 'results': {}}, result['gates']['ann_exact'])
        for error in (OSError, ValueError, RuntimeError):
            self.assertEqual('blocked', self.invoke(*self.fixture(), error=error)[0]['preflight']['status'])
        args = self.fixture()
        self.assertEqual('blocked', self.invoke(*args[:3], dict(args[3]))[0]['preflight']['status'])

    def test_public_protocol_findings_do_not_expose_unknown_dotted_keys(self):
        protocol, corpus, operator, records = self.fixture()
        protocol['run_inputs']['encoder_identity']['value']['private-secret.with.dots'] = 'sensitive'
        result, calls = self.invoke(protocol, corpus, operator, records)
        self.assertEqual(0, calls)
        self.assertTrue(result['protocol']['findings']['malformed'])

    def test_wrapper_matrix_nested_types_and_numeric_domains(self):
        from copy import deepcopy
        names = ('corpus_freeze_review', 'storage_scope', 'effective_config', 'operator_corroboration')
        for name in names:
            base = self.fixture()[3][name]
            variants = []
            for field in ('value', 'source', 'method', 'observed_at'):
                item = deepcopy(base); item.pop(field); variants.append(item)
            for field in ('source', 'method', 'observed_at'):
                for value in (' \t', [], True, None):
                    item = deepcopy(base); item[field] = value; variants.append(item)
            for value in (True, 0, None):
                item = deepcopy(base); item['conflict'] = value; variants.append(item)
            item = deepcopy(base); item['private-secret'] = 'sensitive'; variants.append(item)
            item = deepcopy(base); item['value'][4] = 'sensitive'; variants.append(item)
            for item in variants:
                with self.subTest(name=name, item=item):
                    protocol, corpus, operator, records = self.fixture(); records[name] = item
                    self.assertEqual('blocked', self.invoke(protocol, corpus, operator, records)[0]['preflight']['status'])
        for limit, threshold in ((True, 0), (20, False), (10**400, 0), (float('inf'), 0),
                                 (20, float('nan')), (20.5, 0), (20, -1)):
            with self.subTest(limit=limit, threshold=threshold):
                protocol, corpus, operator, records = self.fixture()
                records['effective_config']['value']['search_config'] = {'limit': limit, 'threshold': threshold}
                self.assertEqual('blocked', self.invoke(protocol, corpus, operator, records)[0]['preflight']['status'])
        for name, field, value in (('storage_scope', 'observed_candidates', ()),
                                   ('storage_scope', 'storage_id', '/private-secret/path'),
                                   ('storage_scope', 'planned_candidates', [{'query_id': 'q0', 'candidate_id': True}])):
            protocol, corpus, operator, records = self.fixture(); records[name]['value'][field] = value
            self.assertEqual('blocked', self.invoke(protocol, corpus, operator, records)[0]['preflight']['status'])
        protocol, corpus, operator, records = self.fixture()
        for name in names: records[name]['conflict'] = False
        records['storage_scope']['value']['planned_candidates'].reverse()
        self.assertEqual('eligible', self.invoke(protocol, corpus, operator, records)[0]['preflight']['status'])

    def assert_blocked_u9_report(self, result):
        self.assertEqual({'status': 'blocked', 'findings': result['preflight']['findings']},
                         result['preflight'])
        self.assertTrue(result['preflight']['findings'])
        self.assertEqual({'status': 'blocked', 'results': {}}, result['gates']['ann_exact'])
        self.assertEqual({'status': 'not_evaluated', 'results': {}},
                         result['gates']['absolute_relevance'])
        self.assertFalse(result['measurement_started'])

    def raw(self, protocol, corpus, operator, records, **extra):
        # Deep inputs cannot be snapshotted recursively; spies must remain independent of them.
        from contextlib import ExitStack
        from tempfile import TemporaryDirectory
        from unittest.mock import Mock, patch
        import retrieval_h5_runner
        from retrieval_h5_runner import run_protocol_evaluation
        evidence = Mock(); evidence.before_run.return_value = records
        bridge = Mock()
        with TemporaryDirectory() as directory:
            sentinel = Path(directory) / 'sentinel'
            sentinel.write_bytes(b'unchanged')
            with ExitStack() as stack:
                self.install_no_effect_spies(stack, patch)
                if extra.get('gate_effect'):
                    def acquire_with_unexpected_gate(**kwargs):
                        retrieval_h5_runner.gate_result(protocol_complete=True, approved=True,
                            provenance_status='verified', rule_satisfied=True)
                        return records
                    evidence.before_run.side_effect = acquire_with_unexpected_gate
                result = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
                    run_id=extra.get('run_id', 'synthetic-run'), bridge=bridge, evidence=evidence,
                    search_config_override=extra.get('override'))
            self.assertEqual(b'unchanged', sentinel.read_bytes())
        bridge.post.assert_not_called(); evidence.after_response.assert_not_called()
        evidence.after_run.assert_not_called()
        self.assert_blocked_u9_report(result)
        json.dumps(result)
        return result, evidence.before_run.call_count

    @staticmethod
    def nested(depth):
        value = []
        for _ in range(depth):
            value = [value]
        return value

    def test_raw_installs_effect_spies_around_real_blocked_execution(self):
        protocol, corpus, operator, records = self.fixture()
        records['corpus_freeze_review']['value']['status'] = 'draft'
        with self.assertRaisesRegex(AssertionError, 'unexpected pilot, metric, or gate effect'):
            self.raw(protocol, corpus, operator, records, gate_effect=True)

    def test_deep_and_cyclic_inputs_block_with_fixed_findings(self):
        protocol_finding = [{'path': 'protocol', 'reason': 'protocol_invalid'}]
        observation_finding = [{'path': 'observations',
                                'reason': 'preflight_evidence_missing_or_conflicting'}]
        for depth in (200, 450, 900):
            with self.subTest(target='protocol', depth=depth):
                protocol, corpus, operator, records = self.fixture()
                protocol['run_inputs']['provenance']['value']['extra'] = self.nested(depth)
                result, calls = self.raw(protocol, corpus, operator, records)
                self.assertEqual((protocol_finding, 0), (result['preflight']['findings'], calls))
            if depth < 900:
                continue  # U8 values stay polymorphic: no new arbitrary depth limit.
            with self.subTest(target='observations', depth=depth):
                protocol, corpus, operator, records = self.fixture()
                records['readonly_mounts']['value'] = self.nested(depth)
                result, calls = self.raw(protocol, corpus, operator, records)
                self.assertEqual((observation_finding, 1), (result['preflight']['findings'], calls))
        protocol, corpus, operator, records = self.fixture()
        records['readonly_mounts']['value'] = self.nested(450)
        self.assertEqual('eligible', self.invoke(protocol, corpus, operator, records)[0]['preflight']['status'])
        protocol, corpus, operator, records = self.fixture()
        cycle = []; cycle.append(cycle); records['readonly_mounts']['value'] = cycle
        result, calls = self.raw(protocol, corpus, operator, records)
        self.assertEqual((observation_finding, 1), (result['preflight']['findings'], calls))
        protocol, corpus, operator, records = self.fixture()
        corpus['queries'][0]['extra'] = self.nested(900)
        result, calls = self.raw(protocol, corpus, operator, records)
        self.assertEqual(([{'path': 'corpus', 'reason': 'corpus_mismatch'}], 0),
                         (result['preflight']['findings'], calls))
        protocol, corpus, operator, records = self.fixture()
        operator['extra'] = self.nested(900)
        result, calls = self.raw(protocol, corpus, operator, records)
        self.assertEqual(([{'path': 'operator_record', 'reason': 'approval_mismatch'}], 0),
                         (result['preflight']['findings'], calls))

    def test_provider_failure_contract_is_explicit(self):
        from unittest.mock import Mock
        from retrieval_h5_runner import run_protocol_evaluation
        for error in (OSError, ValueError, RuntimeError, RecursionError):
            with self.subTest(error=error.__name__):
                result, _ = self.invoke(*self.fixture(), error=error)
                self.assertEqual([{'path': 'observations',
                                   'reason': 'preflight_evidence_missing_or_conflicting'}],
                                 result['preflight']['findings'])
        protocol, corpus, operator, _ = self.fixture()
        evidence = Mock(); evidence.before_run.side_effect = TypeError('private-provider-detail')
        result = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id='synthetic-run', bridge=Mock(), evidence=evidence)
        self.assertEqual([{'path': 'observations',
                           'reason': 'preflight_evidence_missing_or_conflicting'}],
                         result['preflight']['findings'])
        self.assertNotIn('private-provider-detail', json.dumps(result))
        evidence.before_run.assert_called_once()
        evidence.after_response.assert_not_called(); evidence.after_run.assert_not_called()

    def blocked(self, protocol, corpus, operator, records, expected_calls=1, **extra):
        result, calls = self.raw(protocol, corpus, operator, records, **extra)
        self.assertEqual(expected_calls, calls)
        findings = result['preflight']['findings']
        self.assertEqual(findings, sorted(findings, key=lambda f: (f['path'], f['reason'])))
        self.assertEqual(len(findings), len({(f['path'], f['reason']) for f in findings}))
        self.assertEqual(findings, self.raw(protocol, corpus, operator, records, **extra)[0]['preflight']['findings'])
        self.assertNotIn('private', json.dumps(result))
        return findings

    def test_public_local_rejections_have_exact_categories_without_acquisition(self):
        from retrieval_h5_protocol import canonical_protocol_hash
        scope = 'protocol.run_inputs.corpus_and_query_candidate_scope.value'
        limit = 'protocol.run_inputs.ann_exact_comparison_scope.value.search_config.limit'
        def mutate(name, protocol, corpus, operator):
            ids = protocol['run_inputs']['corpus_and_query_candidate_scope']['value']['queries'][0]['candidate_ids']
            if name == 'v1': protocol['schema_version'] = 'h5-acceptance/v1'
            elif name == 'gates': protocol['gates']['absolute_relevance'] = True
            elif name == 'hash': operator['protocol_sha256'] = 'different'
            elif name == 'commit': operator['commit'] = 'different'
            elif name == 'review': operator['review_reference'] = 'different'
            elif name == 'text': corpus['queries'][0]['text'] = 'private altered text'
            elif name == 'under_ten': corpus['queries'][0]['candidates'].pop()
            elif name == 'scope_dup': ids.append(ids[0]); operator['protocol_sha256'] = canonical_protocol_hash(protocol)
            elif name == 'scope_missing': ids.pop(); operator['protocol_sha256'] = canonical_protocol_hash(protocol)
            elif name == 'limit':
                protocol['run_inputs']['ann_exact_comparison_scope']['value']['search_config']['limit'] = 10
                operator['protocol_sha256'] = canonical_protocol_hash(protocol)
        cases = {'v1': ('protocol.schema_version', 'contract_unsupported'),
                 'gates': ('protocol.gates', 'contract_unsupported'),
                 'hash': ('operator_record', 'approval_mismatch'), 'commit': ('operator_record', 'approval_mismatch'),
                 'review': ('operator_record', 'approval_mismatch'), 'text': (scope, 'corpus_mismatch'),
                 'under_ten': (scope, 'corpus_mismatch'), 'scope_dup': (scope, 'corpus_mismatch'),
                 'scope_missing': (scope, 'corpus_mismatch'), 'limit': (limit, 'scope_unsupported')}
        for name, (path, reason) in cases.items():
            with self.subTest(case=name):
                protocol, corpus, operator, records = self.fixture(); mutate(name, protocol, corpus, operator)
                found = self.blocked(protocol, corpus, operator, records, 0)
                self.assertIn({'path': path, 'reason': reason}, found)
        for run_id, path in (('PENDING', 'run_id'), ('', 'run_id')):
            self.assertEqual([{'path': path, 'reason': 'preflight_evidence_missing_or_conflicting'}],
                             self.blocked(*self.fixture(), 0, run_id=run_id))
        for override in ({'limit': 20, 'threshold': 0, 'private': 1}, {'limit': True, 'threshold': 0},
                         {'limit': 20, 'threshold': float('nan')}, {'limit': 10**400, 'threshold': 0}, {'limit': 19, 'threshold': 0}):
            self.assertEqual([{'path': 'search_config_override', 'reason': 'configuration_mismatch'}],
                             self.blocked(*self.fixture(), 0, override=override))

    def test_each_required_u8_record_missing_or_conflicting_blocks_public_preflight(self):
        from copy import deepcopy
        prior_keys = tuple(self.fixture()[3].keys())
        for key in prior_keys:
            for mode in ('missing', 'conflict'):
                with self.subTest(record=key, mode=mode):
                    protocol, corpus, operator, records = self.fixture()
                    if mode == 'missing':
                        del records[key]
                    else:
                        records[key]['conflict'] = True
                    result, calls = self.invoke(protocol, corpus, operator, records)
                    self.assertEqual(1, calls)
                    self.assert_blocked_u9_report(result)
                    self.assertIn({'path': f'observations.{key}',
                                   'reason': 'preflight_evidence_missing_or_conflicting'},
                                  result['preflight']['findings'])
                    self.assertEqual(result['preflight']['findings'], sorted(
                        result['preflight']['findings'], key=lambda finding: (finding['path'], finding['reason'])))

    def test_corpus_missing_extra_duplicate_block_after_recomputing_protocol_hashes(self):
        from copy import deepcopy
        from retrieval_h5_protocol import canonical_protocol_hash
        cases = ('missing_query', 'extra_query', 'duplicate_query',
                 'missing_candidate', 'extra_candidate', 'duplicate_candidate')
        for case in cases:
            with self.subTest(case=case):
                protocol, corpus, operator, records = self.fixture()
                scope = protocol['run_inputs']['corpus_and_query_candidate_scope']['value']
                if case == 'missing_query':
                    scope['queries'].pop()
                elif case == 'extra_query':
                    scope['queries'].append({'query_id': 'extra-query', 'candidate_ids': ['extra-candidate']})
                elif case == 'duplicate_query':
                    scope['queries'].append(deepcopy(scope['queries'][0]))
                elif case == 'missing_candidate':
                    scope['queries'][0]['candidate_ids'].pop()
                elif case == 'extra_candidate':
                    scope['queries'][0]['candidate_ids'].append('extra-candidate')
                else:
                    scope['queries'][0]['candidate_ids'].append(scope['queries'][0]['candidate_ids'][0])
                if case in ('extra_query', 'extra_candidate'):
                    protocol['run_inputs']['ann_exact_comparison_scope']['value']['search_config']['limit'] = 30
                    records['effective_config']['value']['search_config']['limit'] = 30
                operator['protocol_sha256'] = canonical_protocol_hash(protocol)
                records['protocol_hash']['value'] = operator['protocol_sha256']
                records['operator_corroboration']['value'] = deepcopy(operator)
                if case in ('extra_query', 'extra_candidate'):
                    records['storage_scope']['value']['planned_candidates'] = [
                        {'query_id': query['id'], 'candidate_id': candidate['id']}
                        for query in corpus['queries'] for candidate in query['candidates']]
                findings = self.blocked(protocol, corpus, operator, records, expected_calls=0)
                self.assertEqual([{'path': 'protocol.run_inputs.corpus_and_query_candidate_scope.value',
                                   'reason': 'corpus_mismatch'}], findings)

    def test_raw_effect_spy_rejects_filesystem_read(self):
        from contextlib import ExitStack
        from tempfile import TemporaryDirectory
        from unittest.mock import patch

        with TemporaryDirectory() as directory:
            sentinel = Path(directory) / 'sentinel'
            sentinel.write_text('probe', encoding='utf-8')
            with ExitStack() as stack:
                self.install_no_effect_spies(stack, patch)
                with self.assertRaisesRegex(AssertionError, 'unexpected filesystem effect'):
                    sentinel.read_text(encoding='utf-8')

    def test_duplicate_body_query_and_candidate_reject_before_hash_or_evidence(self):
        from copy import deepcopy

        for duplicate in ('query', 'candidate'):
            with self.subTest(duplicate=duplicate):
                protocol, corpus, operator, records = self.fixture()
                if duplicate == 'query':
                    corpus['queries'].append(deepcopy(corpus['queries'][0]))
                else:
                    corpus['queries'][0]['candidates'].append(
                        deepcopy(corpus['queries'][0]['candidates'][0]))

                from unittest.mock import patch
                with patch('retrieval_h5_runner.corpus_hash',
                           side_effect=AssertionError('hash must follow corpus validation')):
                    result, calls = self.raw(protocol, corpus, operator, records)
                self.assertEqual(0, calls)
                self.assertEqual([{'path': 'corpus', 'reason': 'corpus_mismatch'}],
                                 result['preflight']['findings'])

    def test_local_rejection_matrix_keeps_exactly_nine_candidates_and_valid_hashes(self):
        from copy import deepcopy
        from retrieval_h5_protocol import canonical_protocol_hash

        for query_index in range(2):
            with self.subTest(query_index=query_index):
                protocol, corpus, operator, _records = self.fixture()
                corpus['queries'][query_index]['candidates'].pop()
                candidate_ids = [item['id'] for item in corpus['queries'][query_index]['candidates']]
                protocol_scope = protocol['run_inputs']['corpus_and_query_candidate_scope']['value']
                protocol_scope['queries'][query_index]['candidate_ids'] = candidate_ids
                protocol_scope['corpus_hash'] = corpus_hash(corpus)
                operator['protocol_sha256'] = canonical_protocol_hash(protocol)
                self.assertEqual(9, len(corpus['queries'][query_index]['candidates']))
                self.assertEqual(10, len(corpus['queries'][1 - query_index]['candidates']))
                self.assertEqual(protocol_scope['corpus_hash'], corpus_hash(corpus))
                findings = self.blocked(protocol, corpus, operator, _records, expected_calls=0)
                expected = {'path': 'protocol.run_inputs.corpus_and_query_candidate_scope.value',
                            'reason': 'corpus_mismatch'}
                self.assertIn(expected, findings)
                self.assertEqual(1, len(findings))

    def test_specialized_conflict_deduplicates_identical_u8_finding(self):
        protocol, corpus, operator, records = self.fixture()
        records['storage_scope']['conflict'] = True
        findings = self.blocked(protocol, corpus, operator, records)
        expected = {'path': 'observations.storage_scope',
                    'reason': 'preflight_evidence_missing_or_conflicting'}
        self.assertEqual([expected], findings)

    def test_public_protocol_findings_preserve_structural_groups(self):
        cases = (
            ('missing', 'metrics_and_k', 'missing'),
            ('pending', 'encoder_identity', 'pending'),
            ('malformed', 'encoder_identity', 'malformed'),
        )
        for group, decision, expected_group in cases:
            with self.subTest(group=group):
                protocol, corpus, operator, _records = self.fixture()
                if group == 'missing':
                    protocol['run_inputs'].pop(decision)
                elif group == 'pending':
                    protocol['run_inputs'][decision] = {
                        'status': 'pending', 'value': None, 'rationale': 'awaiting synthetic review'}
                else:
                    protocol['run_inputs'][decision]['value']['loaded'] = []
                result, calls = self.invoke(protocol, corpus, operator, _records)
                self.assertEqual(0, calls)
                structural = result['protocol']['findings'][expected_group]
                self.assertTrue(structural)
                self.assertTrue(all(set(finding) == {'path', 'reason'} for finding in structural))
                self.assertTrue(all(finding['path'] and finding['reason'] for finding in structural))
                validation = __import__('retrieval_h5_protocol').validate_protocol(protocol)
                self.assertEqual(validation[expected_group], structural)
                self.assertEqual('blocked', result['preflight']['status'])

    def test_valid_specialized_metadata_is_preserved_without_time_policy(self):
        protocol, corpus, operator, records = self.fixture()
        metadata = {}
        for name in ('corpus_freeze_review', 'storage_scope', 'effective_config', 'operator_corroboration'):
            record = records[name]
            record['source'] = '  synthetic source  '
            record['method'] = '\t synthetic method \t'
            record['observed_at'] = 'not-a-timestamp-at-all'
            metadata[name] = (record['source'], record['method'], record['observed_at'])
        result, calls = self.invoke(protocol, corpus, operator, records)
        self.assertEqual(1, calls)
        self.assertEqual('eligible', result['preflight']['status'])
        self.assertEqual(metadata, {name: (records[name]['source'], records[name]['method'],
                                           records[name]['observed_at']) for name in metadata})

    def test_provider_copy_preserves_input_values_without_normalization(self):
        from copy import deepcopy
        from unittest.mock import Mock
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records = self.fixture()
        # Capture the actual callback inputs; no expected value is copied into observations.
        records['corpus_freeze_review']['value']['review_reference'] = '  corpus review  '
        records['corpus_freeze_review']['source'] = '  independent source  '
        expected = deepcopy((protocol, corpus))
        captured = {}
        evidence = Mock()
        def acquire(*, protocol, corpus, run_id):
            captured.update(protocol=protocol, corpus=corpus)
            return records
        evidence.before_run.side_effect = acquire
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id='synthetic-run', bridge=Mock(), evidence=evidence)
        self.assertEqual('eligible', report['preflight']['status'])
        self.assertEqual(expected, (captured['protocol'], captured['corpus']))
        self.assertEqual(protocol['repository_review']['review_reference'],
                         captured['protocol']['repository_review']['review_reference'])
        self.assertEqual('  corpus review  ', records['corpus_freeze_review']['value']['review_reference'])

    def test_discordant_valid_bindings_report_exact_field_paths(self):
        other_hash = 'sha256:' + 'b' * 64
        identity_other = None
        def changed(name, *path_and_value):
            return (name, path_and_value)
        specs = [
            ('corpus_freeze_review', 'corpus_id', 'other-corpus', 'corpus_mismatch'),
            ('corpus_freeze_review', 'version', 'other-version', 'corpus_mismatch'),
            ('corpus_freeze_review', 'corpus_hash', other_hash, 'corpus_mismatch'),
            ('corpus_freeze_review', 'status', 'draft', 'corpus_mismatch'),
            ('storage_scope', 'run_id', 'other-run', 'scope_unsupported'),
            ('storage_scope', 'bridge_container_id', 'other-container', 'scope_unsupported'),
            ('storage_scope', 'isolation', 'shared-v1', 'scope_unsupported'),
            ('storage_scope', 'observed_candidates', ['existing'], 'scope_unsupported'),
            ('storage_scope', 'planned_candidates', [{'query_id': 'q0', 'candidate_id': 'c0'}], 'scope_unsupported'),
            ('effective_config', 'run_id', 'other-run', 'configuration_mismatch'),
            ('effective_config', 'retrieval_path', '/other', 'configuration_mismatch'),
            ('effective_config', 'candidate_scope', 'other-scope', 'configuration_mismatch'),
            ('effective_config', 'search_config', {'limit': 19, 'threshold': 0}, 'configuration_mismatch'),
            ('effective_config', 'search_config', {'limit': 20, 'threshold': 0.5}, 'configuration_mismatch'),
        ]
        for name, field, value, reason in specs:
            with self.subTest(name=name, field=field, value=value):
                protocol, corpus, operator, records = self.fixture(); records[name]['value'][field] = value
                self.assertEqual([{'path': f'observations.{name}.value.{field}', 'reason': reason}],
                                 self.blocked(protocol, corpus, operator, records))
        for field in ('protocol_sha256', 'commit', 'review_reference', 'corroborated_at', 'corroborated_by'):
            protocol, corpus, operator, records = self.fixture(); records['operator_corroboration']['value'][field] = 'different'
            self.assertEqual([{'path': 'observations.operator_corroboration.value', 'reason': 'approval_mismatch'}],
                             self.blocked(protocol, corpus, operator, records))
        for name, reason in (('protocol_hash', 'approval_mismatch'), ('review_reference', 'approval_mismatch'),
                             ('corpus_hash', 'corpus_mismatch'),
                             ('run_id', 'preflight_evidence_missing_or_conflicting')):
            protocol, corpus, operator, records = self.fixture(); records[name]['value'] = other_hash
            self.assertEqual([{'path': f'observations.{name}.value', 'reason': reason}],
                             self.blocked(protocol, corpus, operator, records))
        protocol, corpus, operator, records = self.fixture()
        records['encoder_expected_identity']['value']['declared']['space_id'] = 'f' * 64
        self.assertEqual([{'path': 'observations.encoder_expected_identity.value',
                           'reason': 'preflight_evidence_missing_or_conflicting'}],
                         self.blocked(protocol, corpus, operator, records))
        protocol, corpus, operator, records = self.fixture()
        records['bridge_container_id']['value'] = 'other-container'
        self.assertEqual([{'path': 'observations.storage_scope.value.bridge_container_id', 'reason': 'scope_unsupported'}],
                         self.blocked(protocol, corpus, operator, records))

    def test_public_report_is_independent_and_hides_sensitive_keys(self):
        from copy import deepcopy
        protocol, corpus, operator, records = self.fixture()
        records['storage_scope']['value']['private-secret'] = 'private-secret'
        records['corpus_freeze_review']['value'][1] = 'private-secret'
        findings = self.blocked(protocol, corpus, operator, records)
        self.assertEqual([{'path': 'observations.corpus_freeze_review.value', 'reason': 'preflight_evidence_missing_or_conflicting'},
                          {'path': 'observations.storage_scope.value', 'reason': 'preflight_evidence_missing_or_conflicting'}], findings)
        protocol, corpus, operator, records = self.fixture()
        result, _ = self.invoke(protocol, corpus, operator, records)
        snapshot = deepcopy(result)
        result['preflight']['findings'].append('mutated'); result['protocol']['findings']['missing'].append('mutated')
        result['corpus']['id'] = 'mutated'
        again, _ = self.invoke(protocol, corpus, operator, records)
        self.assertEqual(snapshot, again)
        self.assertFalse({'working_protocol', 'working_corpus', 'operator_record', 'pairs', 'observations'} & set(again))

    def test_provider_inputs_and_returned_observations_do_not_alias_callers_or_report(self):
        from copy import deepcopy
        from unittest.mock import Mock
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records = self.fixture()
        caller_protocol, caller_corpus = protocol, corpus
        caller_state = deepcopy((protocol, corpus, operator, records))
        received = {}
        evidence = Mock()

        def acquire(*, protocol, corpus, run_id):
            received.update(protocol=protocol, corpus=corpus, run_id=run_id)
            self.assertEqual('synthetic-run', run_id)
            self.assertIsNot(protocol, caller_protocol)
            self.assertIsNot(corpus, caller_corpus)
            self.assertIsNot(protocol, corpus)
            protocol['protocol_id'] = 'provider-mutated'
            corpus['queries'][0]['text'] = 'provider-mutated'
            return records

        evidence.before_run.side_effect = acquire
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id='synthetic-run', bridge=Mock(), evidence=evidence)
        snapshot = deepcopy(report)
        self.assertEqual(caller_state, (protocol, corpus, operator, records))
        self.assertEqual('eligible', report['preflight']['status'])
        self.assertNotIn('provider-mutated', json.dumps(report))
        evidence.after_response.assert_not_called()
        evidence.after_run.assert_not_called()
        self.assertEqual([], report['preflight']['findings'])
        records['run_id']['value'] = 'mutated-after-return'
        received['protocol']['protocol_id'] = 'mutated-after-return'
        received['corpus']['queries'][0]['text'] = 'mutated-after-return'
        self.assertEqual(snapshot, report)

    def test_each_specialized_record_binding_has_exact_safe_finding(self):
        cases = (
            ('corpus_freeze_review', 'corpus_id', 'other-corpus', 'corpus_mismatch'),
            ('corpus_freeze_review', 'version', 'other-version', 'corpus_mismatch'),
            ('corpus_freeze_review', 'corpus_hash', 'sha256:' + 'f' * 64, 'corpus_mismatch'),
            ('corpus_freeze_review', 'status', 'draft', 'corpus_mismatch'),
            ('storage_scope', 'run_id', 'other-run', 'scope_unsupported'),
            ('storage_scope', 'bridge_container_id', 'other-container', 'scope_unsupported'),
            ('storage_scope', 'storage_id', '/private/path', 'scope_unsupported'),
            ('storage_scope', 'isolation', 'shared-v1', 'scope_unsupported'),
            ('storage_scope', 'observed_candidates', ['existing'], 'scope_unsupported'),
            ('storage_scope', 'planned_candidates', [], 'scope_unsupported'),
            ('effective_config', 'run_id', 'other-run', 'configuration_mismatch'),
            ('effective_config', 'retrieval_path', '/other', 'configuration_mismatch'),
            ('effective_config', 'candidate_scope', 'other-scope', 'configuration_mismatch'),
            ('effective_config', 'search_config', {'limit': 19, 'threshold': 0}, 'configuration_mismatch'),
            ('operator_corroboration', 'protocol_sha256', 'other-hash', 'approval_mismatch'),
            ('operator_corroboration', 'commit', 'other-commit', 'approval_mismatch'),
            ('operator_corroboration', 'review_reference', 'other-review', 'approval_mismatch'),
            ('operator_corroboration', 'corroborated_at', 'other-time', 'approval_mismatch'),
            ('operator_corroboration', 'corroborated_by', 'other-operator', 'approval_mismatch'),
        )
        for name, field, value, reason in cases:
            with self.subTest(record=name, field=field):
                protocol, corpus, operator, records = self.fixture()
                records[name]['value'][field] = value
                findings = self.blocked(protocol, corpus, operator, records)
                path = f'observations.{name}.value.{field}'
                if name == 'operator_corroboration':
                    path = 'observations.operator_corroboration.value'
                self.assertEqual([{'path': path, 'reason': reason}], findings)
        bindings = (
            ('protocol_hash', 'different-hash', 'approval_mismatch'),
            ('review_reference', 'different-review', 'approval_mismatch'),
            ('corpus_hash', 'sha256:' + 'e' * 64, 'corpus_mismatch'),
            ('run_id', 'other-run', 'preflight_evidence_missing_or_conflicting'),
            ('bridge_container_id', 'other-container', 'scope_unsupported'),
        )
        for name, value, reason in bindings:
            with self.subTest(binding=name):
                protocol, corpus, operator, records = self.fixture()
                records[name]['value'] = value
                findings = self.blocked(protocol, corpus, operator, records)
                field = 'value'
                path = f'observations.{name}.{field}'
                if name == 'bridge_container_id':
                    path = 'observations.storage_scope.value.bridge_container_id'
                self.assertEqual([{'path': path, 'reason': reason}], findings)

    def test_duplicate_findings_and_sensitive_paths_are_stable(self):
        protocol, corpus, operator, records = self.fixture()
        records['corpus_freeze_review']['value']['status'] = 'draft'
        records['storage_scope']['value']['isolation'] = 'shared'
        findings = self.blocked(protocol, corpus, operator, records)
        self.assertEqual(findings, sorted(findings, key=lambda item: (item['path'], item['reason'])))
        self.assertEqual(len(findings), len({(item['path'], item['reason']) for item in findings}))
        self.assertNotIn('draft', json.dumps(findings))
        protocol, corpus, operator, records = self.fixture()
        records['corpus_freeze_review']['value']['secret.key'] = 'private-value'
        result, calls = self.invoke(protocol, corpus, operator, records)
        self.assertEqual(1, calls)
        self.assertEqual([{'path': 'observations.corpus_freeze_review.value',
                           'reason': 'preflight_evidence_missing_or_conflicting'}],
                         result['preflight']['findings'])
        self.assertNotIn('secret.key', json.dumps(result))


class ProtocolExecutionTests(unittest.TestCase):
    def fixture(self):
        protocol, corpus, operator, records = ProtocolEvidencePreflightTests().fixture()
        expected_identity = protocol["run_inputs"]["encoder_identity"]["value"]
        bridge = FakeBridge(identity=expected_identity)
        from unittest.mock import Mock
        evidence = Mock()
        evidence.before_run.return_value = records
        from retrieval_h5_provenance import LocalObservations
        evidence.after_response.return_value = LocalObservations({})
        evidence.after_run.return_value = LocalObservations({})
        return protocol, corpus, operator, records, bridge, evidence

    @staticmethod
    def install_effect_spies(stack, patch):
        for name in ("run_evaluation", "annotations_from_csv", "compose_report", "protocol_preflight"):
            stack.enter_context(patch("retrieval_h5_runner." + name,
                                      side_effect=AssertionError("unexpected pilot, metric, or gate effect")))
        for name in ("retrieval_quality.annotations_from_csv", "retrieval_quality.compose_report",
                     "retrieval_quality.evaluate_query"):
            stack.enter_context(patch(name, side_effect=AssertionError("unexpected quality helper effect")))

    def test_positive_synthetic_execution_two_queries_with_shared_candidate_ids(self):
        from contextlib import ExitStack
        from unittest.mock import patch
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, bridge, evidence = self.fixture()

        q0_cids = {c["id"] for c in corpus["queries"][0]["candidates"]}
        q1_cids = {c["id"] for c in corpus["queries"][1]["candidates"]}
        self.assertEqual(q0_cids, q1_cids)

        with ExitStack() as stack:
            self.install_effect_spies(stack, patch)
            report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
                run_id="synthetic-run", bridge=bridge, evidence=evidence)

        self.assertTrue(report["measurement_started"])
        self.assertEqual({"status": "eligible", "findings": []}, report["preflight"])
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])
        self.assertEqual({"status": "not_evaluated", "results": {}}, report["gates"]["absolute_relevance"])

        self.assertEqual(20, len(bridge.writes))
        self.assertEqual(2, len(bridge.diagnostic_searches))

        search0 = bridge.diagnostic_searches[0]
        search1 = bridge.diagnostic_searches[1]
        self.assertEqual(20, search0["limit"])
        self.assertEqual(0.0, search0["threshold"])
        self.assertEqual(20, search1["limit"])
        self.assertEqual(0.0, search1["threshold"])

        aliases0 = set(search0["candidate_aliases"].keys())
        aliases1 = set(search1["candidate_aliases"].keys())
        self.assertEqual(10, len(aliases0))
        self.assertEqual(10, len(aliases1))
        self.assertTrue(aliases0.isdisjoint(aliases1))

        evidence.before_run.assert_called_once()
        self.assertEqual(2, evidence.after_response.call_count)
        self.assertEqual("synthetic-run", evidence.after_response.call_args_list[0].kwargs["run_id"])
        self.assertEqual("q0", evidence.after_response.call_args_list[0].kwargs["query_id"])
        self.assertEqual("synthetic-run", evidence.after_response.call_args_list[1].kwargs["run_id"])
        self.assertEqual("q1", evidence.after_response.call_args_list[1].kwargs["query_id"])
        evidence.after_run.assert_called_once_with(run_id="synthetic-run")

    def test_unambiguous_session_id_encoding_prevents_concatenation_collisions(self):
        from retrieval_h5_runner import _session_id
        s1 = _session_id("run-1", "2")
        s2 = _session_id("run", "1-2")
        self.assertNotEqual(s1, s2)
        self.assertTrue(s1.startswith("muninn-h5-v2:"))
        self.assertTrue(s2.startswith("muninn-h5-v2:"))

    def test_duplicate_uuid_across_run_fails_closed_after_first_effect(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, _bridge, evidence = self.fixture()
        bridge = FakeBridge(identity=protocol["run_inputs"]["encoder_identity"]["value"], duplicate_uuid=True)
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence)

        self.assertTrue(report["measurement_started"])
        self.assertEqual("eligible", report["preflight"]["status"])
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])
        findings = report["gates"]["ann_exact"].get("findings", [])
        self.assertTrue(findings)
        self.assertEqual("response_invalid", findings[0]["reason"])
        evidence.after_run.assert_not_called()
        self.assertNotIn("duplicate-uuid-constant", json.dumps(report))

    def test_malformed_add_message_response_fails_closed(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, _bridge, evidence = self.fixture()
        bridge = FakeBridge(identity=protocol["run_inputs"]["encoder_identity"]["value"], malformed_insert=True)
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence)

        self.assertTrue(report["measurement_started"])
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])
        findings = report["gates"]["ann_exact"].get("findings", [])
        self.assertTrue(findings)
        self.assertEqual("response_invalid", findings[0]["reason"])
        evidence.after_run.assert_not_called()

    def test_unknown_alias_in_search_response_fails_closed(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, _bridge, evidence = self.fixture()
        bridge = FakeBridge(identity=protocol["run_inputs"]["encoder_identity"]["value"], unknown_alias=True)
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence)

        self.assertTrue(report["measurement_started"])
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])
        findings = report["gates"]["ann_exact"].get("findings", [])
        self.assertTrue(findings)
        self.assertEqual("response_invalid", findings[0]["reason"])
        evidence.after_run.assert_not_called()

    def test_duplicate_alias_in_search_response_fails_closed(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, _bridge, evidence = self.fixture()
        bridge = FakeBridge(identity=protocol["run_inputs"]["encoder_identity"]["value"], duplicate_alias=True)
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence)

        self.assertTrue(report["measurement_started"])
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])
        findings = report["gates"]["ann_exact"].get("findings", [])
        self.assertTrue(findings)
        self.assertEqual("response_invalid", findings[0]["reason"])
        evidence.after_run.assert_not_called()

    def test_invalid_ranks_in_search_response_fails_closed(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, _bridge, evidence = self.fixture()
        bridge = FakeBridge(identity=protocol["run_inputs"]["encoder_identity"]["value"], invalid_rank=True)
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence)

        self.assertTrue(report["measurement_started"])
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])
        findings = report["gates"]["ann_exact"].get("findings", [])
        self.assertTrue(findings)
        self.assertEqual("response_invalid", findings[0]["reason"])
        evidence.after_run.assert_not_called()

    def test_malformed_search_response_fails_closed(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, _bridge, evidence = self.fixture()
        bridge = FakeBridge(identity=protocol["run_inputs"]["encoder_identity"]["value"], malformed_search=True)
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence)

        self.assertTrue(report["measurement_started"])
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])
        findings = report["gates"]["ann_exact"].get("findings", [])
        self.assertTrue(findings)
        self.assertEqual("response_invalid", findings[0]["reason"])
        evidence.after_run.assert_not_called()

    def test_transport_exception_after_first_effect_fails_closed_without_rollback(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, _bridge, evidence = self.fixture()
        bridge = FakeBridge(identity=protocol["run_inputs"]["encoder_identity"]["value"], transport_error=True)
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence)

        self.assertTrue(report["measurement_started"])
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])
        findings = report["gates"]["ann_exact"].get("findings", [])
        self.assertTrue(findings)
        self.assertEqual("response_invalid", findings[0]["reason"])
        evidence.after_run.assert_not_called()
        self.assertNotIn("simulated bridge transport error", json.dumps(report))

    def test_after_response_or_after_run_exception_fails_closed(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, bridge, evidence = self.fixture()
        evidence.after_response.side_effect = RuntimeError("private after_response explosion")
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence)

        self.assertTrue(report["measurement_started"])
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])
        findings = report["gates"]["ann_exact"].get("findings", [])
        self.assertTrue(findings)
        self.assertEqual("response_invalid", findings[0]["reason"])
        evidence.after_run.assert_not_called()
        self.assertNotIn("private after_response explosion", json.dumps(report))

        protocol, corpus, operator, records, bridge, evidence = self.fixture()
        evidence.after_run.side_effect = OSError("private after_run network failure")
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence)

        self.assertTrue(report["measurement_started"])
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])
        findings = report["gates"]["ann_exact"].get("findings", [])
        self.assertTrue(findings)
        self.assertEqual("response_invalid", findings[0]["reason"])
        self.assertNotIn("private after_run network failure", json.dumps(report))

    def test_no_retries_and_no_extra_identity_requests(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, bridge, evidence = self.fixture()
        report = run_protocol_evaluation(protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence)
        self.assertEqual(20, len(bridge.writes))
        self.assertEqual(2, len(bridge.diagnostic_searches))
        queries_searched = [s["query"] for s in bridge.diagnostic_searches]
        self.assertEqual(["Synthetic need 0", "Synthetic need 1"], queries_searched)


class ProtocolVerdictAndProvenanceTests(unittest.TestCase):
    def complete_fixture(self, *, bridge=None):
        from retrieval_h5_provenance import LocalObservations
        protocol, corpus, operator, preflight_records, default_bridge, evidence = (
            ProtocolExecutionTests().fixture()
        )
        expected_identity = protocol["run_inputs"]["encoder_identity"]["value"]
        active_bridge = bridge if bridge is not None else FakeBridge(identity=expected_identity)
        if bridge is not None and bridge.identity is None:
            bridge.identity = expected_identity

        final_records = LocalObservations({
            "protocol_hash": {"value": preflight_records["protocol_hash"]["value"], "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "review_reference": {"value": preflight_records["review_reference"]["value"], "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "operator_corroboration": {"value": operator, "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "corpus_hash": {"value": preflight_records["corpus_hash"]["value"], "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "runner_hash": {"value": "sha256:" + "1" * 64, "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "quality_hash": {"value": "sha256:" + "2" * 64, "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "bridge_image_id": {"value": "image-1", "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "bridge_container_id": {"value": "container-1", "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "bridge_binary_hash": {"value": "sha256:" + "3" * 64, "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "model_bundle_hash": {"value": "sha256:" + "4" * 64, "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "readonly_mounts": {"value": ["/model"], "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "effective_config": {"value": preflight_records["effective_config"]["value"], "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "compose_inspect": {"value": {"State": "running"}, "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "encoder_loaded_identity": {"value": expected_identity, "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "run_id": {"value": "synthetic-run", "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "started_at": {"value": "2026-01-01T00:00:00Z", "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
            "finished_at": {"value": "2026-01-01T00:01:00Z", "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:01:00Z"},
            "gate_results": {"value": {"ann_exact": "pending"}, "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:01:00Z"},
            "encoder_expected_identity": {"value": expected_identity, "source": "final", "method": "inspect", "observed_at": "2026-01-01T00:00:00Z"},
        })

        def after_response_cb(*, run_id, query_id, diagnostic):
            return LocalObservations({
                "encoder_response_identity": {"value": diagnostic["encoder_identity"], "source": "response", "method": "search", "observed_at": "2026-01-01T00:00:30Z"},
                "encoder_expected_identity": {"value": expected_identity, "source": "response", "method": "protocol", "observed_at": "2026-01-01T00:00:30Z"},
                "encoder_response_binding": {"value": {
                    "response_run_id": run_id,
                    "response_query_id": query_id,
                    "expected_query_id": query_id,
                }, "source": "response", "method": "binding", "observed_at": "2026-01-01T00:00:30Z"},
            })

        evidence.after_response.side_effect = after_response_cb
        evidence.after_run.return_value = final_records
        return protocol, corpus, operator, preflight_records, active_bridge, evidence, final_records

    def test_positive_synthetic_pass_both_metrics_perfect_and_provenance_verified(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture()
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )

        self.assertTrue(report["measurement_started"])
        self.assertEqual("eligible", report["preflight"]["status"])
        ann_gate = report["gates"]["ann_exact"]
        self.assertEqual("PASS", ann_gate["status"])
        results = ann_gate["results"]
        self.assertEqual(10, results["k"])
        self.assertEqual(2, len(results["per_query"]))
        for q_row in results["per_query"]:
            self.assertEqual(1.0, q_row["metrics"]["exact_top_k_recall"])
            self.assertEqual(1.0, q_row["metrics"]["ordered_top_k_match"])
        self.assertEqual({"value": 1.0, "undefined_count": 0}, results["macro_descriptive"]["exact_top_k_recall"])
        self.assertEqual({"value": 1.0, "undefined_count": 0}, results["macro_descriptive"]["ordered_top_k_match"])

        self.assertEqual({"status": "not_evaluated", "results": {}}, report["gates"]["absolute_relevance"])
        self.assertNotIn("acceptance", report)
        self.assertNotIn("global_status", report)

    def test_different_order_full_recall_produces_fail(self):
        from retrieval_h5_runner import run_protocol_evaluation
        bridge = FakeBridge(permute_ann=True)
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture(bridge=bridge)
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )

        self.assertTrue(report["measurement_started"])
        ann_gate = report["gates"]["ann_exact"]
        self.assertEqual("FAIL", ann_gate["status"])
        results = ann_gate["results"]
        self.assertEqual(10, results["k"])
        for q_row in results["per_query"]:
            self.assertEqual(1.0, q_row["metrics"]["exact_top_k_recall"])
            self.assertEqual(0, q_row["metrics"]["ordered_top_k_match"])
        self.assertEqual({"value": 1.0, "undefined_count": 0}, results["macro_descriptive"]["exact_top_k_recall"])
        self.assertEqual({"value": 0.0, "undefined_count": 0}, results["macro_descriptive"]["ordered_top_k_match"])

    def test_empty_exact_ranking_produces_null_metrics_and_blocked_gate(self):
        from retrieval_h5_runner import run_protocol_evaluation
        bridge = FakeBridge(empty_exact=True)
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture(bridge=bridge)
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )

        self.assertTrue(report["measurement_started"])
        ann_gate = report["gates"]["ann_exact"]
        self.assertEqual("blocked", ann_gate["status"])
        results = ann_gate["results"]
        self.assertEqual(10, results["k"])
        for q_row in results["per_query"]:
            self.assertIsNone(q_row["metrics"]["exact_top_k_recall"])
            self.assertIsNone(q_row["metrics"]["ordered_top_k_match"])
        self.assertIsNone(results["macro_descriptive"]["exact_top_k_recall"]["value"])
        self.assertEqual(2, results["macro_descriptive"]["exact_top_k_recall"]["undefined_count"])
        self.assertIsNone(results["macro_descriptive"]["ordered_top_k_match"]["value"])
        self.assertEqual(2, results["macro_descriptive"]["ordered_top_k_match"]["undefined_count"])
        reasons = [f["reason"] for f in ann_gate.get("findings", [])]
        self.assertIn("required_metric_undefined", reasons)

    def test_underfill_and_scores_do_not_override_sequential_ranks(self):
        from retrieval_h5_runner import run_protocol_evaluation
        # 1. Underfill con candidatos ordenados exactamente igual -> PASS
        bridge_pass = FakeBridge(underfill=5, tie_scores=True)
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture(bridge=bridge_pass)
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        self.assertEqual("PASS", report["gates"]["ann_exact"]["status"])
        for q_row in report["gates"]["ann_exact"]["results"]["per_query"]:
            self.assertEqual(1.0, q_row["metrics"]["exact_top_k_recall"])
            self.assertEqual(1.0, q_row["metrics"]["ordered_top_k_match"])

        # 2. Empate de scores con orden permutado en ANN -> FAIL (scores no desempatan)
        bridge_permuted = FakeBridge(underfill=5, tie_scores=True, permute_ann=True)
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture(bridge=bridge_permuted)
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        self.assertEqual("FAIL", report["gates"]["ann_exact"]["status"])
        for q_row in report["gates"]["ann_exact"]["results"]["per_query"]:
            self.assertEqual(1.0, q_row["metrics"]["exact_top_k_recall"])
            self.assertEqual(0, q_row["metrics"]["ordered_top_k_match"])

    def test_macro_descriptive_averages_without_rounding_and_requires_all_defined(self):
        from retrieval_h5_runner import run_protocol_evaluation
        # Un bridge que devuelva orden permutado solo en la primera query
        class MixedBridge(FakeBridge):
            def post(self, route, payload):
                res = super().post(route, payload)
                if route == "/h5/diagnostic/search" and payload["query"] == "Synthetic need 0":
                    res["ann"][0], res["ann"][1] = res["ann"][1], res["ann"][0]
                    res["ann"][0]["rank"] = 1
                    res["ann"][1]["rank"] = 2
                return res

        bridge = MixedBridge()
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture(bridge=bridge)
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        ann_gate = report["gates"]["ann_exact"]
        self.assertEqual("FAIL", ann_gate["status"])
        macro = ann_gate["results"]["macro_descriptive"]
        self.assertEqual(1.0, macro["exact_top_k_recall"]["value"])
        self.assertEqual(0, macro["exact_top_k_recall"]["undefined_count"])
        # Query 0 ordered match es 0, Query 1 ordered match es 1 -> media 0.5 exacta
        self.assertEqual(0.5, macro["ordered_top_k_match"]["value"])
        self.assertEqual(0, macro["ordered_top_k_match"]["undefined_count"])

    def test_adverse_final_provenance_blocks_gate_even_with_perfect_metrics(self):
        from retrieval_h5_runner import run_protocol_evaluation

        # a) Falta finished_at en final_records
        protocol, corpus, operator, records, bridge, evidence, final_records = self.complete_fixture()
        del final_records["finished_at"]
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])

        # b) Conflicto en compose_inspect
        protocol, corpus, operator, records, bridge, evidence, final_records = self.complete_fixture()
        final_records["compose_inspect"]["conflict"] = True
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])

        # c) Binding con otra consulta
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture()
        def bad_query_binding_cb(*, run_id, query_id, diagnostic):
            return LocalObservations({
                "encoder_response_identity": {"value": diagnostic["encoder_identity"], "source": "response", "method": "search", "observed_at": "2026-01-01T00:00:30Z"},
                "encoder_expected_identity": {"value": protocol["run_inputs"]["encoder_identity"]["value"], "source": "response", "method": "protocol", "observed_at": "2026-01-01T00:00:30Z"},
                "encoder_response_binding": {"value": {
                    "response_run_id": run_id,
                    "response_query_id": "other-query",
                    "expected_query_id": query_id,
                }, "source": "response", "method": "binding", "observed_at": "2026-01-01T00:00:30Z"},
            })
        evidence.after_response.side_effect = bad_query_binding_cb
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])

        # d) Binding con otro run
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture()
        def bad_run_binding_cb(*, run_id, query_id, diagnostic):
            return LocalObservations({
                "encoder_response_identity": {"value": diagnostic["encoder_identity"], "source": "response", "method": "search", "observed_at": "2026-01-01T00:00:30Z"},
                "encoder_expected_identity": {"value": protocol["run_inputs"]["encoder_identity"]["value"], "source": "response", "method": "protocol", "observed_at": "2026-01-01T00:00:30Z"},
                "encoder_response_binding": {"value": {
                    "response_run_id": "other-run",
                    "response_query_id": query_id,
                    "expected_query_id": query_id,
                }, "source": "response", "method": "binding", "observed_at": "2026-01-01T00:00:30Z"},
            })
        evidence.after_response.side_effect = bad_run_binding_cb
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])

        # e) Identidad discordante en respuesta
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture()
        def bad_identity_cb(*, run_id, query_id, diagnostic):
            bad_ident = deepcopy(diagnostic["encoder_identity"])
            bad_ident["declared"]["space_id"] = "f" * 64
            return LocalObservations({
                "encoder_response_identity": {"value": bad_ident, "source": "response", "method": "search", "observed_at": "2026-01-01T00:00:30Z"},
                "encoder_expected_identity": {"value": protocol["run_inputs"]["encoder_identity"]["value"], "source": "response", "method": "protocol", "observed_at": "2026-01-01T00:00:30Z"},
                "encoder_response_binding": {"value": {
                    "response_run_id": run_id,
                    "response_query_id": query_id,
                    "expected_query_id": query_id,
                }, "source": "response", "method": "binding", "observed_at": "2026-01-01T00:00:30Z"},
            })
        evidence.after_response.side_effect = bad_identity_cb
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])

    def test_incomplete_per_query_evidence_coverage_blocks_with_perfect_metrics(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture()
        query_ids = [query["id"] for query in corpus["queries"]]
        original_after_response = evidence.after_response.side_effect
        evidence.after_response.side_effect = lambda *, run_id, query_id, diagnostic: (
            None if query_id == query_ids[0]
            else original_after_response(run_id=run_id, query_id=query_id, diagnostic=diagnostic)
        )

        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        gate = report["gates"]["ann_exact"]
        self.assertEqual("blocked", gate["status"])
        self.assertTrue(any(f == {"path": "coverage", "reason": "coverage_incomplete"}
                            for f in gate.get("findings", [])))
        self.assertEqual(2, len(bridge.diagnostic_searches))
        for query_result in gate["results"]["per_query"]:
            self.assertEqual(1.0, query_result["metrics"]["exact_top_k_recall"])
            self.assertEqual(1.0, query_result["metrics"]["ordered_top_k_match"])



    def test_protocol_or_corpus_hash_mutation_before_verdict_blocks_gate(self):
        from retrieval_h5_runner import run_protocol_evaluation
        # Mutamos working_corpus durante after_response
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture()
        orig_after_resp = evidence.after_response.side_effect
        def mutator_cb(*, run_id, query_id, diagnostic):
            corpus["queries"][0]["text"] = "mutated text changing corpus hash"
            return orig_after_resp(run_id=run_id, query_id=query_id, diagnostic=diagnostic)
        evidence.after_response.side_effect = mutator_cb
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        self.assertEqual("blocked", report["gates"]["ann_exact"]["status"])

    def test_absolute_relevance_remains_not_evaluated_and_no_global_acceptance(self):
        from retrieval_h5_runner import run_protocol_evaluation
        protocol, corpus, operator, records, bridge, evidence, _ = self.complete_fixture()
        report = run_protocol_evaluation(
            protocol=protocol, corpus=corpus, operator_record=operator,
            run_id="synthetic-run", bridge=bridge, evidence=evidence,
        )
        self.assertEqual({"status": "not_evaluated", "results": {}}, report["gates"]["absolute_relevance"])
        self.assertNotIn("acceptance", report)
        self.assertNotIn("global_status", report)
