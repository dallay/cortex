# Candle core compatibility patch

Origin: published `candle-core 0.11.0`, upstream revision
`31f35b147389700ed2a178ee66a91c3cc25cc80d`.

Archive URL: <https://crates.io/api/v1/crates/candle-core/0.11.0/download>

Archive SHA-256:
`5ecb245093b0f791b89d3420c3df9c6d49c60ab63ba54db896bf8a3baf486706`.

## Local delta

- Normalized `Cargo.toml`: replace ZIP 8.6 with actual pinned ZIP `=9.0.0`.
- Normalized `Cargo.toml`: disable GEMM default features, retaining `std`,
  `rayon` and `wasm-simd128-enable` for this CPU/F32-only prototype. This excludes
  the optional F16 backend that failed the default Linux ARM64 build.
- `src/npy.rs`: propagate ZIP 9's fallible filename decoding and borrow the
  decoded `Cow<str>` when removing the `.npy` suffix.
- `src/pickle.rs`: collect fallible decoded filenames, propagating ZIP errors.

No tensor kernels, BERT/model mathematics, safetensors logic, training operations
or device implementation is changed. The entire source import is checked against
the checksum-verified archive; modified upstream paths are explicitly recorded.
Four deprecation warnings in the unchanged upstream CPU `erf.rs` are retained,
not hidden by lint allowances or edited as part of this patch.

The GEMM feature restriction is not semantically neutral for general Candle:
F16 matrix multiplication is unsupported and can panic in generic dispatch.
The future loader must reject original non-F32 model weights before construction;
it must not cast incompatible checkpoints into this profile. General F16 support,
kernel changes and raised CPU-baseline flags are outside this patch's scope.

The package's published Cargo.lock is intentionally omitted. Cortex retains its
single root lockfile. This directory is excluded from workspace membership and
selected only as a third-party Cargo dependency patch; it is not another workspace.
Cortex's own unsafe-code and clippy policy is unchanged.

## Security and maintenance

ZIP 8.6 triggers RUSTSEC-2026-0367, RUSTSEC-2026-0368 and RUSTSEC-2026-0369;
the user authorized this narrow compatibility patch rather than an audit waiver.
ZIP 9 is the real registry package, not a version-renamed copy. Check audit,
source-delta proof, safe NPZ compatibility and workspace membership whenever this
patch changes. Replace the patch with a reviewed fixed upstream release when one
is suitable; do not broaden the local delta silently.

The original Apache-2.0 license is retained in `LICENSE`. The packages declare
`MIT OR Apache-2.0`; the selected MIT alternative and source attribution are also
preserved under `docs/muninn/licenses/`. This file does not relicense upstream or
transitive dependencies and grants no release/distribution approval.

This compatibility evidence is not proof of MiniLM loading, sentence inference,
numerical equivalence, C1, TCK or full functional parity.
