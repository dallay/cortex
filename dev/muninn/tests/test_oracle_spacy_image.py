"""Static contract for the H4 spaCy oracle image (U5a infrastructure only).

Reads the image inputs as text plus the pinned upstream lock for provenance.
No Docker invocations, network requests, package resolution or model imports.
Run from the repository root with `python -B`.
"""

import hashlib
from pathlib import Path
import re
import tomllib
import unittest


ROOT = Path(__file__).resolve().parents[3]
BASE = ROOT / "dev/muninn"
DOCKERFILE = BASE / "Dockerfile.oracle-spacy"
IGNORE = BASE / "Dockerfile.oracle-spacy.dockerignore"
REQUIREMENTS = BASE / "requirements-oracle-spacy.txt"
INPUT = BASE / "requirements-oracle-spacy.in"
ORACLE_DOCKERFILE = BASE / "Dockerfile.oracle"
LOCK = BASE / ".cache/sources/agent-memory/uv.lock"
LOCK_SHA256 = "30a0c40d5bba6ceecf9cfd6a280a066ad055bb091bbb2d6ac540c845ec943553"
MODEL_WHEEL = "en_core_web_sm-3.8.0-py3-none-any.whl"
MODEL_DIR = ".cache/model-evidence"
SPACY_PIN = ("spacy", "3.8.16")

REQUIREMENT = re.compile(
    r"([A-Za-z0-9_.-]+)==([^ \t;]+)(?:\s*;\s*.*?)?\s+"
    r"((?:--hash=sha256:[0-9a-f]{64}(?:\s+|$))+)$"
)
HASH = re.compile(r"--hash=sha256:([0-9a-f]{64})")
DIGEST_PIN = re.compile(r"^python:3\.13\.18-slim@sha256:[0-9a-f]{64}$")
SHA256 = re.compile(r"[0-9a-f]{64}")


def normalized(name):
    return re.sub(r"[-_.]+", "-", name).lower()


def logical_lines(text):
    """Join pip continuations and drop comments and blank lines."""
    result, pending = [], ""
    for raw in text.splitlines():
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        if line.endswith("\\"):
            pending += line[:-1].strip() + " "
            continue
        result.append((pending + line).strip())
        pending = ""
    if pending:
        result.append(pending.strip())
    return result


def requirements(text):
    parsed = []
    for line in logical_lines(text):
        match = REQUIREMENT.fullmatch(line)
        if match is None:
            raise AssertionError(f"not an exact, hash-pinned requirement: {line}")
        name, version, hashes = match.groups()
        parsed.append((normalized(name), version, set(HASH.findall(hashes))))
    return parsed


def lock_packages():
    packages = {}
    for package in tomllib.loads(LOCK.read_text())["package"]:
        files = {entry["hash"].split(":", 1)[1]
                 for entry in package.get("wheels", []) + [package.get("sdist")]
                 if entry}
        versions = packages.setdefault(normalized(package["name"]), {})
        if package["version"] in versions:
            raise AssertionError(f"ambiguous lock package: {package['name']}")
        versions[package["version"]] = files
    return packages


def dockerfile_lines(path=DOCKERFILE):
    """Join Dockerfile continuations so one instruction is one logical line."""
    result, pending = [], ""
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.endswith("\\"):
            pending += line[:-1].strip() + " "
            continue
        result.append((pending + line).strip())
        pending = ""
    if pending:
        result.append(pending.strip())
    return result


def oracle_base_digest():
    for line in ORACLE_DOCKERFILE.read_text().splitlines():
        match = re.match(r"FROM\s+(\S+)", line.strip())
        if match and "@" in match.group(1):
            return match.group(1).split("@", 1)[1]
    raise AssertionError("existing oracle Dockerfile pins no base digest")


class RequirementsContractTests(unittest.TestCase):
    def test_every_requirement_is_exact_and_hash_pinned(self):
        pins = requirements(REQUIREMENTS.read_text())
        self.assertTrue(pins)
        names = [name for name, _, _ in pins]
        self.assertEqual(names, sorted(set(names)))
        for name, version, hashes in pins:
            self.assertRegex(version, r"^[0-9]+(\.[0-9]+)*$")
            self.assertTrue(hashes, name)
        self.assertIn(SPACY_PIN, [(name, version) for name, version, _ in pins])

    def test_no_requirement_escapes_hash_pinning_or_the_index(self):
        text = REQUIREMENTS.read_text()
        self.assertNotRegex(text, r"https?://")
        self.assertNotIn(" @ ", text)
        self.assertEqual(len(requirements(text)), len(logical_lines(text)))
        names = {name for name, _, _ in requirements(text)}
        # The model ship is a local wheel, never an index requirement.
        self.assertNotIn(normalized("en_core_web_sm"), names)

    def test_pins_and_hashes_come_from_the_pinned_reference_lock(self):
        self.assertEqual(hashlib.sha256(LOCK.read_bytes()).hexdigest(), LOCK_SHA256)
        self.assertIn(LOCK_SHA256, REQUIREMENTS.read_text())
        packages = lock_packages()
        for name, version, hashes in requirements(REQUIREMENTS.read_text()):
            self.assertIn(name, packages, name)
            self.assertIn(version, packages[name], name)
            self.assertFalse(hashes - packages[name][version],
                             f"{name} hashes absent from the lock")

    def test_export_input_declares_the_same_direct_pin(self):
        lines = logical_lines(INPUT.read_text())
        self.assertFalse([line for line in lines if line.startswith("-")])
        self.assertIn("==".join(SPACY_PIN), [line.split()[0] for line in lines])
        text = INPUT.read_text()
        self.assertIn("uv export", text)
        self.assertIn("--extra spacy", text)
        self.assertIn(MODEL_WHEEL, text)


class DockerfileContractTests(unittest.TestCase):
    def test_every_stage_pins_the_oracle_base_by_digest(self):
        stages = [line.split()[1] for line in dockerfile_lines()
                  if line.startswith("FROM ")]
        self.assertGreaterEqual(len(stages), 2)
        for image in stages:
            self.assertRegex(image, DIGEST_PIN)
        self.assertEqual({image.split("@", 1)[1] for image in stages},
                         {oracle_base_digest()})

    def test_dependency_install_is_hash_pinned_from_the_lock(self):
        text = DOCKERFILE.read_text()
        lines = dockerfile_lines()
        self.assertIn("--require-hashes", text)
        self.assertIn("COPY requirements-oracle-spacy.txt /opt/requirements-oracle-spacy.txt",
                      lines)
        install = [line for line in lines if "-r /opt/requirements-oracle-spacy.txt" in line]
        self.assertTrue(install)
        for line in install:
            self.assertIn("--require-hashes", line)
        # Regression pin: a venv built at a temporary path leaves copied
        # console scripts with a dead interpreter shebang.
        self.assertIn("python -m venv /opt/venv", text)
        self.assertNotIn("/tmp/venv", text)
        for forbidden in ("spacy download", "pip download", "curl", "wget",
                          "--index-url", "--extra-index-url", "--find-links", "https://"):
            self.assertNotIn(forbidden, text)

    def test_model_wheel_is_copied_by_filename_and_installed_locally(self):
        lines = dockerfile_lines()
        self.assertIn(f"COPY {MODEL_DIR}/{MODEL_WHEEL} /tmp/model/{MODEL_WHEEL}", lines)
        install = [line for line in lines if "pip install" in line and MODEL_WHEEL in line]
        self.assertTrue(install, "no local model wheel install")
        for line in install:
            self.assertIn("--no-deps", line)
            self.assertIn("--no-index", line)
        checks = [line for line in lines if "sha256sum -c" in line]
        self.assertTrue(checks, "the copied model wheel digest is never verified")
        self.assertTrue(all(SHA256.search(line) for line in checks), checks)

    def test_declared_model_digest_matches_the_cached_wheel_when_present(self):
        wheel = BASE / MODEL_DIR / MODEL_WHEEL
        if not wheel.is_file():
            self.skipTest("cached model wheel is absent")
        digest = hashlib.sha256(wheel.read_bytes()).hexdigest()
        checks = [line for line in dockerfile_lines() if "sha256sum -c" in line]
        self.assertTrue(any(digest in line for line in checks))

    def test_image_runs_as_nonroot_without_credentials(self):
        lines = dockerfile_lines()
        users = [line.split(None, 1)[1] for line in lines if line.startswith("USER ")]
        self.assertTrue(users)
        self.assertTrue(all(user.startswith("7474:7474") for user in users), users)
        text = DOCKERFILE.read_text().lower()
        for forbidden in ("password", "token", "secret", "credential", "auth"):
            self.assertNotIn(forbidden, text)

    def test_build_context_exposes_only_owned_inputs(self):
        lines = [line.strip() for line in IGNORE.read_text().splitlines() if line.strip()]
        self.assertEqual(lines[0], "**")
        self.assertEqual({line[1:] for line in lines if line.startswith("!")},
                         {"requirements-oracle-spacy.txt", f"{MODEL_DIR}/{MODEL_WHEEL}"})


if __name__ == "__main__":
    unittest.main()
