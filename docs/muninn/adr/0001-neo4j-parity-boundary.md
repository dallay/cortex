# ADR 0001: Neo4j parity boundary

Status: **proposed**. This decision does not authorize the permanent product
structure. It records the minimum boundary against which G0 gathers evidence.

## Context

Observable parity depends on Neo4j behavior, embedding models, the API, and MCP.
An in-memory engine does not reproduce transactional behavior, indexes,
projections, or Cypher errors. The test reference uses Neo4j Community 5.26.

## Proposal

- Keep domain and application layers independent of HTTP, MCP, and the driver.
- Define semantic ports for messages, sessions, context, retrieval, graph,
  embeddings, LLMs, and backend capabilities.
- Place Neo4j translation and persistence in an adapter. Do not expose a Cypher
  query or a mutable `Session` as a domain port.
- Declare capabilities by backend and profile. Preserve distinctions between
  `NotSupportedError`, native not-implemented paths, and operational failures.
- Start a functional port with Neo4j 5.26 and an observable native driver.
  **Do not replace the oracle with HashEmbedder** to claim semantic parity.
- Validate bounded units against the frozen oracle before expanding scope.

## Alternatives not accepted in G0

- In-memory storage as parity evidence: it changes graph semantics and errors.
- A library with embedded Python as the product: it does not satisfy native Rust.
- A unified error/capability layer without oracle evidence: it loses contract detail.

## Still undecided

The Rust Neo4j driver, local model/runtime, graph capability policy, publication,
and minimum Rust version remain subject to spikes and explicit approval. G0
creates no public API, crate, or TCK adapter for Muninn.
