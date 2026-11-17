# Strict MiniLM checkpoint contract

`minilm_checkpoint::validate` performs read-only contract validation through an
existing `VerifiedArtifactCache`. It does not construct BERT, run inference or
accept a numerical equivalence policy.

## Configuration

The validator reads a freshly verified `config.json` buffer. A bounded typed
Serde profile requires all twenty fields, rejects unknown and duplicate fields,
and checks the fixed MiniLM architecture and parameters. No defaults or optional
omissions authorize another profile. An explicit Candle configuration selects
GELU-ERF, absolute positions and no classifier dropout; this is configuration
mapping, not proof of model execution in evaluation mode.

## Tensor state

The validator reads a freshly verified owned weight buffer. Before parsing, the
buffer is bounded to 128 MiB and the eight-byte little-endian header length to
1 MiB, using checked conversion/arithmetic. Safetensors validates the format,
offsets and payload size; this code does not replace its format parser.

The exact unprefixed inventory contains 104 entries:

- 103 original F32 tensors with fixed expected shapes and finite values.
- One I64 `embeddings.position_ids` buffer, shape `[1, 512]`, values `0..511`.

Specifications come from the fixed validated profile, not untrusted input
dimensions. Every expected name is looked up and checked. Other names, missing
entries, namespace aliases, original dtypes, shapes and invalid values reject
before any Candle conversion or model constructor.

The count includes two pooler tensors and the stored position buffer that Candle's
reviewed BERT constructor does not consume. It is not a claim that BERT uses all
104 entries. General F16 GEMM is unsupported by the selected dependency profile;
future construction must retain these original-dtype checks rather than cast an
incompatible state into F32.

## Evidence boundaries

Routine tests use small labeled synthetic views for negative cases. The actual
checkpoint test is ignored by default and counts only when explicitly executed:

```sh
MUNINN_MINILM_ARTIFACT_ROOT=/physical/absolute/private-root \
  cargo test -p muninn-bridge-spike --lib --locked --offline \
  minilm_checkpoint::tests::actual_cache_contract -- --ignored --nocapture
```

The test opens the known bundle against the compiled pinned manifest and verifies
all required artifacts before contract checks. It performs no download, model
construction, permission change or cache repair. Linux unit fixtures do not imply
that the private real macOS cache was mounted or validated in Linux.

The returned report is data, not a reusable readiness capability or a permanent
integrity lease. A prior report cannot authorize unchecked paths or new buffers.

## Raw CPU checkpoint construction

`CpuCheckpoint::load(&cache)` obtains a private `ValidatedCheckpoint` containing
the explicit configuration, checked metadata and owned verified weight bytes.
After every contract check succeeds, it consumes that exact vector through
`VarBuilder::from_buffered_safetensors` on CPU/F32 and calls `BertModel::load`.
There is no weight reread, path reopening, mmap, caller-selected precision,
remote model code or fallback checkpoint between validation and construction.
Candle subsequently materializes its own tensor storage; the vector handoff does
not mean all runtime tensor allocations share the original weight allocation.

The raw model and report have read-only getters. The model getter exposes Candle's
forward capability to future adapter work, not a complete or numerically accepted
sentence encoder. Runtime errors return only a static variant, without model
payloads, underlying error strings or filesystem paths.

Real construction has a separate opt-in test:

```sh
MUNINN_MINILM_ARTIFACT_ROOT=/physical/absolute/private-root \
  cargo test -p muninn-bridge-spike --lib --locked --offline \
  minilm_checkpoint::tests::actual_cache_constructs_cpu_bert -- --ignored --nocapture
```

This test must be explicitly executed before claiming actual construction. It
does not call `forward`. Routine ignored tests do not prove construction or count
as passes. These synchronous offline calls do not provide service admission,
concurrent-load memory quotas or preemptible deadlines. Callers must not infer
those operational guarantees from the artifact-size limit or the raw model type.

Tokenization, attention masks, dropout execution, masked pooling, L2, batch
invariance, C0/C1, TCK and global parity remain separate work.
