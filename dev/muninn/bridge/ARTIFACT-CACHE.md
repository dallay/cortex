# Verified artifact-set acceptance

`artifact_fs::staging::model` verifies stored model artifacts. It does not run
MiniLM, certify numerical behavior or authorize a preparation service.

## Staging

`ModelArtifactStage` owns the pinned manifest and an exclusive staging directory.
The aggregate declared artifact size is bounded before filesystem mutation.
Unlisted paths, duplicate staging calls and invalid source bytes are rejected
before writing. Invalid bytes can be retried because no disk mutation occurred.

After mutation can begin, the stage is poisoned until the entire file operation
succeeds. Each file is created exclusively, written from the verified buffer,
rewound and verified on the same descriptor. The file becomes 0400 before its
synchronization request. A failed mutation/sync cannot be repaired by overwriting
or silently deleting the partial entry.

Writable handles are local to the operation and close before success returns.
macOS requests `F_FULLFSYNC` through rustix; Linux uses `sync_all`. Directory
synchronization remains distinct from file synchronization. These requests and
tests do not demonstrate universal power-loss recovery.

## Publication and receipt

Publication requires every declared artifact to have completed staging, with no
poisoned state. It rechecks each required file's bytes and readonly permissions.
The private canonical receipt contains schema version, model ID, revision and
path-sorted artifact metadata. Its exact bytes determine the artifact bundle ID.
This identity is not an `EmbeddingSpace` or a frozen numerical policy.

The receipt is created exclusively and verified, made readonly and synchronized.
The nested and root directories become 0500 before native no-replace publication.
An existing destination is never replaced or implicitly accepted. The lower
primitive's `OutcomeUnknown` still means the destination can already be visible;
there is no automatic rollback or retry.

## Reopening and reading

`VerifiedArtifactCache::open` checks a bounded receipt against the exact expected
serialization and rechecks every required artifact. The root and nested directory
must be 0500, and required files/receipt must be 0400. Whitespace changes,
truncation or extension do not produce an equivalent receipt.

`read_artifact` permits only declared paths and returns newly verified owned
bytes. It does not reuse a past disk-integrity result to authorize an unchecked
read. The future encoder must consume those returned buffers, not reopen paths.
No receipt or filesystem validation executes model configuration or model code.

## Remaining gates

- The checks are snapshots under the existing ACL/filesystem/trusted-host policy,
  not permanent protection against owner/root changes.
- Acceptance checks required artifacts and the receipt; it is not a generic
  filesystem inventory or backup/export format.
- Retained reservations, orphans, quotas, maintenance and crash recovery still
  need an operational contract before exposing preparation as a service.
- No downloader, real model weights, encoder, C0/C1 acceptance or new TCK result
  is provided by this unit. Synthetic test files are not inference evidence.

```sh
cargo test -p muninn-bridge-spike --lib --locked --offline \
  artifact_fs::staging::model::
```
