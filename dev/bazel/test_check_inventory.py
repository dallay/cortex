import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("check_inventory.py")
spec = importlib.util.spec_from_file_location("check_inventory", SCRIPT)
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class InventoryCheckerTests(unittest.TestCase):
    def setUp(self):
        self.metadata = {
            "packages": [{
                "id": "example@1.0.0",
                "name": "example",
                "version": "1.0.0",
                "manifest_path": str(checker.ROOT / "crates/example/Cargo.toml"),
                "targets": [
                    {"name": "example", "kind": ["lib"]},
                    {"name": "smoke", "kind": ["test"]},
                ],
            }],
            "workspace_members": ["example@1.0.0"],
        }
        self.package_id = "example@1.0.0"
        self.good = {"entries": [
            {"manifest": "crates/example/Cargo.toml", "package_id": self.package_id, "target": "example", "kinds": "lib", "label": "//crates/example:example"},
            {"manifest": "crates/example/Cargo.toml", "package_id": self.package_id, "target": "smoke", "kinds": "test", "exclusion": "Not modeled in this inventory slice; reviewed."},
        ]}
        self.labels = {"//crates/example:example"}

    def test_explicit_reviewed_exclusion_covers_target(self):
        coverage = checker.validate(self.metadata, self.good, self.labels)
        self.assertEqual([row["status"] for row in coverage], ["mapped", "excluded"])

    def test_missing_mapping_is_rejected(self):
        with self.assertRaisesRegex(checker.InventoryError, "missing mapping.*smoke"):
            checker.validate(self.metadata, {"entries": self.good["entries"][:1]}, self.labels)

    def test_duplicate_mapping_is_rejected(self):
        mapping = {"entries": self.good["entries"] + [self.good["entries"][0]]}
        with self.assertRaisesRegex(checker.InventoryError, "duplicate mapping"):
            checker.validate(self.metadata, mapping, self.labels)

    def test_stale_label_is_rejected(self):
        mapping = {"entries": [dict(row) for row in self.good["entries"]]}
        mapping["entries"][0]["label"] = "//gone:old_target"
        with self.assertRaisesRegex(checker.InventoryError, "stale Bazel label"):
            checker.validate(self.metadata, mapping, self.labels)

    def test_stale_cargo_entry_is_rejected(self):
        mapping = {"entries": [dict(row) for row in self.good["entries"]]}
        mapping["entries"][0]["target"] = "removed_target"
        with self.assertRaisesRegex(checker.InventoryError, "stale Cargo inventory identity"):
            checker.validate(self.metadata, mapping, self.labels)

    def test_json_output_is_deterministic(self):
        coverage = checker.validate(self.metadata, self.good, self.labels)
        first = json.dumps({"coverage": coverage}, sort_keys=True, indent=2)
        second = json.dumps({"coverage": checker.validate(self.metadata, self.good, self.labels)},
                            sort_keys=True, indent=2)
        self.assertEqual(first, second)

    def test_duplicate_cargo_identity_is_not_hidden_by_count_parity(self):
        duplicate = json.loads(json.dumps(self.metadata))
        duplicate["packages"][0]["targets"].append({"name": "example", "kind": ["lib"]})
        with self.assertRaisesRegex(checker.InventoryError, "duplicate Cargo inventory identity"):
            checker.validate(duplicate, self.good, self.labels)

    def test_checked_in_mapping_is_valid_json_and_has_unique_cargo_identities(self):
        mapping_path = Path(__file__).with_name("cargo_inventory.json")
        mapping = json.loads(mapping_path.read_text())
        keys = [(row["manifest"], row["package_id"], row["target"], row["kinds"])
                for row in mapping["entries"]]
        self.assertEqual(len(keys), len(set(keys)))
        self.assertNotIn("huginn_core_unit_test", [row["target"] for row in mapping["entries"]])

    def test_checked_in_mapping_does_not_embed_a_checkout_location(self):
        mapping_path = Path(__file__).with_name("cargo_inventory.json")
        self.assertNotIn(
            "file:///",
            mapping_path.read_text(),
            "the committed inventory must not record the absolute location of one checkout",
        )

    def test_cargo_package_id_uses_metadata_id_when_it_is_not_a_path(self):
        row = checker.cargo_inventory(self.metadata)[0]
        self.assertEqual(row["package_id"], self.metadata["packages"][0]["id"])

    def test_path_package_id_omits_the_absolute_checkout_location(self):
        row = checker.cargo_inventory(self.path_metadata(Path("/tmp/checkout")))[0]
        self.assertEqual(
            row["package_id"],
            "path+file://<workspace>/crates/example#example@1.0.0",
            "a path package id must not depend on where the repository is checked out",
        )
        self.assertNotIn("/tmp/checkout", row["package_id"])

    def test_path_package_id_is_stable_across_checkouts(self):
        first = checker.cargo_inventory(self.path_metadata(Path("/tmp/one")))[0]["package_id"]
        second = checker.cargo_inventory(self.path_metadata(Path("/tmp/two")))[0]["package_id"]
        self.assertEqual(first, second)

    def path_metadata(self, checkout: Path) -> dict:
        package_id = f"path+file://{checkout}/crates/example#example@1.0.0"
        return {
            "packages": [{
                "id": package_id,
                "name": "example",
                "version": "1.0.0",
                "manifest_path": str(checker.ROOT / "crates/example/Cargo.toml"),
                "targets": [{"name": "example", "kind": ["lib"]}],
            }],
            "workspace_members": [package_id],
        }

    def test_malformed_mapping_is_reported_as_inventory_error(self):
        with self.assertRaisesRegex(checker.InventoryError, "mapping must be an object"):
            checker.validate(self.metadata, [], self.labels)

    def test_malformed_mapping_identity_is_reported_as_inventory_error(self):
        malformed = {"entries": [{**self.good["entries"][0], "manifest": []}]}
        with self.assertRaisesRegex(checker.InventoryError, "manifest must be a non-empty string"): 
            checker.validate(self.metadata, malformed, self.labels)

    def test_malformed_metadata_is_reported_as_inventory_error(self):
        with self.assertRaisesRegex(checker.InventoryError, "metadata must contain packages"):
            checker.cargo_inventory({"workspace_members": []})


if __name__ == "__main__":
    unittest.main()
