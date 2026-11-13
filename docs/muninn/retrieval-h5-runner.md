# Muninn H5 retrieval pilot runner

The one-shot harness in `dev/muninn/retrieval_h5_runner.py` calls the authenticated product
bridge routes `add_message` and, in the H5 diagnostic image only, `h5/diagnostic/search`. The product bridge runs the pinned local
MiniLM bundle and uses its existing Neo4j vector index. The harness does not call setup,
teardown, clear, or delete routes. Reports are descriptive (`measured_not_accepted`) and do not
establish a passing threshold or general hardware support.

The diagnostic image is built with the explicit `h5-diagnostic` Cargo feature. The default build
does not compile or register this route. The feature-gated route uses the same encoder, normalized
query, token-derived bridge owner, query-specific session, limit, and threshold for ANN and
`Store::search_messages_exact`. The exact search also receives that encoder's embedding-space ID.
Both calls use typed `Store` methods. The route returns only internal message IDs, scores, and ranks.
The route requires bridge bearer authentication and is reachable only from the internal H5 Compose
network; the H5 Compose file publishes no ports. The one-shot runner correlates internal IDs to
corpus aliases and drops the IDs before writing the report. The report includes no text, vectors,
internal IDs, owner identifiers, or credentials. The diagnostic remains descriptive and does not
change normal ANN ranking or establish acceptance.

## Safe local run prerequisites

Do not start this run until every item below is verified locally:

1. Obtain the completed annotation CSV from its approved local location. Do not edit or copy its
   completed judgments. Compute its raw SHA-256 with `shasum -a 256 <csv>`.
2. Verify the exact corpus and CSV with the existing validators. Supply the immutable local model
    bundle at `.cache/minilm-artifacts/ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1`.
3. Provision dedicated, ephemeral Neo4j auth and bridge token files under ignored
    `tmp/muninn/h5-secrets/`. Keep that directory mode `0700` and both files mode `0600`. The
    network-disabled staging container receives only those two read-only file mounts and uses
    `DAC_OVERRIDE` to read their host-owned contents before copying them into the project volume.
    Do not reuse `dev/muninn/.cache/neo4j.auth`,
    `dev/muninn/.cache/bridge.token`, or any shared/pre-existing credentials. Set
    `MUNINN_H5_NEO4J_AUTH_FILE` and `MUNINN_H5_BRIDGE_TOKEN_FILE` to those files' paths. This task
    does not generate credentials. Do not proceed if the dedicated files are unavailable.
4. Build the diagnostic bridge image separately only under the approved isolated build procedure.
     Use `--build-arg MUNINN_BRIDGE_FEATURES=h5-diagnostic` and a new local image tag. This Compose
    file does not build or pull images. Set `MUNINN_H5_BRIDGE_IMAGE` to a locally present immutable
   digest reference; verify it with `docker image inspect`. Set `MUNINN_H5_MODEL_BUNDLE` to the exact
    existing local bundle directory. The one-shot `artifacts` service copies this read-only bind mount
     without preserving host ownership or modes, sets directory modes to 0500 and file modes to 0400,
     then changes ownership to uid/gid 7474 in a project-scoped named volume. The bridge mounts only that
    staged volume read-only. No host-owned bundle is mounted directly into the bridge. The
    network-disabled `credentials` service copies the bytes from the two dedicated host credential
    files into a separate project-scoped named volume, changes ownership to uid/gid 7474, and sets
    both files to mode 0400. Neo4j and the bridge wait for that service and mount the credential
    volume read-only at `/run/secrets`. Compose file-backed secret mounts are not used. Never set a database URI; the bridge connects to its fixed
   `neo4j:7687` service.
5. Verify `dev/muninn/compose.h5.yaml` resolves to project `muninn-h5-pilot-v1`, that its only
    writable database storage is `/data` tmpfs, that the network is internal, and that no service
    publishes a host port. The runner uses the bridge service alias only on the internal network.
    If host resource paths cannot be proven disposable, stop.
6. Start only the named project with `docker compose -p muninn-h5-pilot-v1 -f
   dev/muninn/compose.h5.yaml up -d --pull never`. Do not use an existing Muninn/Neo4j project.
7. Set `MUNINN_H5_RUNNER_FILE` to `dev/muninn/retrieval_h5_runner.py`,
    `MUNINN_H5_QUALITY_MODULE_FILE` to `dev/muninn/retrieval_quality.py`, and
    `MUNINN_H5_CORPUS_FILE` to the exact pilot corpus,
    and `MUNINN_H5_ANNOTATIONS_FILE` to the completed annotation CSV. Set the raw CSV hash,
    verified bridge image identity, and source/build identity using the required Compose variables.
    The runner reads the token from the staged read-only credentials volume at
    `/run/secrets/bridge_token`; do not pass the token in environment variables or Compose config.
    The runner accepts `http://bridge:8080` only when the exact H5 disposable, project, and internal
    runner markers are present. The supplied bridge and source/build identities are caller-supplied
    inputs; the runner does not inspect Docker or independently verify them. The report marks the
    bridge identity `caller_supplied_not_verified`. Verify the bridge container's image digest
    locally before the run. The runner records a SHA-256 over its current runner and metrics source
    files. It raises the ANN search limit to at least the total corpus candidate count because ANN
    selects globally before session filtering. It refuses candidate sets above the bridge maximum of
    100. Metric cutoffs remain K={1,3,5}.
9. Use a unique runner container name. After the pilot bridge and prerequisites are already running,
    initialize the output volume explicitly, then create the one-shot runner with `--no-deps`.
    This prevents Compose from rerunning completed artifact and credential staging jobs. The
    evaluation profile's `runner-output-init` service has no network and mounts only the named
    output volume. The actual bridge and Neo4j services must already be up. Replace `<unique-id>`
    with a fresh identifier and use the same value for both commands:

   ```sh
    docker compose -p muninn-h5-pilot-v1 -f dev/muninn/compose.h5.yaml --profile evaluation \
      run --no-deps runner-output-init
    runner="muninn-h5-pilot-v1-runner-<unique-id>"
    docker compose -p muninn-h5-pilot-v1 -f dev/muninn/compose.h5.yaml --profile evaluation \
      run -d --no-deps --name "$runner" runner
   status="$(docker wait "$runner")"
   test "$status" = 0 || { echo "runner exited with status $status" >&2; exit 1; }
   mkdir -p tmp/muninn
   docker cp "$runner:/out/report.json" "tmp/muninn/retrieval-h5-<unique-id>.json"
   docker rm "$runner"
   ```

   Keep the copied report in `tmp/muninn/` until review. Do not remove the project output volume
   before the copy. The report records its runner source digest, so dirty source is not
   misrepresented by a Git commit ID.
10. Review the JSON for expected provenance and absence of query/candidate text, internal IDs,
    owner identifiers, and credentials. The report includes
   per-query and macro Recall@K, MRR, and nDCG@K, raw CSV SHA-256, rubric/reviewer/adjudication,
   corpus hash, model and source/build identities with their verification status, search
    configuration, and host/runtime facts. Its H5 diagnostic section records both ANN and exact
    alias/score/rank lists for each query. It emits no query or candidate text or embeddings.

The runner rejects all non-loopback endpoints except the exact internal bridge alias in marked
runner mode, absent H5 markers, and non-H5 loopback ports. It disables proxy-derived transport
configuration. It does not inspect or control Docker, and it does not verify
the supplied Docker image identity. The caller must verify image identity before the run. A missing
ability to prove host resource paths or image identity is a preflight failure, not permission to
proceed.

## Teardown

After review and when authorized, discard only the disposable project by running
`docker compose -p muninn-h5-pilot-v1 -f dev/muninn/compose.h5.yaml down --volumes --remove-orphans`.
    This removes only the explicitly named pilot's containers and its `artifacts`, `credentials`,
    and `runner-output` volumes. Never use a database reset or teardown API, and never target another
    Compose project.
