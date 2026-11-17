# Descriptor ACL policy

`ArtifactDir` now checks ACLs on the actual opened descriptor before returning
a root, child directory or regular file. It rejects any extended ACL, including
restrictive entries and directory default ACLs. This is a conservative artifact
preparer policy, not an upstream memory authorization contract.

## Linux

The initial supported local filesystems are ext-family, tmpfs and overlay. Other
filesystem types fail closed. `rustix::fs::fgetxattr_query` checks access ACLs and,
for directories, default ACLs. Only `NODATA` counts as absence. Success, including
size zero, means an ACL is present; other errors mean inspection is unavailable.
The overlay type identifies a layer, not its backing mounts. The trusted host
must exclude untrusted network backing; this type check does not prove locality
or inspect a remote server's separate authorization mechanisms.

## macOS

The initial scope is linked APFS files/directories. Descriptor identity and link
count are checked around the native ACL query. Synthetic, network, unknown and
unlinked cases are not accepted.

The pinned macOS-only `calcifer-macos-acl` wrapper calls `acl_get_fd_np`; it does
not inspect `/dev/fd` paths. Its `ENOENT`-to-empty behavior depends on the native
linked-APFS-vnode assumption. The underlying Apple function can also propagate
metadata errors, so this does not prove that every hypothetical kernel failure
can be distinguished from an absent ACL. This residual limit is recorded; it
must not be generalized to unsupported filesystems.

The small, recent FFI dependency has no dependencies or build script. Its archive
checksum and source were checked against immutable repository commit
`24a15cc4f7c46802d93d2f9cc93e45e1d5a5313e`; the MIT license text and copyright
are preserved in `docs/muninn/licenses/calcifer-macos-acl-0.1.0-MIT.txt`. The
published archive omitted that text. macOS distribution must include this notice;
no distribution approval is implied by these checks.

No production path calls the wrapper's ACL-clearing API or weakens Cortex's own
`unsafe_code = "deny"`. ACL fixtures use native tools only inside temporary tests.

This unpublished spike is verified using the repository's `rust-toolchain.toml`.
It does not inherit the workspace's older, unverified MSRV declaration: the new
macOS dependency already requires a newer compiler. No independent minimum Rust
version is claimed without a dedicated compatibility test; release policy is a
separate gate. The workspace compiler and lints are not changed here.

## Guarantees not provided

The check is a snapshot, not a lease against later owner/root changes. It does
not freeze an inode, provide shared-user authorization, verify model weights,
publish an immutable cache or establish durable storage. Staging ownership,
exclusive publication, file synchronization, downloads and model inference remain
unimplemented gates.

The earlier READONLY T1.2b1 receipt describes the pre-ACL implementation. Use the
new ACL receipt for the additional behavior, not that historical receipt.
