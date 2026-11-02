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
