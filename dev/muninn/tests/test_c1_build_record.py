"""Source closure regressions; no model or compiler execution."""
import fnmatch
import json
from pathlib import Path
import re
import tempfile
import tomllib
import unittest

import c1_build_record
import c1_manifest_build

ROOT = Path(__file__).resolve().parents[3]


class BuildMaterialsTests(unittest.TestCase):
    def create_complete_root(self, root):
        for relative in c1_build_record.FILES:
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b'fixture')
        for relative in c1_build_record.TREES:
            path = root / relative
            path.mkdir(parents=True, exist_ok=True)
            (path / 'source.rs').write_bytes(b'fixture')

    def test_external_and_build_control_materials_are_bound(self):
        required_files = {'dev/muninn/fixtures/artifact-lock.json',
                          'dev/muninn/c1_build_record.py', 'dev/muninn/c1_provenance.py',
                          '.github/workflows/ci.yml'}
        self.assertTrue(required_files <= set(c1_build_record.FILES))
        self.assertNotIn('crates/shared/macos-acl', c1_build_record.TREES)
        lock = tomllib.loads((ROOT / 'Cargo.lock').read_text())
        acl_package = next(package for package in lock['package']
                           if package['name'] == 'calcifer-macos-acl')
        self.assertTrue(acl_package['source'].startswith('registry+'))
        self.assertRegex(acl_package['checksum'], r'^[0-9a-f]{64}$')
        plan = json.loads((ROOT / 'dev/muninn/c1_attempt_plan.json').read_text())['stages'][0]
        self.assertEqual(set(plan['files']), set(c1_build_record.FILES))
        self.assertEqual(set(plan['source_trees']), set(c1_build_record.TREES))
        self.assertTrue(set(c1_build_record.FILES) <= set(c1_manifest_build.SOURCE_FILES))
        self.assertTrue(set(c1_build_record.TREES) <= set(c1_manifest_build.SOURCE_TREES))

    def test_all_static_include_inputs_are_in_closure(self):
        admitted = set(c1_build_record.FILES)
        for tree in c1_build_record.TREES:
            admitted.update(path.relative_to(ROOT).as_posix() for path in (ROOT / tree).rglob('*') if path.is_file())
        for tree in c1_build_record.TREES:
            for source in (ROOT / tree).rglob('*.rs'):
                for relative in re.findall(r'include_(?:bytes|str)!\s*\(\s*"([^"]+)"', source.read_text()):
                    path = (source.parent / relative).resolve().relative_to(ROOT).as_posix()
                    self.assertIn(path, admitted, f'{source}: {relative}')

    def test_pull_request_paths_cover_every_build_input_family(self):
        filters = json.loads((ROOT / '.github/ci-path-filters.json').read_text())
        c1_paths = set(filters['muninn_c1'])
        required = {
            'crates/muninn/**',
            'apps/muninn/**',
            'third_party/candle-core/**',
            'dev/muninn/c1_*.py',
            'dev/muninn/product_*.py',
            'dev/muninn/fixtures/minilm-c1-*.json',
            'dev/muninn/c1_attempt_plan.json',
            'dev/muninn/minilm_tokenizer_reference.py',
            'dev/muninn/requirements-minilm-tokenizer.txt',
            'Cargo.toml',
            'Cargo.lock',
            'rust-toolchain.toml',
            'dev/muninn/fixtures/artifact-lock.json',
            'docs/muninn/specs/tck-baseline.csv',
            '.github/workflows/ci.yml',
            '.agents/rpi/goal/muninn-functional-parity/attempt68-bundle-verify.jsonl',
        }
        self.assertTrue(required <= c1_paths, sorted(required - c1_paths))
        for relative in c1_build_record.FILES:
            self.assertTrue(
                any(fnmatch.fnmatchcase(relative, pattern) for pattern in c1_paths),
                f"C1 source material is not routed to the C1 gate: {relative}",
            )
        for tree in c1_build_record.TREES:
            self.assertTrue(
                any(pattern.endswith("/**") and tree.startswith(pattern[:-3]) for pattern in c1_paths),
                f"C1 source tree is not routed to the C1 gate: {tree}",
            )

    def test_missing_tree_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.create_complete_root(root)
            (root / c1_build_record.TREES[0] / 'source.rs').unlink()
            (root / c1_build_record.TREES[0] / 'source.rs').parent.rmdir()
            with self.assertRaises(ValueError):
                c1_build_record.materials(root)

    def test_symlinked_file_ancestor_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'root'
            external = Path(directory) / 'external-control'
            root.mkdir()
            external.mkdir()
            self.create_complete_root(root)
            (root / '.github').rename(external / '.github')
            (root / '.github').symlink_to(external / '.github', target_is_directory=True)
            with self.assertRaises(ValueError):
                c1_build_record.materials(root)
