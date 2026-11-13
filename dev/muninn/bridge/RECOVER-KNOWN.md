# Bounded recovery of the known stopped preparation

This is a finite operator workflow for the exact three-file prefix left by the
recorded failed MiniLM preparation. It is not a general resume or cleanup service.

## Host precondition

The trusted host must already have observed the original worker terminate. The
recorded supervisor reported exit code 1 for this operation. Legacy markers have
no liveness lock: their existence, a PID, identity arguments or a JSON receipt
cannot establish worker termination or grant authorization.

Do not use this workflow against an active or unknown preparer. A server accepting
untrusted recovery requests would need a separate authorization/lifetime contract.
The finite CLI does not provide that service boundary.

## Exact snapshot and admission

The operator supplies the expected root device/inode and canonical source UUID.
The pinned production manifest supplies the first three root-file paths and every
expected size/hash. The implementation rejects other partial layouts.

Checks include the exact root inventory, matching identity, same-device source,
owner/mode/native ACL policy, empty single-link marker/lease files and single-link
readonly artifact files. Bounded descriptor enumeration requires EOF: six calls
cover the pre-claim inventory and seven cover the inventory after claiming.
Root/source/control/file identities and hashes are rechecked before effects and
before copying. No incoming directory is adopted based only on existence.

The logical byte budget is preserved artifact bytes plus one complete new set plus
a 64 KiB metadata allowance, no more than 128 MiB. Available filesystem space is
also checked, but not reserved. This is neither a physical allocation quota nor
an RSS bound; verification can hold more than one artifact-sized memory buffer.

An exclusive `.muninn-recovery-once` marker admits one cooperating recovery. It
persists on every failure. Sync errors can report `OutcomeUnknown`; no marker is
deleted and no automatic retry or third attempt follows.

## Copy and acquisition

Consuming the claim creates one distinct UUID-named exclusive stage in the same
root. The old stage and lease are never renamed, chmodded, overwritten or reused.
Each prefix file is opened read-only on a fresh descriptor, rewound and verified
again through the existing staging writer. Prefix buffers are short-lived and
processed sequentially.

Only after the complete prefix copy succeeds does the CLI construct its HTTP
client and fetch the remaining pinned artifacts. Publication and readback retain
the existing no-replace, readonly, exact-receipt and fresh-buffer checks. The
original parent descriptor is retained for reopening the result.

```sh
cargo build -p muninn-bridge-spike --example prepare_minilm --locked --offline
target/debug/examples/prepare_minilm recover-known \
  /physical/absolute/original-root DEVICE INODE SOURCE_UUID
```

The identity arguments are data. They do not replace the host precondition or
disk validation. Do not reuse a stale before-fix executable. `artifact_reused_and_staged`
records a rehashed/copied prefix; final `cache_artifact_verified` events record
fresh actual buffers from the accepted bundle. Output contains no request URLs,
signed queries, model content or underlying native error strings.

No deletion, root switch or ordinary `prepare` retry is part of recovery. A failed
recovery preserves both old data and any new partial stage for explicit disposition.
General quotas, orphan maintenance, crash recovery, inference, C1, TCK and full
parity remain separate gates even if all model artifacts are acquired.
