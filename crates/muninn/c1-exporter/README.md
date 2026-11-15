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
operations. These diagnostics do not establish the cause of the macOS 15 failure.

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
