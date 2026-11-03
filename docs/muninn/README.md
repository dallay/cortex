# Muninn

Muninn is Cortex's persistent memory product. The implementation lives in
`crates/muninn`; `dev/muninn/` holds the reference oracle, capture, and
verification tooling. Functional parity is not claimed.

## Documentation policy

`docs/muninn/` contains maintained official documentation in English. Identifiers,
recorded outputs, and third-party licenses retain their original content.
Evidence, logs, and historical reports belong to the corresponding RPI task,
not to the official documentation tree. RPI tasks remain in Spanish.

Historical receipts are immutable. Resolve their former paths through the
[relocation manifest](../../.agents/rpi/plan/tasks/muninn-documentation-organization/migration-manifest.json)
and [relocation guide](../../.agents/rpi/plan/tasks/muninn-documentation-organization/relocation-map.md).
The artifact lock is a normative build input; it lives with the tooling fixtures
at `dev/muninn/fixtures/artifact-lock.json`. Moving it did not change its bytes,
policy, or acceptance gates.

- [Pinned reference and gaps](reference-lock.md).
- [Proposed architecture decision](adr/0001-neo4j-parity-boundary.md).
- [Reproducible oracle environment](../../dev/muninn/README.md).
- [Generated static inventory](specs/parity-matrix.csv).
- [Initial oracle evidence](../../.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/g0-oracle.json).
- [TCK baseline and adapter mediation](reference-tck.md).
- [MCP discovery by profile and transport](reference-mcp.md).
- [API by backend and artifact references](reference-api-artifacts.md).
- [First Rust Bronze and verified persistence](../../.agents/rpi/plan/tasks/muninn-h1-delivery/rust-spike.md).
- [Source attribution and licenses](NOTICE.md).

## Compatibility status

G0 is **partially implemented**. A real Bolt backend capture ran without
extraction or embeddings. A `delete_message` discrepancy was recorded; the
reference was not changed to conceal it.

The pinned TCK ran against the adapted oracle, not against Muninn: 166 passes,
9 failures, and 14 skips from 189 collected cases. Native deletion produced
1 pass and 12 failures from 13 selected cases.

Verified against the Muninn product today:

- Schema v1 migrations, validated adoption, single-writer coordination, and
  non-destructive retries, on isolated Neo4j.
- Bronze **93/93** on the normative TCK pin.
- GM-01 conversation lifecycle: 61 cases, differential comparison pass, zero
  errors, with authorized deviations `GM01-DEV-001` and `GM01-DEV-002`
  (`gm01-acceptance.md`).
- EXACT search capped at 10,000 candidates with an explicit capacity error and
  a 5-second client budget.
- MiniLM local inference: normalized 384-dimension vectors, batch/single
  consistency.

Not established: C1 provenance from a protected `main` (deferred until
integration), ANN acceptance policy, and a measured global parity percentage.
No Muninn conformance tier beyond Bronze is claimed.

MCP discovery added four runtime registries and eight stdio/HTTP sessions:
6 core tools, 16 extended tools without a backend, and 20 extended offline NAMS
tools. It does not validate NAMS service authentication, permissions, or persistence.

The canonical inventory adds 322 operation/backend rows and 80 separate exports.
Three model references and bundled APOC have provenance receipts and hashes;
no inference or plugin activation is claimed.
