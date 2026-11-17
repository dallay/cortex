# Exclusive staging visibility primitive

`artifact_fs::staging::ExclusiveStage` is an opaque, non-Clone owner of a staging
directory. It is a low-level filesystem primitive, not a complete cache preparer.

## Source-name ownership

Creation first reserves an internal UUID with an exclusive 0600 `.lease-ID`
file, then creates a 0700 `.stage-ID` directory relative to the checked parent
descriptor. No caller-selected source name is public. Retained reservation files
prevent cooperating constructors from reusing names after source rename, owner
drop, errors or successful publication.

A reservation is not a live-process lock: it intentionally remains on disk.
Abandoned directories and reservations need bounded maintenance and quotas before
any service exposes preparation. This primitive does not delete them automatically
or identify safe garbage-collection candidates.

## Publication

`publish_directory(self, target)` consumes the stage. The target must be a
64-character lowercase hexadecimal name. The caller must derive that identifier
from its authorized cache/manifest policy, not use it as an authorization token.

The method rechecks parent/source policy, compares the named source inode to the
owned descriptor, syncs the source directory, calls native no-replace rename and
syncs the parent. Linux uses `RENAME_NOREPLACE`; macOS uses `RENAME_EXCL` through
the verified safe rustix API. There is no existence-check/overwriting-rename fallback.

An existing target is never adopted as valid or deleted to enable publication.
After successful rename, a parent-sync failure returns `OutcomeUnknown`; the
target can already be visible. There is no rollback, cleanup or automatic retry.

The inode comparison is not atomic with rename. Safety relies on exclusive
source ownership among cooperating APIs and the existing trusted-host assumption;
it does not defeat malicious same-UID/root namespace mutation.

## What the result does not prove

The returned `ArtifactDir` is the already-open source inode, not a ReadyModel or
ReadyCache. Empty/incomplete model directories can be represented by this low-level
primitive. The acceptance layer must verify all required files and receipts before
using it and before claiming model readiness.

Directory sync does not prove staged file data was synchronized, and this method
does not implement Apple fullfsync or universal crash durability. Those gates,
complete cache validation, transport, weights and inference remain separate.

## Verification

Tests use temporary fixtures and synthetic markers only: existing file, empty
directory, populated directory and dangling-symlink targets stay unchanged; one
of two publishers wins; source-name reuse stays blocked; changed source inode and
unsafe modes reject publication; pre/post sync failures preserve their stated
outcomes; parent-path replacement does not redirect descriptor-bound operations.

```sh
cargo test -p muninn-bridge-spike --lib --locked --offline artifact_fs::staging::
```
