<div align="center">

# 🧠 Muninn

Persistent graph memory for AI agents

[![Rust MSRV](https://img.shields.io/badge/dynamic/toml?url=https%3A%2F%2Fraw.githubusercontent.com%2Fdallay%2Fcortex%2Fmain%2FCargo.toml&query=%24.workspace.package%5B%27rust-version%27%5D&label=Rust%20MSRV&suffix=%2B&color=DEA584&logo=rust&labelColor=4A4A4A)](https://www.rust-lang.org)
[![Neo4j](https://img.shields.io/badge/Neo4j-graph%20storage-018BFF?logo=neo4j)](https://neo4j.com)
[![MCP](https://img.shields.io/badge/MCP-integration-7C3AED?logo=modelcontextprotocol)](https://modelcontextprotocol.io)

**A Rust-based persistent memory component in the Cortex AI infrastructure stack.**

[Overview](#overview) · [Status](#implementation-status) · [Quick Start](#quick-start) · [CLI](#command-line) · [Documentation](#documentation)

</div>

---

## Overview

Muninn provides graph-backed memory capabilities for AI agents, using Neo4j for persistence. The repository includes a Rust application, memory libraries, provider adapters, and MCP integration.

Muninn is under active development. Validated behavior covers specific, tested parts of the reference contract; it is not a claim of full compatibility with `neo4j-labs/agent-memory` or production readiness for every feature. Review the current [compatibility status](../../docs/muninn/README.md#compatibility-status) before adopting it.

## Implementation Status

The current evidence includes validated schema migrations and coordination, Bronze coverage of 93/93 cases on the pinned normative TCK, a tested conversation lifecycle, bounded exact search, and local MiniLM inference. These results apply to the documented test scope; they do not establish an overall parity percentage or conformance beyond Bronze.

Some areas remain incomplete or unverified, including full reference parity, ANN acceptance, and production authorization and persistence guarantees for every MCP profile. See [Muninn's status and limitations](../../docs/muninn/README.md#compatibility-status) and the [pinned reference gaps](../../docs/muninn/reference-lock.md) for current evidence.

## Quick Start

### Requirements

- The Rust toolchain declared by the repository's [`rust-toolchain.toml`](../../rust-toolchain.toml).
- A reachable Neo4j instance for commands that access the graph.

### Build and inspect the CLI

Run these commands from the repository root:

```bash
cargo build --release -p muninn
cargo run --release -p muninn -- --help
```

### Configure Neo4j and check connectivity

Muninn reads Neo4j settings from command-line options or the `NAM_NEO4J__*` environment variables. Provide the password through a local secret manager or another secure method before running these commands. Do not commit credentials or place them in shared command history.

```bash
export NAM_NEO4J__URI=bolt://127.0.0.1:7687
export NAM_NEO4J__USERNAME=neo4j

cargo run --release -p muninn -- doctor
```

`doctor` checks the configured environment. Use `schema` to create or verify the graph constraints and indexes when appropriate:

```bash
cargo run --release -p muninn -- schema
```

## Command-Line Interface

| Command | Purpose |
| --- | --- |
| `doctor` | Check the environment and graph connection. |
| `schema` | Create or verify graph constraints and indexes. |
| `stats` | Show graph statistics and memory metrics. |
| `ontology list` / `ontology show` | Inspect built-in ontology definitions. |
| `ontology validate <path>` | Validate an ontology document. |
| `mcp --owner-scope <scope>` | Serve MCP tools over stdio by default. |
| `bridge` | Serve the agent-memory TCK HTTP bridge for compatibility testing. |

The MCP `owner-scope` selects a data scope; it is **not authentication**. The MCP HTTP transport binds to loopback by default. Do not expose it on a network without an appropriate access-control boundary. The `bridge` command is for isolated compatibility testing. Its destructive clear option is disabled by default and must only be considered with disposable test data.

## Architecture

`apps/muninn` is the CLI composition root. The `crates/muninn` workspace contains the core contracts, application services, Neo4j adapter, providers, and MCP implementation. `dev/muninn` contains reference-oracle and verification tooling; it is not a production Muninn service.

## Documentation

- [Muninn documentation index](../../docs/muninn/README.md) — compatibility status, evidence, and technical references.
- [Architecture](../../docs/muninn/architecture.md) — crate structure, memory tiers, and integrations.
- [Pinned reference and gaps](../../docs/muninn/reference-lock.md) — compatibility evidence and unverified areas.
- [MCP implementation notes](../../crates/muninn/mcp/README.md) — transport, protocol boundaries, and supported behavior.
- [Reference test environment](../../dev/muninn/README.md) — isolated oracle capture and verification workflow.
- [Interactive architecture diagram](../../docs/architecture/muninn-architecture.html).
- [Cortex overview](../../README.md) — the full product suite and development workflow.

## Development

```bash
cargo test --locked -p muninn
cargo clippy --locked -p muninn --all-targets -- -D warnings
```
