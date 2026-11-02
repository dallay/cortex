# Local C0 receipt validation

This stdlib-only validator reads files. It does not launch commands, load models,
or produce an orchestration receipt. A prospective run40 producer must write the
receipt after collecting its declared manifests and outputs.

```sh
python3 dev/muninn/validate_c0_receipt.py --workspace "$PWD" \
  --receipt dev/muninn/artifacts/attempt40/receipt.json
PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests \
  -p test_validate_c0_receipt.py -v
```

## Exact schema (version 1)

All receipt objects reject extra keys. The comparison artifact can contain extra
measurement fields. SHA-256 values are 64 lowercase hexadecimal characters.

```text
receipt = {
  "schema_version": 1,
  "run_id": nonblank string,
  "scope": "c0_local_provenance_not_authentication",
  "numerical_acceptance": false,
  "files_before": [file, ...],
  "files_after": [file, ...],
  "outputs": [output, output, output, output],
  "steps": [step, ...],
  "runtime": {"rustc": nonblank string, "cargo": nonblank string, "host": nonblank string},
  "bundle_id": sha256
}
file = {"path": relative_path, "size": integer, "sha256": sha256}
output = {"role": role, "path": relative_path, "size": integer, "sha256": sha256}
step = {"id": step_id, "argv": [string, ...], "exit_code": 0}
```

Required step IDs, each once: `build`, `rust_export`, `bundle_verify`,
`python_export`, `compare`. One additional `tests` step is allowed. Step order is
not evidence and is not enforced. `argv` must be a nonempty list of strings;
its contents are neither executed nor treated as authority.

Output roles, exactly once each: `rust_export`, `python_export`, `comparison`,
`bundle_verify`. No additional output roles are allowed.

Each manifest is nonempty and has unique paths. Outputs cannot overlap the input
manifest. The combined number of entries in all three lists is at most 256
(before and after entries both count). Before and after must contain identical
path/size/hash mappings; list order does not matter. Integers cannot be booleans.
File sizes range from zero to 256 MiB. Hashing uses chunks of at most 1 MiB and
stops at the file limit. The current local Rust export binary is approximately
30 MiB, below this limit.

Paths use `/` separators and must be relative to the workspace. Absolute paths,
empty components, `.`, `..`, backslashes, drive prefixes and NUL are rejected.
Every component is opened without following symlinks. Only regular files are
read. The receipt path obeys the same rules. The workspace argument itself is
resolved as the caller-selected root. Use a quiescent workspace: validation is
not an atomic filesystem snapshot and does not protect against later changes.

The receipt and comparison JSON are each limited to 256 KiB. Duplicate keys,
nonfinite numbers (including overflow), invalid UTF-8 and malformed JSON fail.
Actual size and hash must match all `files_after` and `outputs` entries.
Exactly one input path must have basename `minilm-c0-v1.json`. The comparison
artifact must contain `state: "measured_not_accepted"`, `corpus_id: "c0-v1"`,
and `input_sha256` equal to that input's manifest hash.

## Result and limits of evidence

Success prints JSON with `state: "valid_provenance_not_acceptance"`, the
`run_id`, the fixed `scope`, and `numerical_acceptance: false`; exit status is 0.
Failure prints `{"state":"invalid_receipt","error":category}` and exits 1.
Fixed categories are `schema`, `path`, `file_access`, `duplicate_key`,
`nonfinite`, `json_limit`, `json`, `manifest`, `file_limit`, `roles`,
`manifest_changed`, `steps`, `file_mismatch`, and `comparison`. CLI usage errors
use argparse's standard exit status 2.

This proves only consistency with local files at validation time. It does not
authenticate who generated them, prove that commands ran, prove that an output
came from a particular executable, establish historical before-state, verify
bundle contents from the bundle ID, or guarantee build reproducibility.
The runtime sidecar fields and zero exit codes are untrusted claims, not trusted
facts. Export and bundle verification contents are hashed, not independently
recomputed. No numerical acceptance or model-quality conclusion follows.
