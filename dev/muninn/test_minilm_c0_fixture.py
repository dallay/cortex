"""Validación estática C0; solo biblioteca estándar, sin inferencia ni caché."""
import hashlib
import json
from pathlib import Path
import re
import unicodedata
import unittest

ROOT = Path(__file__).resolve().parents[2]
FIX = Path(__file__).resolve().parent / "fixtures"


def load(name):
    return json.loads((FIX / name).read_text(encoding="utf-8"))


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


class C0FixtureTests(unittest.TestCase):
    def setUp(self):
        self.corpus = load("minilm-c0-v1.json")
        self.labels = load("minilm-c0-v1-labels.json")
        self.manifest = load("minilm-c0-v1-runtime.json")
        self.refs = {f"{b['id']}/{i}": text
                     for b in self.corpus["batches"]
                     for i, text in enumerate(b["texts"])}

    def test_export_schema_and_limits(self):
        self.assertEqual(set(self.corpus), {"corpus_id", "batches"})
        self.assertEqual(self.corpus["corpus_id"], "c0-v1")
        self.assertLessEqual((FIX / "minilm-c0-v1.json").stat().st_size, 256 * 1024)
        batches = self.corpus["batches"]
        self.assertTrue(1 <= len(batches) <= 32)
        self.assertEqual(len({b["id"] for b in batches}), len(batches))
        for batch in batches:
            self.assertEqual(set(batch), {"id", "texts"})
            self.assertRegex(batch["id"], r"\A[A-Za-z0-9_-]{1,64}\Z")
            self.assertTrue(1 <= len(batch["texts"]) <= 32)
            for text in batch["texts"]:
                self.assertIsInstance(text, str)
                self.assertLessEqual(len(text.encode("utf-8")), 4096)

    def test_labels_and_coverage(self):
        self.assertEqual(self.labels["corpus_id"], "c0-v1")
        self.assertEqual(self.labels["status"], "human_hypotheses_not_results")
        required = {"spanish", "english", "low_lexical_overlap", "lexical_negatives",
                    "unicode", "empty", "rust", "cypher", "technical_decisions",
                    "length_boundaries", "prefix_tail", "batch_composition",
                    "batch_order", "batch_repetition"}
        self.assertEqual(set(self.labels["coverage"]), required)
        for refs in self.labels["coverage"].values():
            self.assertTrue(refs)
            for ref in refs:
                self.assertIn(ref, self.refs)
        ids = set()
        for case in self.labels["triplets"]:
            self.assertNotIn(case["id"], ids)
            ids.add(case["id"])
            for key in ("query", "positive", "negative"):
                self.assertIn(case[key], self.refs)
            self.assertEqual(len({case[k] for k in ("query", "positive", "negative")}), 3)
            self.assertTrue(case["rationale_es"])
        self.assertGreaterEqual(len(ids), 8)
        self.assertEqual(self.refs["unicode/0"], unicodedata.normalize("NFC", self.refs["unicode/1"]))
        self.assertNotEqual(self.refs["unicode/0"], self.refs["unicode/1"])
        self.assertEqual(self.refs["empty/0"], "")

    def test_length_hypotheses_and_tails(self):
        expected = [(125, 127), (126, 128), (127, 129), (253, 255), (254, 256), (255, 257)]
        self.assertEqual([(x["hello_repetitions"], x["expected_encoded_length_before_truncation"])
                          for x in self.labels["length_hypotheses"]], expected)
        for case in self.labels["length_hypotheses"]:
            self.assertEqual(self.refs[case["ref"]], " ".join(["hello"] * case["hello_repetitions"]))
            self.assertFalse(case["tokenizer_verified"])
        tails = [self.refs[f"prefix_tail/{i}"] for i in range(2)]
        self.assertEqual(tails[0].split()[:255], tails[1].split()[:255])
        self.assertNotEqual(tails[0].split()[255:], tails[1].split()[255:])
        self.assertTrue(self.refs["prefix_tail/2"].startswith("Autorizar"))
        self.assertTrue(self.refs["prefix_tail/3"].startswith("Denegar"))

    def test_batch_relationships(self):
        for relation in self.labels["equal_text_groups"]:
            self.assertGreaterEqual(len(relation), 2)
            texts = [self.refs[ref] for ref in relation]
            self.assertEqual(len(set(texts)), 1)
        for i, source in enumerate(["semantic_es/0", "lengths/5", "empty/0", "unicode/1"]):
            self.assertEqual(self.refs[f"mixed/{i}"], self.refs[source])
            self.assertEqual(self.refs[f"mixed_reverse/{3-i}"], self.refs[source])
        self.assertEqual(self.refs["repeat/0"], self.refs["repeat/2"])

    def test_frozen_hashes_and_sources(self):
        for record in self.manifest["frozen_files"]:
            path = ROOT / record["path"]
            self.assertEqual(digest(path), record["sha256"], str(path))
        self.assertEqual(self.manifest["status"], "preparation_config_not_runtime_evidence")
        lock = json.loads((ROOT / "docs/muninn/evidence/artifact-lock.json").read_text())
        model = next(m for m in lock["models"] if m["id"] == "sentence-transformers/all-MiniLM-L6-v2")
        self.assertEqual(self.manifest["model"]["id"], model["id"])
        self.assertEqual(self.manifest["model"]["revision"], model["revision"])
        receipt = {"schema_version": 1, "model_id": model["id"], "revision": model["revision"],
                   "files": [{k: f[k] for k in ("path", "size", "sha256")}
                             for f in sorted(model["files"], key=lambda x: x["path"])]}
        raw = json.dumps(receipt, separators=(",", ":"), ensure_ascii=False).encode()
        self.assertEqual(self.manifest["model"]["bundle_id"], hashlib.sha256(raw).hexdigest())
        doc = (FIX / "minilm-c0-v1.md").read_text()
        for name in ("minilm-c0-v1.json", "minilm-c0-v1-labels.json", "minilm-c0-v1-runtime.json"):
            self.assertIn(digest(FIX / name), doc)
        # Las versiones declaradas deben proceder de los archivos fuente congelados.
        for record in self.manifest["versions"]:
            self.assertIn(record["source_fragment"], (ROOT / record["source"]).read_text())
            self.assertIn(record["version"], record["source_fragment"])


if __name__ == "__main__":
    unittest.main()
