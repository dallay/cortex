"""Tests for the Bazel BUILD file generator.

The generator translates the Cargo workspace inventory into Bazel targets. These
tests pin the mapping rules: target kinds, dependency labels, feature sets,
build scripts, embedded data files, and deterministic output.
"""

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("generate_build_files.py")
spec = importlib.util.spec_from_file_location("generate_build_files", SCRIPT)
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


def target(name, kinds, src_path, crate_types=None, required_features=None):
    return {
        "name": name,
        "kind": list(kinds),
        "src_path": src_path,
        "crate_types": list(crate_types or ["lib"]),
        "required-features": list(required_features or []),
        "edition": "2021",
    }


def package(
    name="demo",
    directory="crates/demo",
    targets=None,
    dependencies=None,
    features=None,
    version="0.0.1",
    package_id=None,
):
    return {
        "name": name,
        "version": version,
        "id": package_id or f"path+file:///repo/{directory}#{version}",
        "manifest_path": f"/repo/{directory}/Cargo.toml",
        "edition": "2021",
        "targets": targets if targets is not None else [
            target("demo", ["lib"], f"/repo/{directory}/src/lib.rs"),
        ],
        "dependencies": dependencies or [],
        "features": features or {},
    }


class WorkspaceFixture:
    """A temporary repository tree with Cargo metadata and sources."""

    def __init__(self, packages, files=None):
        self._temp = tempfile.TemporaryDirectory()
        self.root = Path(self._temp.name)
        self.packages = [self._relocate(package) for package in packages]
        for relative, content in (files or {}).items():
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)

    def _relocate(self, package):
        """Move the fixture paths from the placeholder root into the temp tree."""
        relocated = dict(package)
        relocated["manifest_path"] = str(self.root / str(package["manifest_path"]).removeprefix("/repo/"))
        relocated["targets"] = [
            {
                **cargo_target,
                "src_path": str(self.root / str(cargo_target["src_path"]).removeprefix("/repo/")),
            }
            for cargo_target in package.get("targets", [])
        ]
        relocated["dependencies"] = [
            {
                **dependency,
                "path": str(self.root / str(dependency["path"]).removeprefix("/repo/"))
                if dependency.get("path") else None,
            }
            for dependency in package.get("dependencies", [])
        ]
        return relocated

    def metadata(self):
        return {
            "packages": self.packages,
            "workspace_members": [pkg["id"] for pkg in self.packages],
            "workspace_root": str(self.root),
        }

    def close(self):
        self._temp.cleanup()


class GeneratorTests(unittest.TestCase):
    def setUp(self):
        self.fixtures = []

    def tearDown(self):
        for fixture in self.fixtures:
            fixture.close()

    def fixture(self, packages, files=None):
        fixture = WorkspaceFixture(packages, files)
        self.fixtures.append(fixture)
        return fixture

    # -- helpers ---------------------------------------------------------

    def plan(self, packages, files=None, **options):
        fixture = self.fixture(packages, files)
        index = generator.package_index(fixture.packages, fixture.root)
        plans = [
            generator.build_plan(pkg, fixture.root, index, **options)
            for pkg in fixture.packages
        ]
        return plans, fixture

    def targets_by_name(self, plan):
        return {target_plan.name: target_plan for target_plan in plan.targets}

    # -- naming ----------------------------------------------------------

    def test_crate_name_normalizes_dashes(self):
        self.assertEqual(generator.crate_name("rook-core"), "rook_core")
        self.assertEqual(generator.crate_name("muninn-mcp"), "muninn_mcp")

    # -- library ---------------------------------------------------------

    def test_lib_target_is_emitted_with_crate_root_and_dependencies(self):
        packages = [package(
            targets=[
                target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            ],
        )]
        plans, _ = self.plan(packages)
        plan = plans[0]
        lib = self.targets_by_name(plan)["demo"]
        self.assertEqual(lib.rule, "rust_library")
        self.assertEqual(lib.attributes["crate_name"], "demo")
        self.assertEqual(lib.attributes["crate_root"], "src/lib.rs")

    def test_workspace_path_dependencies_become_explicit_labels(self):
        library = package(
            name="demo-lib",
            directory="crates/demo-lib",
            targets=[target("demo_lib", ["lib"], "/repo/crates/demo-lib/src/lib.rs")],
        )
        consumer = package(
            name="demo",
            directory="crates/demo",
            dependencies=[{
                "name": "demo-lib",
                "req": "*",
                "kind": None,
                "optional": False,
                "uses_default_features": True,
                "features": [],
                "path": "/repo/crates/demo-lib",
            }],
        )
        plans, _ = self.plan([library, consumer])
        consumer_plan = next(plan for plan in plans if plan.directory == "crates/demo")
        lib = self.targets_by_name(consumer_plan)["demo"]
        self.assertIn("//crates/demo-lib:demo_lib", lib.attributes["deps"])
        self.assertNotIn('"demo-lib"', str(lib.attributes["deps"]))

    def test_external_dependencies_use_all_crate_deps(self):
        consumer = package(
            name="demo",
            directory="crates/demo",
            dependencies=[{
                "name": "serde",
                "req": "^1",
                "kind": None,
                "optional": False,
                "uses_default_features": True,
                "features": [],
            }],
        )
        plans, _ = self.plan([consumer])
        lib = self.targets_by_name(plans[0])["demo"]
        self.assertIn("all_crate_deps(normal = True)", lib.attributes["deps"])

    def test_optional_dependency_is_excluded_until_its_feature_is_enabled(self):
        consumer = package(
            name="demo",
            directory="crates/demo",
            features={"extra": ["dep:serde"]},
            dependencies=[{
                "name": "serde",
                "req": "^1",
                "kind": None,
                "optional": True,
                "uses_default_features": True,
                "features": [],
            }],
        )
        plans, _ = self.plan([consumer])
        lib = self.targets_by_name(plans[0])["demo"]
        self.assertNotIn("serde", str(lib.attributes["deps"]))

    # -- unit tests ------------------------------------------------------

    def test_unit_test_target_is_emitted_when_the_lib_has_unit_tests(self):
        packages = [package()]
        fixture = self.fixture(packages, {
            "crates/demo/src/lib.rs": "#[cfg(test)]\nmod tests {}\n",
        })
        index = generator.package_index(fixture.packages, fixture.root)
        plan = generator.build_plan(fixture.packages[0], fixture.root, index)
        unit_test = self.targets_by_name(plan)["demo_unit_test"]
        self.assertEqual(unit_test.rule, "rust_test")
        self.assertEqual(unit_test.attributes["crate"], ":demo")

    def test_unit_test_target_is_absent_without_unit_tests(self):
        packages = [package()]
        fixture = self.fixture(packages, {"crates/demo/src/lib.rs": "pub fn demo() {}\n"})
        index = generator.package_index(fixture.packages, fixture.root)
        plan = generator.build_plan(fixture.packages[0], fixture.root, index)
        self.assertNotIn("demo_unit_test", self.targets_by_name(plan))

    # -- integration tests ------------------------------------------------

    def test_integration_test_targets_are_emitted(self):
        packages = [package(targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target("smoke", ["test"], "/repo/crates/demo/tests/smoke.rs", crate_types=["bin"]),
        ])]
        plans, _ = self.plan(packages)
        integration = self.targets_by_name(plans[0])["smoke"]
        self.assertEqual(integration.rule, "rust_test")
        self.assertEqual(integration.attributes["srcs"], ["tests/smoke.rs"])
        self.assertIn(":demo", integration.attributes["deps"])

    def test_module_from_another_package_is_addressed_by_label(self):
        packages = [package(targets=[
            target("smoke", ["test"], "/repo/crates/demo/tests/smoke.rs", crate_types=["bin"]),
        ])]
        fixture = self.fixture(packages, {
            "crates/demo/tests/smoke.rs":
                '#[path = "../../other/tests/support/helper.rs"]\nmod support;\n',
            "crates/other/tests/support/helper.rs": "pub fn helper() {}\n",
            "crates/other/BUILD.bazel": "",
        })
        index = generator.package_index(fixture.packages, fixture.root)
        plan = generator.build_plan(fixture.packages[0], fixture.root, index)
        integration = self.targets_by_name(plan)["smoke"]
        self.assertEqual(
            integration.attributes["srcs"],
            ["//crates/other:tests/support/helper.rs", "tests/smoke.rs"],
            "a module that lives in another Bazel package must be referenced by label",
        )

    def test_module_from_the_root_package_is_addressed_by_label(self):
        packages = [package(targets=[
            target("smoke", ["test"], "/repo/crates/demo/tests/smoke.rs", crate_types=["bin"]),
        ])]
        fixture = self.fixture(packages, {
            "crates/demo/tests/smoke.rs": '#[path = "../../../shared/helper.rs"]\nmod support;\n',
            "shared/helper.rs": "pub fn helper() {}\n",
        })
        index = generator.package_index(fixture.packages, fixture.root)
        plan = generator.build_plan(fixture.packages[0], fixture.root, index)
        integration = self.targets_by_name(plan)["smoke"]
        self.assertEqual(
            integration.attributes["srcs"],
            ["//:shared/helper.rs", "tests/smoke.rs"],
        )

    def test_cargo_bin_exe_used_by_a_module_declares_the_binary_dependency(self):
        packages = [package(targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target("demo", ["bin"], "/repo/crates/demo/src/main.rs", crate_types=["bin"]),
            target("smoke", ["test"], "/repo/crates/demo/tests/smoke.rs", crate_types=["bin"]),
        ])]
        files = {
            "crates/demo/tests/smoke.rs": '#[path = "support/binary.rs"]\nmod support;\n',
            "crates/demo/tests/support/binary.rs":
                "pub fn bin() -> &'static str { env!(\"CARGO_BIN_EXE_demo\") }\n",
        }
        plans, _ = self.plan(packages, files=files)
        smoke = self.targets_by_name(plans[0])["smoke"]
        self.assertEqual(
            smoke.attributes["env"],
            {"CARGO_BIN_EXE_demo": "$(execpath :demo_bin)"},
            "a module that reads CARGO_BIN_EXE_* needs the same contract as the entry file",
        )
        self.assertIn(":demo_bin", smoke.attributes["deps"])

    # -- binaries ---------------------------------------------------------

    def test_example_target_can_use_dev_dependencies(self):
        packages = [package(targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target(
                "sample",
                ["example"],
                "/repo/crates/demo/examples/sample.rs",
                crate_types=["bin"],
            ),
        ])]
        plans, _ = self.plan(packages)
        example = self.targets_by_name(plans[0])["sample"]
        self.assertEqual(example.rule, "rust_binary")
        self.assertIn("normal_dev = True", example.attributes["deps"])
        self.assertIn("proc_macro_dev = True", example.attributes["proc_macro_deps"])

    def test_binary_target_uses_a_disambiguated_name_and_keeps_the_crate_name(self):
        packages = [package(targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target("demo", ["bin"], "/repo/crates/demo/src/main.rs", crate_types=["bin"]),
        ])]
        plans, _ = self.plan(packages)
        targets = self.targets_by_name(plans[0])
        self.assertIn("demo_bin", targets)
        self.assertEqual(targets["demo_bin"].attributes["crate_name"], "demo")

    def test_binary_target_without_name_collision_keeps_its_cargo_name(self):
        packages = [package(name="muninn-mcp", directory="crates/muninn/mcp", targets=[
            target("muninn_mcp", ["lib"], "/repo/crates/muninn/mcp/src/lib.rs"),
            target("muninn-mcp", ["bin"], "/repo/crates/muninn/mcp/src/main.rs", crate_types=["bin"]),
        ])]
        plans, _ = self.plan(packages)
        targets = self.targets_by_name(plans[0])
        self.assertIn("muninn_mcp_bin", targets)
        self.assertEqual(targets["muninn_mcp_bin"].attributes["crate_name"], "muninn_mcp")

    def test_binary_with_unit_tests_produces_a_test_target(self):
        packages = [package(targets=[
            target("demo", ["bin"], "/repo/crates/demo/src/main.rs", crate_types=["bin"]),
        ])]
        files = {
            "crates/demo/src/main.rs": "fn main() {}\n#[cfg(test)]\nmod tests {}\n",
        }
        plans, _ = self.plan(packages, files=files)
        targets = self.targets_by_name(plans[0])
        self.assertIn("demo_test", targets)
        self.assertEqual(targets["demo_test"].rule, "rust_test")
        self.assertEqual(targets["demo_test"].attributes["srcs"], ["src/main.rs"])
        self.assertEqual(targets["demo_test"].attributes["crate_root"], "src/main.rs")

    def test_binary_without_unit_tests_has_no_test_target(self):
        packages = [package(targets=[
            target("demo", ["bin"], "/repo/crates/demo/src/main.rs", crate_types=["bin"]),
        ])]
        files = {"crates/demo/src/main.rs": "fn main() {}\n"}
        plans, _ = self.plan(packages, files=files)
        targets = self.targets_by_name(plans[0])
        self.assertIn("demo", targets)
        self.assertNotIn("demo_test", targets)

    def test_c1_exporter_unit_test_is_not_sandboxed(self):
        packages = [package(
            name="muninn-c1-exporter",
            directory="crates/muninn/c1-exporter",
            targets=[target(
                "muninn_c1_exporter",
                ["lib"],
                "/repo/crates/muninn/c1-exporter/src/lib.rs",
            )],
        )]
        files = {
            "crates/muninn/c1-exporter/src/lib.rs": "#[cfg(test)] mod tests {}\n",
        }
        plans, _ = self.plan(packages, files=files)
        unit_test = self.targets_by_name(plans[0])["muninn_c1_exporter_unit_test"]
        self.assertEqual(unit_test.attributes["tags"], ["no-sandbox"])

    # -- features ---------------------------------------------------------

    def test_default_feature_list_becomes_crate_features(self):
        packages = [package(
            features={"default": ["extra"], "extra": []},
            dependencies=[{
                "name": "serde",
                "req": "^1",
                "kind": None,
                "optional": True,
                "uses_default_features": True,
                "features": [],
            }],
        )]
        plans, _ = self.plan(packages)
        lib = self.targets_by_name(plans[0])["demo"]
        self.assertEqual(lib.attributes["crate_features"], ["extra"])
        self.assertIn("all_crate_deps(normal = True)", lib.attributes["deps"])

    def test_package_without_default_feature_enables_no_features(self):
        packages = [package(features={"extra": []})]
        plans, _ = self.plan(packages)
        lib = self.targets_by_name(plans[0])["demo"]
        self.assertNotIn("crate_features", lib.attributes)

    def test_required_features_produce_a_feature_variant_library(self):
        packages = [package(features={"test-hooks": []}, targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target("hooked", ["test"], "/repo/crates/demo/tests/hooked.rs",
                   crate_types=["bin"], required_features=["test-hooks"]),
        ])]
        plans, _ = self.plan(packages)
        targets = self.targets_by_name(plans[0])
        self.assertIn("demo_test_hooks", targets)
        self.assertEqual(targets["demo_test_hooks"].attributes["crate_features"], ["test-hooks"])
        self.assertIn(":demo_test_hooks", targets["hooked"].attributes["deps"])

    # -- build scripts ----------------------------------------------------

    def test_custom_build_target_produces_a_cargo_build_script(self):
        packages = [package(targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target("build-script-build", ["custom-build"], "/repo/crates/demo/build.rs",
                   crate_types=["bin"]),
        ])]
        plans, _ = self.plan(packages)
        targets = self.targets_by_name(plans[0])
        self.assertIn("build_script", targets)
        self.assertEqual(targets["build_script"].rule, "cargo_build_script")
        self.assertIn(":build_script", targets["demo"].attributes["deps"])

    # -- embedded data ----------------------------------------------------

    def test_sibling_test_files_do_not_leak_foreign_references(self):
        packages = [package(targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target("alpha", ["test"], "/repo/crates/demo/tests/alpha.rs", crate_types=["bin"]),
            target("beta", ["test"], "/repo/crates/demo/tests/beta.rs", crate_types=["bin"]),
        ])]
        plans, _ = self.plan(packages, files={
            "crates/demo/src/lib.rs": "pub fn demo() {}\n",
            "crates/demo/tests/alpha.rs": "#[test]\nfn alpha() {}\n",
            "crates/demo/tests/beta.rs": 'const RAW: &str = include_str!("../../fixtures/data.json");\n',
            "fixtures/data.json": "{}\n",
        })
        targets = self.targets_by_name(plans[0])
        # The sibling file that references the foreign fixture must not drag the
        # other test target into the same exclusion.
        self.assertIn("alpha", targets)
        self.assertNotIn("compile_data", targets["alpha"].attributes)
        self.assertNotIn("beta", targets)
        self.assertIn("beta", plans[0].exclusions)

    # -- inventory coverage -----------------------------------------------

    def test_default_selection_keeps_every_package(self):
        packages = [
            package(),
            package(name="huginn", directory="apps/huginn"),
        ]
        selected = generator.select_packages(
            packages, Path("/repo"), generator.DEFAULT_EXCLUDED_PREFIXES, None
        )
        self.assertEqual([pkg["name"] for pkg in selected], ["demo", "huginn"])

    def test_reviewed_exclusion_is_reported_in_the_inventory(self):
        packages = [package(name="huginn-runtime", directory="crates/huginn/runtime", targets=[
            target(
                "wasm_outline",
                ["test"],
                "/repo/crates/huginn/runtime/tests/wasm_outline.rs",
                crate_types=["bin"],
            ),
        ])]
        plans, _ = self.plan(packages)
        entries = generator.inventory_entries(plans[0])
        self.assertEqual(len(entries), 1)
        self.assertNotIn("label", entries[0])
        self.assertIn("experimental-wasm", entries[0]["exclusion"])

    def test_included_files_become_compile_data(self):
        packages = [package()]
        fixture = self.fixture(packages, {
            "crates/demo/src/lib.rs": 'const RAW: &str = include_str!("data.json");\n',
            "crates/demo/src/data.json": "{}\n",
        })
        index = generator.package_index(fixture.packages, fixture.root)
        plan = generator.build_plan(fixture.packages[0], fixture.root, index)
        lib = self.targets_by_name(plan)["demo"]
        self.assertEqual(lib.attributes["compile_data"], '["src/data.json"]')

    def test_files_outside_the_package_are_reported_as_cross_package_references(self):
        packages = [package()]
        fixture = self.fixture(packages, {
            "BUILD.bazel": 'exports_files(["crates/shared/data.json"])\n',
            "crates/demo/src/lib.rs": 'const RAW: &str = include_str!("../../shared/data.json");\n',
            "crates/shared/data.json": "{}\n",
        })
        index = generator.package_index(fixture.packages, fixture.root)
        plan = generator.build_plan(fixture.packages[0], fixture.root, index)
        lib = self.targets_by_name(plan)["demo"]
        self.assertEqual(
            lib.attributes["compile_data"],
            '["//:crates/shared/data.json"]',
        )

    # -- rendering --------------------------------------------------------

    def test_render_is_deterministic(self):
        packages = [package(targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target("smoke", ["test"], "/repo/crates/demo/tests/smoke.rs", crate_types=["bin"]),
        ])]
        plans, _ = self.plan(packages)
        first = generator.render_package(plans[0])
        second = generator.render_package(plans[0])
        self.assertEqual(first, second)

    def test_rendered_file_declares_the_generated_marker_and_required_loads(self):
        packages = [package()]
        plans, _ = self.plan(packages)
        text = generator.render_package(plans[0])
        self.assertIn(generator.GENERATED_MARKER, text)
        self.assertIn('load("@crates//:defs.bzl"', text)
        self.assertIn('load("@rules_rust//rust:defs.bzl"', text)

    def test_rendered_targets_are_parseable_and_expose_names(self):
        packages = [package(targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target("smoke", ["test"], "/repo/crates/demo/tests/smoke.rs", crate_types=["bin"]),
        ])]
        plans, _ = self.plan(packages)
        text = generator.render_package(plans[0])
        parsed = generator.parse_targets(text)
        self.assertEqual(sorted(parsed), ["demo", "smoke"])
        self.assertEqual(parsed["demo"][0], "rust_library")
        self.assertEqual(parsed["smoke"][0], "rust_test")

    # -- inventory --------------------------------------------------------

    def test_inventory_entries_cover_every_modeled_target(self):
        packages = [package(targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target("smoke", ["test"], "/repo/crates/demo/tests/smoke.rs", crate_types=["bin"]),
        ])]
        plans, _ = self.plan(packages)
        entries = generator.inventory_entries(plans)
        self.assertEqual(len(entries), 2)
        for entry in entries:
            self.assertIn("label", entry)
            self.assertTrue(entry["label"].startswith("//crates/demo:"))

    def test_inventory_entries_are_sorted_and_json_serializable(self):
        packages = [package(targets=[
            target("demo", ["lib"], "/repo/crates/demo/src/lib.rs"),
            target("smoke", ["test"], "/repo/crates/demo/tests/smoke.rs", crate_types=["bin"]),
        ])]
        plans, _ = self.plan(packages)
        entries = generator.inventory_entries(plans)
        serialized = json.dumps(entries, sort_keys=True)
        self.assertEqual(serialized, json.dumps(entries, sort_keys=True))
        self.assertEqual(entries, sorted(entries, key=generator.inventory_sort_key))

    def test_a_package_plan_id_omits_the_absolute_checkout_location(self):
        plan = generator.PackagePlan(
            directory="crates/demo",
            package={"id": "path+file:///repo/crates/demo#0.0.1", "name": "demo"},
        )
        self.assertEqual(
            plan.package_id,
            "path+file://<workspace>/crates/demo#0.0.1",
            "the plan identity must not depend on where the repository is checked out",
        )

    def test_merge_inventory_records_portable_ids_for_unmodeled_packages(self):
        path = Path(self._temp.name) / "missing.json" if hasattr(self, "_temp") else Path("/nonexistent/inventory.json")
        merged = generator.merge_inventory(
            path,
            [],
            Path("/repo"),
            [package(name="other", directory="crates/other")],
        )
        identities = [entry["package_id"] for entry in merged]
        self.assertEqual(identities, ["path+file://<workspace>/crates/other#0.0.1"])


if __name__ == "__main__":
    unittest.main()
