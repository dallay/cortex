"""Fail-closed execution runner for frozen C1 stages.

The runner never performs model inference itself and never touches Neo4j. It
executes external exporter processes against byte-exact snapshots of frozen
inputs, captures their output and logs, checks what they claim to have
consumed, and writes an attempt receipt only when every verification passes.

Receipts are fail-closed. Any missing or unverifiable fact aborts the attempt
with a ``RunnerError`` and leaves no receipt behind, so an interrupted or
tampered attempt can never be presented as a completed one.

Guarantees and limits, stated so that no caller over-reads the receipt:

* Exact byte delivery is enforced. Every consumed input is read once through
  the hardened admission helper and materialized into a private staging file
  with mode ``0400``; the process then receives exactly those bytes, through
  stdin or through that path.
* Exact byte consumption is checked against what the exporter itself reports.
  The current exporter interfaces accept a file path and return the digest of
  the bytes they read. The runner cross-checks that digest against the
  snapshot digest and re-reads the staged file afterwards, but the value is an
  exporter self-report, not an independent attestation of the consumed stream.
* Native binaries require GitHub attestations for both the binary and build
  record, constrained to the exact protected-main workflow and source commit.
  Python interpreter stages bind scripts through frozen source inputs.
"""
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import stat
import subprocess
import sys
import tempfile

import c1_execution
import c1_provenance

CONTRACT = "minilm-cpu-f32-v1"
RECEIPT_NAME = "attempt-receipt.json"
RECEIPT_SCHEMA_VERSION = 1
HEX_SHA256 = re.compile(r"[0-9a-f]{64}")
ROLE_PATTERN = re.compile(r"[a-z0-9][a-z0-9_-]{0,31}")
OUTPUT_NAME_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}")
PLACEHOLDER_PATTERN = re.compile(r"\{[A-Za-z0-9_-]+\}")


class RunnerError(RuntimeError):
    """A provenance or execution check failed; no receipt was written."""

    def __init__(self, category, detail=None):
        super().__init__(category if detail is None else "{0}: {1}".format(category, detail))
        self.category = category
        self.detail = detail


@dataclass(frozen=True)
class SourceBinding:
    """One source file of a trusted build record."""

    path: str
    sha256: str


@dataclass(frozen=True)
class BuildBinding:
    """Trusted record binding an executable to the sources it was built from."""

    binary: str
    binary_sha256: str
    sources: tuple = ()
    toolchain: tuple = ()
    provenance: tuple = ()


@dataclass(frozen=True)
class InputBinding:
    """A frozen input, its role in the stage, and how the stage receives it."""

    role: str
    relative: str
    delivery: str


@dataclass(frozen=True)
class ConsumptionCheck:
    """A JSON field that must carry the digest of the consumed snapshot.

    ``output`` names a captured output file, or the literal ``stdout`` when the
    exporter writes its report to the standard stream, as the current exporter
    interfaces do.
    """

    output: str
    role: str
    field: str


@dataclass(frozen=True)
class Stage:
    """One external exporter invocation inside an attempt."""

    name: str
    argv: tuple
    build: BuildBinding
    inputs: tuple = ()
    outputs: tuple = ()
    consumed: tuple = ()
    output_dir: str = ""
    timeout: float = 900.0
    env: tuple = ()


def _sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def _canonical_bytes(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":")).encode("utf-8")


def _pairs_no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise RunnerError("report_duplicate_key")
        result[key] = value
    return result


def _read_regular(path, category):
    """Read a regular file without following a symlink at the final component."""
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    except OSError:
        raise RunnerError(category) from None
    try:
        info = os.fstat(descriptor)
        if not stat.S_ISREG(info.st_mode):
            raise RunnerError(category)
        chunks = []
        while True:
            chunk = os.read(descriptor, 1024 * 1024)
            if not chunk:
                break
            chunks.append(chunk)
        return b"".join(chunks)
    finally:
        os.close(descriptor)


def _digest_of(path, category):
    return _sha256_bytes(_read_regular(path, category))


def _write_exclusive(path, data, mode=0o600):
    """Create a file that must not exist yet, then restrict its mode."""
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, mode)
    except FileExistsError:
        raise RunnerError("file_preexisting", str(path)) from None
    with os.fdopen(descriptor, "wb") as handle:
        handle.write(data)
    os.chmod(path, 0o400)


def _require_absent(path, category):
    if os.path.lexists(path):
        raise RunnerError(category, str(path))


def _path_identity(path):
    """Device and inode of a path, used to detect a swapped directory or file."""
    try:
        info = os.lstat(path)
    except OSError:
        raise RunnerError("path_identity", str(path)) from None
    return (info.st_dev, info.st_ino)


def _require_real_directory(path, category):
    try:
        info = os.lstat(path)
    except OSError:
        raise RunnerError(category, str(path)) from None
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
        raise RunnerError(category, str(path))


def _field_value(document, field):
    current = document
    for part in field.split("."):
        if type(current) is not dict or part not in current:
            return None
        current = current[part]
    return current


def _manifest_source_digest(admitted, source_path):
    """Return the frozen digest of a build source, or fail closed.

    A build record may only cite sources that the frozen manifest admits. The
    manifest binds the exact bytes of every input, so a source outside it is
    not part of the attempt's trusted inventory.
    """
    try:
        relative = Path(os.path.realpath(str(source_path))).relative_to(
            Path(os.path.realpath(str(admitted.root))))
    except ValueError:
        raise RunnerError("build_source_not_admitted", str(source_path)) from None
    key = relative.as_posix()
    for entry_path, _size, entry_digest in admitted.entries:
        if entry_path == key:
            return key, entry_digest
    raise RunnerError("build_source_not_admitted", key)


def _verify_build(build, admitted):
    """Check the executable and its sources against the trusted build record."""
    if not isinstance(build, BuildBinding):
        raise RunnerError("build_record")
    if type(build.binary_sha256) is not str or not HEX_SHA256.fullmatch(build.binary_sha256):
        raise RunnerError("build_binary_digest_argument")
    binary = Path(build.binary)
    if not binary.is_absolute():
        raise RunnerError("build_binary_argument")
    try:
        info = os.lstat(binary)
    except OSError:
        raise RunnerError("build_binary_missing", str(binary)) from None
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise RunnerError("build_binary_invalid", str(binary))
    if not os.access(str(binary), os.X_OK):
        raise RunnerError("build_binary_not_executable", str(binary))
    if _digest_of(binary, "build_binary_invalid") != build.binary_sha256:
        raise RunnerError("build_binary_digest_mismatch", str(binary))
    if str(binary.resolve()) != str(Path(sys.executable).resolve()):
        if not build.provenance:
            raise RunnerError("missing_build_provenance")
        try:
            args = list(build.provenance)
            if len(args) == 5:
                args[3] = [{"path": Path(source.path).relative_to(admitted.root).as_posix(),
                            "size": Path(source.path).stat().st_size,
                            "sha256": source.sha256} for source in build.sources]
            c1_provenance.verify(binary, *args)
        except (c1_provenance.ProvenanceError, TypeError) as error:
            raise RunnerError("build_provenance_rejected", str(error)) from error
    bound_sources = []
    for source in build.sources:
        if type(source.sha256) is not str or not HEX_SHA256.fullmatch(source.sha256):
            raise RunnerError("build_source_digest_argument")
        source_path = Path(source.path)
        if not source_path.is_absolute():
            raise RunnerError("build_source_argument")
        try:
            source_info = os.lstat(source_path)
        except OSError:
            raise RunnerError("build_source_missing", str(source_path)) from None
        if stat.S_ISLNK(source_info.st_mode) or not stat.S_ISREG(source_info.st_mode):
            raise RunnerError("build_source_invalid", str(source_path))
        if _digest_of(source_path, "build_source_invalid") != source.sha256:
            raise RunnerError("build_source_digest_mismatch", str(source_path))
        manifest_path, manifest_digest = _manifest_source_digest(admitted, source.path)
        if manifest_digest != source.sha256:
            raise RunnerError("build_source_manifest_digest_mismatch", manifest_path)
        bound_sources.append({"path": source.path, "sha256": source.sha256,
                              "manifest_path": manifest_path, "admitted": True})
    return {
        "binary": str(binary),
        "binary_sha256": build.binary_sha256,
        "sources": bound_sources,
        "toolchain": [list(entry) for entry in build.toolchain],
    }


def _stage_output_dir(stage, attempt_dir):
    relative = stage.output_dir or os.path.join("outputs", stage.name)
    if os.path.isabs(relative) or ".." in relative.split("/"):
        raise RunnerError("stage_output_dir_argument")
    return attempt_dir / relative


def _materialize_inputs(stage, admitted, staging):
    """Read every frozen input once and stage its exact bytes."""
    roles = {}
    stdin_bytes = None
    for binding in stage.inputs:
        if type(binding.role) is not str or not ROLE_PATTERN.fullmatch(binding.role):
            raise RunnerError("input_role_argument")
        if binding.role in roles:
            raise RunnerError("input_role_duplicate")
        if binding.delivery not in ("stdin", "path"):
            raise RunnerError("input_delivery_argument")
        snapshot = c1_execution.read_frozen_input(admitted, binding.relative)
        roles[binding.role] = {"relative": binding.relative, "size": snapshot.size,
                               "sha256": snapshot.sha256, "delivery": binding.delivery}
        if binding.delivery == "stdin":
            if stdin_bytes is not None:
                raise RunnerError("input_stdin_conflict")
            stdin_bytes = snapshot.data
        else:
            staged = staging / binding.role
            _write_exclusive(staged, snapshot.data)
            roles[binding.role]["staged_path"] = str(staged)
    return roles, stdin_bytes


def _resolve_argv(stage, roles, output_dir, attempt_dir):
    argv = []
    for argument in stage.argv:
        if type(argument) is not str:
            raise RunnerError("stage_argv_argument")
        resolved = argument
        for role in roles:
            token = "{" + role + "}"
            if token in resolved:
                staged = roles[role].get("staged_path")
                if staged is None:
                    raise RunnerError("stage_argv_placeholder_unknown", token)
                resolved = resolved.replace(token, staged)
        resolved = resolved.replace("{output_dir}", str(output_dir))
        resolved = resolved.replace("{attempt}", str(attempt_dir))
        leftover = PLACEHOLDER_PATTERN.search(resolved)
        if leftover is not None:
            raise RunnerError("stage_argv_placeholder_unknown", leftover.group(0))
        argv.append(resolved)
    if not argv:
        raise RunnerError("stage_argv_empty")
    return argv


def _stage_environment(stage):
    environment = {}
    for name in ("PATH", "HOME", "LANG", "LC_ALL", "TMPDIR"):
        value = os.environ.get(name)
        if value:
            environment[name] = value
    environment.setdefault("PATH", "/usr/bin:/bin")
    for entry in stage.env:
        if type(entry) is not tuple or len(entry) != 2:
            raise RunnerError("stage_env_argument")
        environment[entry[0]] = entry[1]
    return environment


def _run_process(argv, cwd, stdin_bytes, stdout_path, stderr_path, environment, timeout):
    with open(stdout_path, "wb") as stdout_handle, open(stderr_path, "wb") as stderr_handle:
        process = subprocess.Popen(
            argv,
            cwd=str(cwd),
            stdin=subprocess.PIPE if stdin_bytes is not None else subprocess.DEVNULL,
            stdout=stdout_handle,
            stderr=stderr_handle,
            env=environment,
            start_new_session=True,
        )
        try:
            process.communicate(input=stdin_bytes, timeout=timeout)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except OSError:
                process.kill()
            process.wait()
            raise RunnerError("stage_timeout", " ".join(argv)) from None
    return process.returncode


def _capture_outputs(stage, output_dir):
    records = []
    for name in stage.outputs:
        if type(name) is not str or not OUTPUT_NAME_PATTERN.fullmatch(name):
            raise RunnerError("stage_output_name_argument")
        path = output_dir / name
        if not os.path.lexists(path):
            raise RunnerError("output_missing", name)
        data = _read_regular(path, "output_invalid")
        records.append({"name": name, "path": str(path), "size": len(data),
                        "sha256": _sha256_bytes(data)})
    return records


def _verify_consumption(stage, output_dir, roles, records, streams):
    by_name = {record["name"]: record for record in records}
    checks = []
    for check in stage.consumed:
        if type(check) is not ConsumptionCheck:
            raise RunnerError("stage_consumption_argument")
        if check.role not in roles:
            raise RunnerError("consumption_role_unknown", check.role)
        if check.output == "stdout":
            source = streams["stdout"]
        else:
            record = by_name.get(check.output)
            if record is None:
                raise RunnerError("consumption_output_unknown", check.output)
            source = output_dir / check.output
        try:
            document = json.loads(_read_regular(source, "output_invalid").decode("utf-8"),
                                  object_pairs_hook=_pairs_no_duplicates)
        except ValueError:
            raise RunnerError("report_parse_failed", check.output) from None
        reported = _field_value(document, check.field)
        if reported != roles[check.role]["sha256"]:
            raise RunnerError("output_consumption_mismatch",
                              "{0}.{1}".format(check.output, check.field))
        checks.append({"output": check.output, "role": check.role, "field": check.field,
                       "snapshot_sha256": roles[check.role]["sha256"],
                       "reported_sha256": reported})
    return checks


def _verify_immutable(records, roles, staging):
    """Re-read captured outputs and staged inputs; any drift aborts the attempt."""
    for record in records:
        if _digest_of(record["path"], "output_invalid") != record["sha256"]:
            raise RunnerError("output_mutated", record["name"])
    for role, entry in roles.items():
        staged = entry.get("staged_path")
        if staged is None:
            continue
        if _digest_of(staged, "input_mutated") != entry["sha256"]:
            raise RunnerError("input_mutated", role)


def _seal(records, logs):
    """Make captured evidence read-only once it has been verified."""
    for record in records:
        try:
            os.chmod(record["path"], 0o400)
        except OSError:
            raise RunnerError("output_seal_failed", record["name"]) from None
    for record in logs:
        try:
            os.chmod(record["path"], 0o400)
        except OSError:
            raise RunnerError("log_seal_failed", record["path"]) from None


def _run_stage(stage, admitted, attempt_dir, logs_dir, staging, after_capture):
    if type(stage) is not Stage:
        raise RunnerError("stage_argument")
    if type(stage.name) is not str or not OUTPUT_NAME_PATTERN.fullmatch(stage.name):
        raise RunnerError("stage_name_argument")
    build_record = _verify_build(stage.build, admitted)
    roles, stdin_bytes = _materialize_inputs(stage, admitted, staging)
    output_dir = _stage_output_dir(stage, attempt_dir)
    _require_absent(output_dir, "attempt_output_preexisting")
    parent = output_dir.parent
    if parent != attempt_dir:
        os.makedirs(parent, mode=0o700, exist_ok=True)
    try:
        os.mkdir(output_dir, 0o700)
    except FileExistsError:
        raise RunnerError("attempt_output_preexisting", str(output_dir)) from None
    _require_real_directory(output_dir, "output_dir_invalid")

    argv = _resolve_argv(stage, roles, output_dir, attempt_dir)
    stdout_path = logs_dir / (stage.name + ".stdout.log")
    stderr_path = logs_dir / (stage.name + ".stderr.log")
    returncode = _run_process(argv, attempt_dir, stdin_bytes, stdout_path, stderr_path,
                               _stage_environment(stage), stage.timeout)
    if returncode != 0:
        raise RunnerError("stage_failed", "{0} exited with {1}".format(stage.name, returncode))

    _require_real_directory(output_dir, "output_dir_invalid")
    records = _capture_outputs(stage, output_dir)
    streams = {"stdout": stdout_path, "stderr": stderr_path}
    consumption = _verify_consumption(stage, output_dir, roles, records, streams)
    logs = [{"stream": "stdout", "path": str(stdout_path),
             "sha256": _digest_of(stdout_path, "log_invalid")},
            {"stream": "stderr", "path": str(stderr_path),
             "sha256": _digest_of(stderr_path, "log_invalid")}]

    if after_capture is not None:
        after_capture(stage, attempt_dir)

    _verify_immutable(records, roles, staging)
    _seal(records, logs)

    return {
        "name": stage.name,
        "argv": argv,
        "env": [list(entry) for entry in stage.env],
        "build": build_record,
        "inputs": {role: {key: value for key, value in entry.items() if key != "staged_path"}
                   for role, entry in roles.items()},
        "outputs": records,
        "consumed": consumption,
        "logs": logs,
        "returncode": returncode,
    }


def _runtime_identity():
    return {
        "python": platform.python_version(),
        "implementation": platform.python_implementation(),
        "platform": platform.platform(),
        "machine": platform.machine(),
        "executable": sys.executable,
    }


def run_attempt(root, manifest_path, manifest_sha256, stages, attempt_dir, after_capture=None):
    """Run every stage against frozen snapshots and write a receipt when sound.

    The attempt directory must not exist. On any failure the attempt leaves
    partial evidence behind but never a receipt.

    ``after_capture`` is an optional callback invoked with the stage and the
    attempt directory once the outputs have been captured and before they are
    re-verified. It exists so that tests can simulate concurrent mutation.
    Production callers pass ``None``.
    """
    attempt_dir = Path(attempt_dir)
    if not attempt_dir.is_absolute():
        raise RunnerError("attempt_dir_argument")
    _require_absent(attempt_dir, "attempt_dir_preexisting")
    stages = tuple(stages)
    if not stages:
        raise RunnerError("stage_missing")
    try:
        admitted = c1_execution.admit(root, manifest_path, manifest_sha256)
    except c1_execution.ProvenanceError as error:
        raise RunnerError("admission_failed", str(error)) from None
    root_identity = _path_identity(admitted.root)
    manifest_identity = _path_identity(admitted.manifest_path)

    try:
        os.mkdir(attempt_dir, 0o700)
    except FileExistsError:
        raise RunnerError("attempt_dir_preexisting", str(attempt_dir)) from None
    logs_dir = attempt_dir / "logs"
    os.mkdir(logs_dir, 0o700)
    staging = Path(tempfile.mkdtemp(prefix="c1-run-"))
    os.chmod(staging, 0o700)
    try:
        records = []
        inputs = {}
        for stage in stages:
            record = _run_stage(stage, admitted, attempt_dir, logs_dir, staging, after_capture)
            records.append(record)
            for role, entry in record["inputs"].items():
                if role in inputs and inputs[role] != entry:
                    raise RunnerError("input_role_conflict", role)
                inputs[role] = entry
        try:
            if _path_identity(admitted.root) != root_identity:
                raise RunnerError("workspace_replaced", str(admitted.root))
            if _path_identity(admitted.manifest_path) != manifest_identity:
                raise RunnerError("manifest_replaced", str(admitted.manifest_path))
            c1_execution.verify_unchanged(admitted)
        except c1_execution.ProvenanceError as error:
            raise RunnerError("frozen_inputs_changed", str(error)) from None

        receipt = {
            "schema_version": RECEIPT_SCHEMA_VERSION,
            "status": "complete",
            "numeric_contract_version": CONTRACT,
            "workspace_root": str(admitted.root),
            "manifest": {"path": str(admitted.manifest_path),
                         "sha256": admitted.manifest_sha256},
            "frozen_inputs": [{"path": path, "size": size, "sha256": digest}
                              for path, size, digest in admitted.entries],
            "runtime": _runtime_identity(),
            "inputs": inputs,
            "stages": records,
        }
        receipt["receipt_sha256"] = _sha256_bytes(_canonical_bytes(receipt))
        _write_exclusive(attempt_dir / RECEIPT_NAME,
                         json.dumps(receipt, sort_keys=True, indent=2).encode("utf-8") + b"\n",
                         mode=0o600)
        return receipt
    finally:
        shutil.rmtree(staging, ignore_errors=True)
