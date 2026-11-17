"""Validación de referencias sin descargar ni ejecutar modelos."""

import importlib.util
import copy
import json
from pathlib import Path
import sys
import tempfile
import unittest


def load_lock():
    path = Path(__file__).resolve().parents[1] / "artifact_lock.py"
    if not path.is_file():
        raise AssertionError("Falta el validador de artefactos")
    spec = importlib.util.spec_from_file_location("artifact_lock", path)
    module = importlib.util.module_from_spec(spec)
    sys.path.insert(0, str(path.parent))
    try:
        spec.loader.exec_module(module)
    finally:
        sys.path.pop(0)
    return module


class ArtifactTests(unittest.TestCase):
    def test_manifest_does_not_promote_metadata_or_accept_escaping_cache_paths(self):
        lock = load_lock()
        record = lock.file_record("config.json", 2, content=b"{}", cache_file="config.json")
        manifest = json.loads((Path(__file__).resolve().parents[3]/"dev/muninn/fixtures/artifact-lock.json").read_text())
        lock.validate_manifest(manifest)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/"config.json").write_bytes(b"{}")
            lock.verify_cached_file(record, root)
            (root/"config.json").write_bytes(b"modified")
            with self.assertRaisesRegex(ValueError,"hash|Tamaño"):
                lock.verify_cached_file(record, root)
        with tempfile.TemporaryDirectory() as directory:
            parent = Path(directory)
            root = parent/"cache"
            root.mkdir()
            (parent/"outside.json").write_bytes(b"{}")
            (root/"config.json").symlink_to(parent/"outside.json")
            with self.assertRaisesRegex(ValueError,"autorizado"):
                lock.verify_cached_file(record, root)
        manifest["plugins"][0]["activation_verified"] = True
        with self.assertRaisesRegex(ValueError,"activación"):
            lock.validate_manifest(manifest)
        manifest["plugins"][0]["activation_verified"] = False
        manifest["models"][0]["runtime_validation"] = "passed"
        with self.assertRaisesRegex(ValueError,"runtime"):
            lock.validate_manifest(manifest)
        manifest["models"][0]["runtime_validation"] = "not_run"
        manifest["models"][0]["revision"] = "main"
        with self.assertRaisesRegex(ValueError,"revisión"):
            lock.validate_manifest(manifest)

    def test_empty_profile_and_invalid_plugin_receipts_are_rejected(self):
        lock = load_lock()
        source = json.loads((Path(__file__).resolve().parents[3]/"dev/muninn/fixtures/artifact-lock.json").read_text())
        for field in ("models", "plugins"):
            changed=copy.deepcopy(source)
            changed[field]=[]
            with self.assertRaises(ValueError):
                lock.validate_manifest(changed)
        for key,value in (("path","../../outside.jar"),("size",-1),("size",True),("activation_verified","true")):
            changed=copy.deepcopy(source)
            changed["plugins"][0][key]=value
            with self.assertRaises(ValueError):
                lock.validate_manifest(changed)

    def test_metadata_hash_is_not_claimed_as_verified_weight_bytes(self):
        lock = load_lock()
        record = lock.file_record("model.safetensors", 774366564, lfs_sha256="a" * 64)
        self.assertEqual(record["verification"], "metadata_lfs_no_bytes")
        self.assertNotIn("cache_file", record)
        actual = lock.file_record("config.json", 2, content=b"{}", cache_file="config.json")
        self.assertEqual(actual["verification"], "bytes_at_capture")
        with self.assertRaisesRegex(ValueError, "hash"):
            lock.file_record("config.json", 2, content=b"{}", cache_file="config.json", lfs_sha256="a" * 64)
        for path in ("../secret", "/absolute", "a/../secret", "a\\secret"):
            with self.assertRaises(ValueError):
                lock.file_record(path, 2, lfs_sha256="a" * 64)

    def test_spacy_minor_compatibility_rejects_the_old_hub_model(self):
        lock = load_lock()
        self.assertTrue(lock.compatible_version("3.8.16", ">=3.8.0,<3.9.0"))
        self.assertFalse(lock.compatible_version("3.8.16", ">=3.7.2,<3.8.0"))
        with self.assertRaises(ValueError):
            lock.compatible_version("3.8.16", "latest")


if __name__ == "__main__":
    unittest.main()
