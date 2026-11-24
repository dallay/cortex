# Muninn Architecture

Technical deep-dive into Muninn's internal design and component interactions.

For a visual overview of how Muninn fits within the full Cortex monorepo, see the
[interactive architecture diagrams](../architecture/). The [Muninn internals diagram](../architecture/muninn-architecture.html)
zooms into the memory tiers, extraction pipeline, embeddings, and Neo4j storage covered below.

## Crate Structure

Muninn follows a layered architecture with clear separation between domain contracts, application logic, infrastructure adapters, and the MCP bridge:

```
crates/muninn/
├── core/                    # Domain layer — memory contracts only
│   ├── src/lib.rs           # Memory tier traits and models
│   └── ...                  # Short-term, long-term, reasoning memory interfaces
│
├── application/             # Application layer — memory orchestration
│   ├── src/lib.rs           # MemoryService, consolidation, retrieval
│   └── ...                  # Use cases for memory operations
│
├── bridge/                  # Bridge layer — external integrations
│   ├── src/lib.rs           # Bridge adapters for cross-component communication
│   └── ...
│
├── mcp/                     # MCP server — Model Context Protocol
│   ├── src/lib.rs           # MCP tool definitions and handlers
│   └── ...                  # Memory tools exposed via MCP
│
├── neo4j/                   # Infrastructure — Neo4j graph database
│   ├── src/lib.rs           # Bolt client, Cypher queries, schema migrations
│   └── ...                  # Graph persistence layer
│
├── ontology/                # Domain — entity ontology and schemas
│   ├── src/lib.rs           # Entity types, relationships, properties
│   └── ...                  # Schema definitions and validation
│
├── providers/               # Infrastructure — embedding and extraction providers
│   ├── src/lib.rs           # Provider traits and registry
│   └── ...                  # OpenAI, MiniLM, Hash embedding backends
│
├── c1-exporter/             # Infrastructure — C1 provenance exporter
│   └── src/lib.rs           # Export utilities for compliance
│
└── observability/           # Infrastructure — metrics and tracing
    └── src/lib.rs           # Health checks, metrics, tracing spans

apps/muninn/                 # Binary crate (standalone server entry point)
```

## Memory Tiers

Muninn organizes memory into three tiers, each with distinct retention, retrieval, and persistence characteristics:

### Short-Term Memory (STM)

Holds recent conversation messages and context. Provides fast access to the current session's dialogue history with automatic expiration and summarization.

- **Storage**: In-memory with Neo4j persistence
- **Retrieval**: Chronological and contextual
- **Lifecycle**: Messages are consolidated into long-term memory and summarized over time

### Long-Term Memory (LTM)

Stores extracted entities, facts, and relationships derived from conversations. This is the persistent knowledge graph that survives across sessions.

- **Storage**: Neo4j graph database
- **Retrieval**: Semantic search via embeddings, exact match, and graph traversal
- **Lifecycle**: Entities are deduplicated, resolved, and enriched over time

### Reasoning Memory

Captures traces, tool calls, and plans from agent executions. Provides a record of the agent's reasoning process for debugging and learning.

- **Storage**: Neo4j graph database
- **Retrieval**: Query by session, tool, or outcome
- **Lifecycle**: Retained for analysis and pattern recognition

## Extraction Pipeline

Entity extraction transforms raw conversation text into structured knowledge:

1. **Input**: Raw messages from short-term memory
2. **NER (Named Entity Recognition)**: Identifies entities using Spacy (local NLP) or LLM-based extraction
3. **Classification**: Categorizes entities by type (person, organization, location, concept, etc.)
4. **Relationship Extraction**: Identifies relationships between extracted entities
5. **Output**: Structured entities and relationships for long-term storage

## Embedding System

Semantic search is powered by vector embeddings with multiple backend options:

| Backend | Type            | Dimensions | Use Case                          |
|---------|-----------------|------------|-----------------------------------|
| OpenAI  | Cloud API       | 1536       | Production semantic search        |
| MiniLM  | Local inference | 384        | Offline and privacy-sensitive use |
| Hash    | Deterministic   | N/A        | Fallback and testing              |

Embeddings are generated for entity text and stored alongside graph nodes for ANN (Approximate Nearest Neighbor) and EXACT search operations.

## Neo4j Graph Schema

The graph database stores all persistent memory data using a Cypher-based schema:

- **Nodes**: Entities with type-specific properties (labels, metadata, embeddings)
- **Relationships**: Typed connections between entities (KNOWS, PART_OF, RELATED_TO, etc.)
- **Indexes**: Vector indexes for ANN search, B-tree indexes for exact lookups
- **Migrations**: Schema versioning with non-destructive retry logic

## MCP Integration

Muninn exposes its memory capabilities through the Model Context Protocol (MCP):

- **Tools**: Memory read, write, search, and consolidation operations
- **Transport**: stdio and HTTP sessions
- **Discovery**: Runtime registries with profile-based tool exposure
- **Core tools**: 6 essential memory operations
- **Extended tools**: 16+ advanced operations (extraction, enrichment, etc.)

## Consolidation

Automatic consolidation maintains memory quality over time:

- **Deduplication**: Identifies and merges duplicate entities
- **Summarization**: Compresses conversation history into concise summaries
- **Entity Resolution**: Links entities across sessions using name normalization and external data
- **Enrichment**: Enhances entities with geocoding, Wikidata, and other external sources

## Health and Observability

- **Health checks**: Bolt connectivity, schema validation, embedding backend status
- **Metrics**: Memory operations count, latency percentiles, cache hit rates
- **Tracing**: Span-based tracing for extraction, embedding, and retrieval pipelines
