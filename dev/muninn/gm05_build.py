"""Runner-owned GM-05 build from current bytes, not an external executable.

This is local provenance, not a hermetic build or signed attestation. The system
Rust toolchain and downloaded dependencies remain trusted execution inputs.
"""
from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import tomllib

from gm01_runner import (_copy_file, _include_dependencies, _validate_cargo_config,
                         digest, EXCLUDED_CONTEXT_PARTS, TARGET_SOURCE_DIRS)

# Dashboard/generated files are not Cargo inputs. Literal active includes are
# also discovered by GM-01's include helper, including resources outside these trees.
EXCLUDED = EXCLUDED_CONTEXT_PARTS | {'dashboard', '__pycache__'}
REQUIRED = ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')


def _safe_file(root, relative):
    path = root
    for part in Path(relative).parts:
        path = path / part
        if path.is_symlink():
            raise RuntimeError('Build input contains a symlink')
    if not path.is_file() or not path.resolve().is_relative_to(root):
        raise RuntimeError('Build input is not a regular workspace file')
    if path.name.startswith('.env') or path.suffix in {'.pem', '.key', '.p12'}:
        raise RuntimeError('Sensitive file is not a build input')
    return path


def _manifest_paths(value, directory, root):
    if isinstance(value, dict):
        for key, child in value.items():
            if key == 'path' and isinstance(child, str):
                path = Path(child)
                if path.is_absolute() or not (directory / path).resolve().is_relative_to(root):
                    raise RuntimeError('Cargo manifest path escapes snapshot')
            _manifest_paths(child, directory, root)
    elif isinstance(value, list):
        for child in value:
            _manifest_paths(child, directory, root)


def _config_paths(value):
    if isinstance(value, dict):
        for child in value.values():
            _config_paths(child)
    elif isinstance(value, list):
        for child in value:
            _config_paths(child)
    elif isinstance(value, str) and (Path(value).is_absolute() or '..' in Path(value).parts):
        raise RuntimeError('Cargo configuration path escapes snapshot')


def snapshot_sources(root, destination):
    """Copy hashed current workspace files, retaining dirty/untracked build inputs."""
    root = Path(root).resolve()
    paths = set(REQUIRED)
    for name in ('apps', 'crates', 'third_party'):
        base = root / name
        if base.is_symlink():
            raise RuntimeError('Build input contains a symlink')
        for directory, directories, files in os.walk(base, followlinks=False):
            directories[:] = [name for name in directories if name not in EXCLUDED]
            for name in directories + files:
                path = Path(directory) / name
                if path.is_symlink():
                    raise RuntimeError('Build input contains a symlink')
            for name in files:
                path = Path(directory) / name
                relative = path.relative_to(root).as_posix()
                _safe_file(root, relative)
                paths.add(relative)
    # Reuse GM-01's credential validation before copying Cargo configuration.
    if (root / '.cargo').is_symlink():
        raise RuntimeError('Cargo configuration directory contains a symlink')
    for name in ('.cargo/config', '.cargo/config.toml'):
        path = root / name
        if path.exists() or path.is_symlink():
            _safe_file(root, name)
            _validate_cargo_config(path)
            _config_paths(tomllib.loads(path.read_text(encoding='utf-8')))
            paths.add(name)
    for relative in REQUIRED:
        _safe_file(root, relative)
    active_sources = [root / relative for relative in paths if relative.endswith('.rs')
                      and any(relative.startswith(directory + '/')
                              for directory in TARGET_SOURCE_DIRS)]
    included = _include_dependencies(root, active_sources)
    while included:
        following = []
        for path in included:
            relative = path.relative_to(root).as_posix()
            _safe_file(root, relative)  # Reject sensitive files before hashing bytes.
            if relative not in paths and path.suffix == '.rs':
                following.append(path)
            paths.add(relative)
        included = _include_dependencies(root, following)
    manifest = {}
    destination.mkdir(mode=0o700)
    for relative in sorted(paths):
        source = _safe_file(root, relative)
        if source.name == 'Cargo.toml':
            _manifest_paths(tomllib.loads(source.read_text(encoding='utf-8')),
                            source.parent, root)
        expected = digest(source)
        _copy_file(root, destination, relative, expected)
        manifest[relative] = expected
    return manifest


@dataclass
class BuiltCandidate:
    path: Path
    manifest: dict
    target: Path

    def verify(self):
        if (self.path.is_symlink() or not self.path.is_file()
                or not self.path.resolve().is_relative_to(self.target.resolve())
                or any(path.is_symlink() for path in (self.target, self.path.parent))
                or not self.path.stat().st_mode & stat.S_IXUSR
                or digest(self.path) != self.manifest['binary_sha256']):
            raise RuntimeError('Built executable integrity check failed')


@contextmanager
def build_candidate(root):
    """Own the snapshot and fresh target for the full build-and-capture lifetime."""
    with tempfile.TemporaryDirectory(prefix='gm05-build-') as temporary:
        private = Path(temporary).resolve()
        snapshot, target = private / 'source', private / 'target'
        sources = snapshot_sources(root, snapshot)
        home = private / 'home'
        home.mkdir()
        environment = {'PATH': os.environ.get('PATH', os.defpath), 'HOME': str(home),
                       'CARGO_HOME': str(private / 'cargo'),
                       'RUSTUP_HOME': os.environ.get('RUSTUP_HOME', str(Path.home() / '.rustup'))}
        toolchain = {}
        for name, command in (('cargo', ['cargo', '--version']), ('rustc', ['rustc', '-Vv'])):
            result = subprocess.run(command, cwd=snapshot, env=environment, check=False,
                                    capture_output=True, text=True)
            if result.returncode:
                raise RuntimeError('Build toolchain inspection failed')
            toolchain[name] = result.stdout.strip()
        command = ['cargo', 'build', '--locked', '--release', '-p', 'muninn',
                   '--bin', 'muninn', '--target-dir', str(target)]
        result = subprocess.run(command, cwd=snapshot, env=environment, check=False,
                                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        if result.returncode:
            raise RuntimeError('Candidate build failed')
        # Explicit package/bin and host release layout; target did not exist before
        # this invocation. Never accept a path supplied by the caller or build log.
        binary = target / 'release' / ('muninn.exe' if os.name == 'nt' else 'muninn')
        if (binary.is_symlink() or not binary.is_file()
                or not binary.resolve().is_relative_to(target)):
            raise RuntimeError('Candidate build produced no private executable')
        manifest = {'sources': sources, 'source_manifest_sha256': hashlib.sha256(
            json.dumps(sources, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
            'build_command': command, 'build_cwd': str(snapshot), 'toolchain': toolchain,
            'binary_sha256': digest(binary)}
        artifact = BuiltCandidate(binary, manifest, target)
        artifact.verify()
        yield artifact
