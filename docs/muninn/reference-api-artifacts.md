# API by backend and model/plugin references

## Scope

This G0 unit materializes a consumable `MemoryClient` inventory and pins artifact
references for a candidate local profile. It neither implements the Rust service
nor demonstrates inference, functional support, or parity.

## Canonical API inventory

- [322 operation/backend rows](specs/api-profile.csv): **161 operation or property
  names**, each represented for Bolt and NAMS.
- [Metadata, hashes, and 80 exports](../../.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/api-profile.json).
- Curated bindings and notes in `dev/muninn/api-bindings.json` and `api-notes.json`.

The inventory includes the `MemoryClient` constructor, asynchronous context,
methods and properties, the `connect` factory, and eleven concrete accessors.
It uses the four main Protocols and the additional ontology contract. It retains
contract signatures separately from implementation signatures.

These are not 322 accepted capabilities. Every row retains
`muninn_status=no_implementado` and `runtime_status=no_verificado`.
`declaracion_con_cuerpo` only means a body exists in the source. `sin_binding`
is not converted to `NotSupportedError` or an inherited method. A member without
a known declaration has `kind=no_declarado`.

The generator does not import upstream: it reads Git objects at the pinned
commit and analyzes AST. It checks configured inheritance before incorporating
`BaseMemory`. It does not attribute that inheritance to NAMS classes. Notes
without a valid operation block generation. Bindings remain a curated, reviewed
map, not an automatic analysis of every assignment flow.

The 80 `__all__` elements are declared exports, not additional operations. They
may be models, exceptions, aliases, or optional integrations. No claim is made
that all are usable without their extras. The factory groups overloads. The
deprecated `client.graph` proxy is recorded as an open proxy: dynamic attribute
forwarding is not invented as a list of explicit methods.

## Artifact provenance

[The artifact lock](../../dev/muninn/fixtures/artifact-lock.json)
contains three distinct Hugging Face identities:

| Use | Repository | Revision | License |
| --- | --- | --- | --- |
| Candidate parity profile | `sentence-transformers/all-MiniLM-L6-v2` | `1110a243fdf4706b3f48f1d95db1a4f5529b4d41` | Apache-2.0 |
| Independent smoke test, not parity | `sentence-transformers/paraphrase-MiniLM-L3-v2` | `4ca70771034acceecb2e72475f72050fcdde4dd8c` | Apache-2.0 |
| External Huginn reference | `Qwen/Qwen3.5-9B` | `c22eb327c15f5424a6658d85fd8e3380327dc9c3` | Apache-2.0 |

Each record has a repository receipt with SHA-256, checkpoint/tokenizer filenames,
sizes and hashes, local license SHA-256, configuration, architecture, and source.
Receipt SHA-256 values anchor committed metadata; verification rejects bytes
that differ from those anchors and checks license SHA-256 values before trusting
their content. License files are retained under `docs/muninn/licenses/`.

**No inference, quality, or numerical equivalence is claimed** by pinning these
identities. Qwen is not the oracle's default LLM. The smoke model is not the
MiniLM L6 parity model. An ONNX variant does not authorize a backend change.

The candidate profile declares `cpu`, `float32`, batch 1, and length 256.
Tokenizer files, special tokens, normalizer, pre-tokenizer, truncation, and
padding remain declared by upstream files; there is no local tensor or inference
execution in this unit. `unverified` flags distinguish upstream declarations
from runtime confirmation. Runtime-dependent parameters such as attention,
position-id, and dense-bias dropout are explicitly left in that state; the lock
does not invent values. The later CPU receipt characterizes them separately.

The verifier requires three exact identities and the complete field inventory,
rejects duplicate names/paths and URIs not derived from the pinned revision,
checks declared receipt size, and preserves `inference_executed=false`. When a
cache is supplied, it streams every artifact in 1 MiB blocks, compares byte count
and SHA-256, and rejects symlinks outside the cache.

## Plugin

The Neo4j image pins **5.26.32** and bundles APOC. A local receipt verifies that
`apoc.version()` returns `5.26.32` in that image; it does not test any other APOC
function. See
[apoc-smoke.json](../../.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/apoc-smoke.json).
APOC is neither copied into `plugins/` nor activated as a plugin.

The local profile declares zero additional plugins. This is a controlled
constraint, not proof that every future upstream function works without plugins.
The receipt retains the actual `/plugins` listing, including the base image's
`README.txt`; it does not confuse that text file with an active plugin.
