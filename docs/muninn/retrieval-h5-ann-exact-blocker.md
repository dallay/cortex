# H5 ANN versus exhaustive retrieval: diagnostic path

## Scope

The H5 diagnostic compares ANN and exhaustive retrieval for the approved disposable pilot. It does
not change the normal bridge contract, ANN ranking, corpus, annotations, or acceptance status.

## Implementation facts

- `h5-diagnostic` is an opt-in Cargo feature. The default bridge build does not compile or register
  the diagnostic route.
- The authenticated `/h5/diagnostic/search` route exists only in the feature build. The H5 runner
  reaches it over the private Compose network. The Compose file publishes no host ports.
- The diagnostic handler embeds a query once with the same `EmbeddingPort` as normal ANN search.
  It passes that exact embedding, normalized limit and threshold, and the same optional session to
  `Store::search_messages` and `Store::search_messages_exact`. Exact search receives the encoder's
  `space_id`. Both typed Store methods validate the Store's token-derived owner guard. This owner
  is not a caller-supplied `user_identifier`.
- The response contains only validated corpus aliases, scores, and ranks. It contains no UUIDs,
  text, metadata, or vectors. The bridge maps internal IDs to the submitted synthetic aliases
  before serialization. The runner writes only aliases, scores, and ranks. It does not log IDs or
  credentials.
- The diagnostic output is descriptive only. It does not establish parity or acceptance.

## Verification

The feature-gated projection test verifies that serialized search results contain only `alias`,
`rank`, and `score`, and checks the internal UUID is absent. Handler tests verify auth and alias
validation. Runner tests verify alias correlation and reject unknown or duplicate aliases.
The Store has an ignored Neo4j integration test for owner-guarded, session-scoped exact retrieval
against ANN on a deterministic dataset. The test was not run separately; the feature-gated H5 pilot
did run against the disposable Neo4j service. Unit tests also verify that both statements carry the
Store owner guard and that exact search applies its session filter before scoring. The pilot result
is descriptive and does not establish a score tolerance or acceptance threshold; see
`retrieval-h5-local-results.md`.

The requested feature build succeeded locally on macOS with Cargo offline. An initial Linux-container
build failed because the container registry did not have a verified `linux-raw-sys 0.12.1` package.
After the lockfile fetch recorded in `retrieval-h5-diagnostics.md`, the offline Linux ARM64 build
succeeded using the verified host registry mounted read-only and a dedicated named target volume.
The resulting feature binary and distinct runtime image are recorded in that document. The separate
H5 service run and its teardown are recorded in `retrieval-h5-local-results.md`. A live comparison
was completed; it does not establish numerical acceptance.

Run focused checks from the repository root:

```sh
cargo test --offline --locked --manifest-path dev/muninn/bridge/Cargo.toml --lib
cargo test --offline --locked --manifest-path dev/muninn/bridge/Cargo.toml --features h5-diagnostic --lib
PYTHONPATH=dev/muninn python3 -m unittest dev/muninn/tests/test_retrieval_h5_runner.py
```

Build a distinct diagnostic image only when its pinned base images are already local. Use
`CARGO_NET_OFFLINE=true`, `--network=none`, `--pull=false`,
`--build-arg MUNINN_BRIDGE_FEATURES=h5-diagnostic`, and a new local tag. Do not replace the prior
image digest. If the build cannot complete offline, stop before creating or starting H5 containers.
