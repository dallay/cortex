# Local MiniLM in Muninn

## Status and boundaries

The local provider is a CPU/F32 candidate. Its compilation and configuration
tests do not constitute C1 numerical acceptance or H1 closure.
C1 inference does not run without verified exporter provenance.

## Explicit configuration

Build from the workspace root:

```sh
cargo build --locked -p muninn --features minilm-cpu
```

Select `--embedding-provider minilm` and provide:

- `--embedding-bundle-path`: local root of the existing artifact cache.
- `--embedding-bundle-id`: bundle digest pinned by `artifact-lock.json`.

`NAM_EMBEDDING__BUNDLE_PATH` and `NAM_EMBEDDING__BUNDLE_ID` are also supported.
The provider does not download artifacts. It checks the manifest, receipt, and
bundle bytes before loading the encoder. Invalid configuration prevents the
Neo4j connection from opening and requests from being served. The local
provider has 384 dimensions; the schema uses that value.

Synchronous inference uses `spawn_blocking` and one execution permit per
instance. The permit remains inside the blocking job even if the future
waiting for its result is cancelled. Errors expose neither texts nor bundle paths.

## Vector space identity

Each message with a vector retains its identity and dimensions. Search requires
the same identity, together with the owner and requested filters, before sorting
and limiting. Legacy vectors without identity are not mixed with new data.
They are neither automatically migrated nor deleted.

MiniLM pins the bundle and `minilm-cpu-f32-v1` contract. Hash uses a separate
space for the TCK and provides no semantics. OpenAI-compatible providers require
`--embedding-identity` or `NAM_EMBEDDING__IDENTITY`, with an immutable identity
that describes revision and preprocessing. A mutable model label does not
prove space compatibility; the operator must pin that identity.

## Checks without inference

```sh
cargo test --locked -p muninn --features minilm-cpu --bin muninn
cargo test --locked -p muninn-providers --features minilm-cpu
cargo clippy --locked -p muninn -p muninn-providers \
  --all-targets --features minilm-cpu -- -D warnings
```

These checks do not load weights or establish real semantic retrieval.

## Search in Neo4j 5.26

Search first attempts global ANN with `k = limit`. Cypher then applies owner,
optional session, embedding identity and dimensions, optional metadata, and
threshold filters before returning results. If fewer than `limit` rows remain,
it runs an exact scoped query and returns only that query's complete result,
limited to `limit`, without mixing both lists.

This hybrid is not native scoped ANN. If ANN fills `k`, ranking remains
approximate and guarantees neither exact equivalence nor complete recall.
ANN index, dimension, and storage errors propagate without fallback. Predicates
after ANN do not protect the procedure from different query and index dimensions.
Fallback adds a second query and can scan all messages in the scope; it does
not establish bounded performance, C1/MiniLM acceptance, or global H1 closure.
