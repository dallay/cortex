# Candle dependency notice

This records the user-authorized local compatibility patch for Candle from
<https://github.com/huggingface/candle>.
The `candle-core`, `candle-nn` and `candle-transformers` version is
0.11.0, whose published archives identify upstream revision
`31f35b147389700ed2a178ee66a91c3cc25cc80d`.

Local core selects actual ZIP 9.0.0 and propagates its fallible filename API in
archive readers. All model-math sources remain identical to the verified archive.
Its manifest also excludes GEMM's optional F16 backend for the CPU/F32-only
prototype; general Candle F16 matrix multiplication is not supported.
See `third_party/candle-core/CORTEX-PATCH.md` and its reproducible patch file.

The packages declare `MIT OR Apache-2.0`. The MIT option is selected; its
unaltered upstream text is preserved in `candle-0.11.0-MIT.txt`, copied from the
immutable repository revision. The nn/transformers archives omit the license
text; the core archive contains the Apache-2.0 alternative. Preserve this notice
and applicable upstream notices when distributing the software.

This notice does not relicense safetensors or any native/transitive dependency.
The resolved graph's license/build/feature review and audit evidence are separate.
No independent product release or redistribution approval is implied.
