import csv
import hashlib
import io
import json
import unittest
from pathlib import Path

from retrieval_h5_runner import HttpBridge, run_evaluation, validate_endpoint, runner_connection
from retrieval_quality import corpus_hash


class FakeBridge:
    def __init__(self):
        self.writes = []
        self.searches = []
        self.diagnostic_searches = []

    def post(self, route, payload):
        if route == "/add_message":
            self.writes.append(payload)
            return {"id": f"uuid-{len(self.writes)}"}
        if route == "/search_messages":
            self.searches.append(payload)
            # Model Neo4j ANN: select the global top-k before the bridge filters by session.
            global_hits = [f"uuid-{i+1}" for i, item in enumerate(self.writes)][:payload["limit"]]
            ids = [raw_id for raw_id in global_hits
                   if self.writes[int(raw_id.removeprefix("uuid-"))-1]["session_id"] == payload["session_id"]]
            return [{"id": raw_id, "score": 0.9-i*.1} for i, raw_id in enumerate(ids)]
        if route == "/h5/diagnostic/search":
            self.diagnostic_searches.append(payload)
            payload = dict(payload)
            ann = self.post("/search_messages", payload)
            return {"ann": [{"alias": payload["candidate_aliases"][hit["id"]], "score": hit["score"], "rank": i + 1}
                              for i, hit in enumerate(ann)],
                    "exact": [{"alias": payload["candidate_aliases"][hit["id"]], "score": hit["score"], "rank": i + 1}
                               for i, hit in enumerate(ann)]}
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


if __name__ == "__main__": unittest.main()
