# C1 exporter

Minimal extraction from `dev/muninn/bridge`, not a MiniLM acceptance verdict.
No network client, bridge server, Neo4j adapter or TLS stack is included.

## Activation

The default feature set is empty. Candle, safetensors and tokenizers are optional.
The binary requires `cpu-f32`, which retains the candidate CPU/F32 contract:

```sh
cargo check -p muninn-c1-exporter --locked
cargo test -p muninn-c1-exporter --locked
cargo build -p muninn-c1-exporter --bin export_minilm_c1 --features cpu-f32 --locked
cargo test -p muninn-c1-exporter --features cpu-f32 --locked
```

Building or testing this package does not authorize inference. Verify binary
provenance separately before executing the exporter. The existing actual-cache
differential test remains ignored and requires explicit authorization and cache.

## Filesystem test diagnostics

Unit-test builds emit `[c1-fs-test] operation=<fixed label> errno=<native code>`
when instrumented staging, freezing, receipt or publication operations fail.
The diagnostics contain neither paths nor native error messages. An injected
error without a native code reports `errno=None`. Error classification, permissions,
synchronization and exclusive publication remain unchanged. Normal library and
exporter builds do not compile the diagnostic module.

On the affected macOS 15 runner, first run the focused publication test:

```sh
cargo test --locked -p muninn-c1-exporter --features cpu-f32 --target aarch64-apple-darwin publishes_only_a_complete_verified_artifact_set_with_its_receipt -- --nocapture
```

Then run the package tests without a filter. Failed tests include their captured
diagnostics by default; `--nocapture` also displays expected injected failures.
The labels distinguish `freeze.chmod_directory`, `freeze.sync_directory`,
`publish.pre_sync`, `publish.rename_noreplace`, `publish.post_sync` and file/receipt
operations.

## Publication order on macOS 15

macOS 15 (Darwin 24) denies renaming a directory whose mode lacks write permission.
The denial returns `EACCES` and does not depend on open descriptors below the
directory, on `RENAME_EXCL`, or on the destination name. Newer macOS releases
accept the same rename, so the defect is invisible on a recent workstation.

The staged directory therefore stays writable (`0o700`) until the rename publishes
it, and the published entry is frozen to `0o500` right after the rename and before
the parent sync makes the new name durable. Freezing before the rename cannot work
on macOS 15, and freezing after the parent sync would let a crash expose a
writable published directory under a durable name.

<!-- coderabbit-ai-generated:v1 -->

<!-- coderabbit-ai-generated-section:v1:417d21557b00c6e6be816f2475c740543e5ae0cacdf5c1fb32d35bdf87612384 -->

The interval between the rename and the freeze keeps the root directory writable
for its owner only. The artifact files are read-only (`0o400`), and the `1_Pooling`
child directory is `0o500`, but the owner can replace top-level artifacts during
this interval. Reopening the cache verifies the receipt and all artifact digests;
reading an artifact checks its digest again. These checks detect replacements with
different content; they do not prevent replacement.

<!-- /coderabbit-ai-generated:v1 -->

Keep this test-only instrumentation for platform regressions. To remove it, remove
`artifact_diagnostics.rs`, its `cfg(test)` module declaration and the matching
`cfg(test)` reporting statements. Do not remove the filesystem operations or their
error mappings.

## Reusable seam

The library exposes `model_manifest::PinnedMiniLmManifest`,
`artifact_fs::ArtifactDir` and
`artifact_fs::staging::model::VerifiedArtifactCache` on Linux/macOS.
With `cpu-f32`, it also exposes `minilm_tokenizer::MiniLmTokenizer` and
`minilm_encoder::CandidateCpuSentenceEncoder`. The encoder remains a candidate,
not an accepted product provider. `embedding_space` and `minilm_c1_identity`
retain the C1 identity declarations.

The extraction retains artifact staging/recovery because the verified cache
shares those existing modules. It adds no new product-facing adapter.
Numeric code, tests, compiled artifact lock and frozen evidence remain unchanged.
Only crate names and relocated fixture paths differ from the source modules.

## Spike workspace

This package uses the root workspace and root `Cargo.lock`. The original
`dev/muninn/bridge` workspace and its lockfile are intentionally untouched.
Migrate callers to the root commands above before retiring that workspace.
Audit any remaining spike examples, Dockerfiles and validation scripts first;
do not remove their lockfile or workspace declaration while callers still use
`--manifest-path dev/muninn/bridge/Cargo.toml`.
