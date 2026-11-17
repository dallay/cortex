# Offline H5 numerical diagnostics

`dev/muninn/retrieval_h5_diagnostics.py` prepares singleton inputs for the existing
C1 exporters and compares supplied Rust and Python exports. It uses only the
Python standard library and existing local validation modules. It does not load
models, contact services, access databases, change rankings, or edit judgments.

## Prepare an input

Run from the repository root. Output parent directories must already exist.
All output paths must be new files. The source corpus remains unchanged.

```sh
python3 dev/muninn/retrieval_h5_diagnostics.py prepare \
  --corpus dev/muninn/fixtures/retrieval-pilot-v1-corpus.json \
  --derived tmp/muninn/h5-singletons.json \
  --manifest tmp/muninn/h5-singletons-manifest.json
```

The derived corpus uses a `c1_h5_*` identifier and neutral `item_NNN` batch IDs.
Each query and each query-local candidate gets one singleton batch. The current
pilot produces 18 batches. Repeated candidates across queries remain separate
occurrences. The original candidate IDs are retained only in the manifest mapping.
The manifest contains text hashes, not text. The derived corpus necessarily
contains the original text: handle that file as sensitive input.

Preparation enforces the existing exporter limits: at most 32 batches, 4096 UTF-8
bytes per text, and 256 KiB for the derived corpus. No exporter is executed.
Any later export execution requires separate authorization and available local
artifacts. Use the same explicit numerical contract identifier for both exports;
a contract declaration does not establish numerical acceptance.

## Compare supplied exports

```sh
python3 dev/muninn/retrieval_h5_diagnostics.py compare \
  --corpus dev/muninn/fixtures/retrieval-pilot-v1-corpus.json \
  --derived tmp/muninn/h5-singletons.json \
  --manifest tmp/muninn/h5-singletons-manifest.json \
  --rust tmp/muninn/h5-rust-export.json \
  --python tmp/muninn/h5-python-export.json \
  --historical tmp/muninn/retrieval-h5-20261107b.json \
  --output tmp/muninn/h5-numerical-diagnostics.json
```

`--historical` is optional. The comparator validates the corpus, exact derived
bytes, manifest mapping and hashes, pinned C1 identity declarations, matching
embedding spaces, execution labels, unique batch IDs, complete coverage, singleton
vector counts, 384 dimensions, finite nonzero vector norms, and token matrix
shapes and values before arithmetic. JSON inputs reject duplicate keys, NaN,
Infinity, and files larger than 8 MiB. Incompatible inputs produce no report.
A historical report must identify the same corpus and cover every query and
query-local candidate exactly once.

The report records:

- SHA-256 hashes of every input file, computed during this comparison.
- Supplied runtime evidence status and a receipt hash where present. Runtime
  receipts are not independently measured or authenticated by this tool.
- Maximum absolute component error, embedding cosine, both norms, absolute norm
  difference, and each vector's deviation from unit norm.
- Exhaustive raw cosine scores against each query's candidate set for each
  runtime, descending order, adjacent score margins, and exact tie groups.
- Historical orders and score observations, when supplied, plus exact order
  equality against each replay. This is not a quality judgment.

Exact ties use floating-point equality, with corpus order used only for display.
No numerical tolerance or near-tie acceptance rule is invented. The report omits
text, token arrays, vector arrays, and runtime receipt payloads. Original query
and candidate identifiers remain in ranking results. Keep input IDs nonsensitive.
Hashes identify bytes; they do not prove source-to-binary correspondence.

## Interpretation limits

This tool measures arithmetic on supplied exports. A replay is **not** a capture
of the vectors that produced historical service scores. Matching export hashes
and identity declarations cannot authenticate exporter execution, original texts
inside an external runtime, or the historical bridge binary.

Neo4j score comparison remains `blocked/unverified-scale`. The tool never
subtracts raw cosine from historical scores, assumes a transformation, declares
a global PASS, or applies the frozen D1 encoder policy to Neo4j scores. Numerical
acceptance, service-vector capture, model changes, and ranking changes are outside
this diagnostic's scope.

## Bridge diagnostic route

The experimental `h5-diagnostic` bridge feature exposes
`POST /h5/diagnostic/search` only in feature-enabled builds. The route requires
the normal bearer-token authentication, a non-empty `session_id`, and a bounded
`candidate_aliases` object that maps each internal message UUID to its corpus
candidate alias (for example, `candidate-01`). Alias values must match
`candidate-NN`, be unique, and contain at most 10,000 entries. ANN and exact
search remain scoped by the bridge owner and the requested session. A result is
rejected if its internal ID has no alias in that query's map.

The response contains only `alias`, `score`, and `rank` for each hit. It does not
contain internal UUIDs, message text, embeddings, metadata, or credentials. The
temporary mapping is query-local and must contain only that query's corpus
candidates. The bridge does not log request bodies or diagnostic result rows.
The route does not change ranking behavior or use caller-supplied Cypher.

Exit code zero means the requested file was written, not that numerical parity
or retrieval quality was accepted. Existing outputs are never overwritten.
Preparation writes two separate files; an I/O failure during the second write can
leave the first file in place. Choose new output paths before retrying.

## Local tests

Tests use synthetic vectors and temporary files. They do not construct a model.

```sh
PYTHONPATH=dev/muninn python3 -m unittest discover \
  -s dev/muninn/tests -p 'test_retrieval_h5_diagnostics.py'
```

## Container build status

On 2026-11-07, the local Rust builder image
`rust@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247`
and Python runtime image
`python@sha256:76baad6037690d6e8c4397fbcaef17bca6f19eecef822b708fe8fa3417d30bb5`
were verified with `docker image inspect`. The build was attempted with networking
disabled, the checkout mounted read-only, only `$HOME/.cargo/registry` mounted
read-only, and a container-ephemeral target directory. The first attempt hit the
repository's `rust-toolchain.toml` channel `1.99`; setting
`RUSTUP_TOOLCHAIN=1.99.1-aarch64-unknown-linux-gnu` selected the toolchain already
installed in the pinned builder image, without network access.

The authorized standalone bridge-lock fetch downloaded only `openssl-probe 0.1.6`
(`d05e27ee213611ffe7d6348b942e8f942b37114c00cc03cec254295a4a17852e`) and
`linux-raw-sys 0.12.1`
(`32a66949e030da00e8c7d4434b251670a91556f4144941d37452769c25d58a53`). The
SHA-256 of each cached `.crate` file matched its `Cargo.lock` checksum. The bridge
lock SHA-256 remained
`015c8c2036f5d79b8d7a41165e68f07e641287539bb6f6f9cf16dbcd2355e0a0`.
No further network access was used.

The first offline Linux ARM64 build attempt on macOS stopped because
`aarch64-linux-gnu-gcc` was unavailable. A retry inside the locally available,
pinned Rust builder image used `--network none`, a read-only checkout, the host
`$HOME/.cargo/registry` mounted read-only, `CARGO_NET_OFFLINE=true`,
`--offline --locked`, and an ephemeral target directory. Cargo compiled locked
dependencies but failed with `No space left on device`. A second attempt with
debug info disabled failed while creating a Cargo fingerprint for the same
storage-capacity reason. No missing locked package was reported in these Docker
attempts.

The pinned Rust builder and Python runtime base digests were present locally and
were inspected. A follow-up isolated the output location as the experiment
variable. The build used a new named Docker volume,
`muninn-h5-target-20261108-e7c4a91d`, after confirming that volume did not exist.
The source checkout was mounted read-only at `/src`, the host Cargo registry was
mounted read-only, and Docker networking was disabled. The container used the
local ARM64 Rust builder digest above, `RUSTUP_TOOLCHAIN=1.99.1-aarch64-unknown-linux-gnu`,
`CARGO_NET_OFFLINE=true`, `CARGO_PROFILE_DEV_DEBUG=0`, and
`CARGO_BUILD_JOBS=2`. `df -h /target` reported a 125 GiB filesystem with 62 GiB
available. The locked offline command compiled `muninn-bridge-spike` with
`h5-diagnostic` successfully in 44.12 seconds. This distinguishes the prior
container-ephemeral target failure from the named-volume run; it does not establish
the underlying Docker Desktop filesystem cause.

The resulting bridge binary SHA-256 is
`a7716eb271093112907c8c608ba86526760b1f10f237955c4dfb165424161c05`. The feature
declaration and feature-gated route registration were checked in source. A new
ARM64 runtime image was built from the already-local Python digest, with that
binary and the three existing Candle license/notice files:
`muninn-h5-diagnostic:local-20261108-e7c4a91d`, image ID
`sha256:3d21785ca096ecebeb75b077268b092898f8e82d1899790dd62c532bb2f00111`.
Image inspection confirmed `linux/arm64`, user `7474:7474`, and entrypoint
`/usr/local/bin/muninn-bridge-spike`; an in-image check confirmed the binary hash,
executable mode, and license files. The base image has no named `7474` account;
the configured numeric UID/GID matches the existing bridge Dockerfile contract.
No H5 service or bridge process was started, no credentials or active H5 resources
were used during the build, and no existing image tag or unrelated volume was
removed. The dedicated build volume remains present.

The separate local H5 validation ran on 2026-11-08 under the already-authorized
disposable project. Results, quality metrics, report SHA-256, and provenance limits
are recorded in `retrieval-h5-local-results.md`. After teardown, a read-only Docker
inventory found zero containers, volumes, and networks with the exact project label
`com.docker.compose.project=muninn-h5-pilot-v1`; the temporary credential directory
was absent. The image, model bundle, report, and dedicated build volume remain.
The run is descriptive and does not establish numerical acceptance.
