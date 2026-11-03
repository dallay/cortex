# Exporting the MiniLM C0 text reference

`dev/muninn/export_minilm_c0.py` exports CPU embeddings and scores using
the **exact texts in the frozen corpus**. Tokenization belongs to the Python
reference; this program does not consume C1 token tensors.

The reference forces CPU/F32 and eager attention for `BertModel`. It disables
SDPA/flash, CuDNN, TF32, reduced-precision reductions, and parallel execution
outside the declaration of one intra-op thread and one inter-op thread.
`--help` does not load the model.

## Input

A JSON file declares:

- `fixture_schema`: `minilm-c0-text-reference-v1`.
- `cases`: each case contains `case_id` and exactly one of `text` (string) or
  `texts` (nonempty list of strings).
- `score_pairs`: optional; each element contains `pair_id`, `left`, and `right`.
  Each reference contains `case_id` and an optional `vector_index` (default: 0).

The exporter does not split text, trim whitespace, or derive cases. It preserves
text content and order and records the SHA-256 of the input bytes.

## Execution

```sh
python dev/muninn/export_minilm_c0.py \
  --artifact-cache /ruta/al/cache \
  --input /ruta/al/text-reference-input.json \
  --output /ruta/al/python-reference-output.json
```

The local cache must already contain the verified checkpoint and bundle.
Execution uses neither the network nor GPU. The checkpoint is loaded on CPU,
cast to F32, and each selected parameter is checked before execution.

## Output

The JSON output uses `execution_state=EXECUTION_RECORDED` and contains:

- `consumed_input_sha256` and `consumed_input_bytes`;
- bundle identity, checkpoint SHA-256, engine, dtype, and thread configuration;
- embedding and embedding hash for each case;
- cosine similarity and normalized `score` for each declared pair.

This receipt records an execution. It does not declare `PASS`, C1 acceptance,
or H1 closure. C1 runs through its separate numerical path with frozen tokens.
