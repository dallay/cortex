# Isolated GM-01 runner

## Status

The runner packages the capture and delegates it to Docker Compose. Real
attempts `gm01-dory-20261102-r1` and `gm01-dory-20261102-r2` were blocked at
`compose_up`, without internal captures. R2 recorded
`compose_diagnostics.status=no_containers`. Inspection with `dory mount --json`
showed that the only host share is `/Users/acosta` through `virtio-fs`; the
previous `TemporaryDirectory` placed `secrets.file` files under
`/private/var/folders`, outside that share. A non-root bind-mount test in Dory
confirmed that a temporary file there cannot be mounted (exit code 125), while
a test file under `dev/muninn/.cache` mounts and can be read (exit code 0).
This confirms that the previous secrets path was incompatible with Dory's share.

The runner now creates `reference.auth` and `rust.auth` in a private temporary
subdirectory under `dev/muninn/.cache`, retaining it until `compose down`
finishes and then removing only that subdirectory. Other staging remains
outside `.cache`. R3 finished with `comparison=pass` for this scenario. Evidence
is under `.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/gm01-dory-20261102-r3/`;
the manifest declares `h1_acceptance=not_established`. The capture does not
validate deletion, restart, the complete graph, multi-tenant isolation, scoped
ANN, or MiniLM. Global GM-01 and H1 remain open.

The manifest retains `h1_acceptance=not_established`. A successful comparison
neither accepts GM-01/H1 nor demonstrates general parity.

## Preparation and execution

The operator provides a clean `agent-memory` checkout pinned to
`9d45af5529ecb5f2fe352013468a11087f3163d4` and an evidence directory that does
not exist. The runner verifies HEAD, tracked-file status, and the bytes in
`dev/muninn/source-integrity.json`. It then materializes a verified temporary
copy before creating the build context.

From the repository root:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 dev/muninn/gm01_runner.py \
  --reference-checkout /ruta/al/agent-memory-fijado \
  --evidence .agents/rpi/plan/tasks/muninn-h1-delivery/evidence/gm01-isolated-nueva
```

The runner:

1. Validates and copies the contract fixture and clean upstream source.
2. Generates a temporary Compose project with isolated network and volume names.
3. Creates transient credentials and mounts them through `secrets.file`.
4. Builds reference/Rust runners and captures both backends.
5. Compares contract, `_metadata`, selected nodes, scores, and candidate edges.
6. Performs tenant deletion and scoped `clear_session` when the reference exposes it.
7. Writes a reproducible manifest and removes the temporary resources.

## Manifest

`manifest.json` records SHA-256 values for the contract fixture and its six input
families, local runner/tool sources, prepared upstream source, Python lock,
Compose configuration, image identities, commands, and exit codes. It also
references `reference.json`, `rust.json`, `comparison.json`, and `lifecycle.json`
with paths, sizes, and SHA-256 values. Credentials appear in neither the manifest
nor command trace.

If setup fails, the manifest retains `status=blocked` and the failure phase;
there is no valid comparison. If both backends run and differ, it retains
`status=completed`, `comparison=mismatch`, and process exit code 1. A missing
reference method is not implemented or replaced by another operation.

## Tests without Docker

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s dev/muninn/tests -p 'test_gm01_runner.py' -v
```

These tests validate the contract, manifest, temporary Compose setup, and failure
handling. They do not count as real GM-01 execution.
