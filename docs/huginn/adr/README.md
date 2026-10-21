# Huginn architecture decision records

These records capture decisions accepted during the Huginn MVP planning on
2026-10-06 and the product-naming decision accepted on 2026-10-10.
Accepted means the direction was chosen; it does not mean the agent is
implemented or its behavior has been verified.

[ADR-0011](0011-huginn-product-naming.md) sets the product's official
name: **Huginn** (the autonomous coding agent) and reserves **Muninn**
for the future persistent memory subsystem. The previous "provisional
internal identifier" wording in earlier ADRs is preserved on those
records so the historical decisions remain readable; only this index and
living documentation treat Huginn as the canonical name.

## Índice de decisiones

`Accepted` indica que se eligió una dirección; no confirma por sí solo que esté implementada o verificada. Las relaciones de abajo resumen refinamientos y supersesiones parciales. Consulta [`validation.md`](../validation.md) y los documentos de cada entrega para encontrar evidencia de implementación.

| ADR | Decisión | Estado y relación actual |
| --- | --- | --- |
| [ADR-0001](0001-agent-product-and-documentation-boundaries.md) | Límites del producto agente y su documentación | Accepted; ADR-0011 resolvió el nombre provisional, y `docs/agent` fue sustituido por la ubicación actual `docs/huginn` |
| [ADR-0002](0002-own-rust-core-with-selective-reuse.md) | Núcleo Rust propio con reutilización selectiva | Accepted |
| [ADR-0003](0003-minimal-service-composition-kernel.md) | Kernel mínimo de composición de servicios | Accepted; ADR-0010 refinó el momento de invertir en plugins/UI y ADR-0012 la arquitectura |
| [ADR-0004](0004-personal-coding-agent-with-cli.md) | Agente personal de programación con CLI | Accepted para el MVP original; ADR-0010 reemplazó parcialmente la postergación de la TUI y la elección de una interfaz interactiva lineal a largo plazo |
| [ADR-0005](0005-openai-compatible-model-boundary.md) | Límite de modelo compatible con OpenAI | Accepted; ADR-0014 amplió la decisión con catálogo de proveedores y routing por turno |
| [ADR-0006](0006-native-tools-and-trusted-mcp-stdio.md) | Herramientas nativas y MCP stdio de confianza | Accepted |
| [ADR-0007](0007-read-freely-approve-effects.md) | Lectura libre y aprobación de efectos | Accepted |
| [ADR-0008](0008-repository-instructions-and-durable-context.md) | Instrucciones del repositorio y contexto durable | Accepted; ADR-0009 concretó el almacenamiento y los contratos de runtime |
| [ADR-0009](0009-sqlite-config-and-mvp-runtime-contracts.md) | SQLite, configuración y contratos de runtime del MVP | Accepted |
| [ADR-0010](0010-ratatui-plugin-first-interactive-terminal.md) | Terminal interactiva Ratatui-first y plugin-first | Accepted; ADR-0012 refinó la arquitectura interactiva |
| [ADR-0011](0011-huginn-product-naming.md) | Nombre e identidad del producto Huginn | Accepted; resuelve el nombre provisional de ADR-0001–00010 |
| [ADR-0012](0012-hexagonal-application-and-plugin-lifecycle.md) | Aplicación hexagonal y capacidades con ciclo de vida propio | Accepted; la implementación y su evidencia de validación se registran por separado |
| [ADR-0013](0013-authorable-plugin-contract-and-loading.md) | Contrato de plugins extensible y estrategia futura de carga | Accepted para el contrato in-process; la carga en runtime, hot reload y sandboxing quedan diferidos |
| [ADR-0014](0014-builtin-providers-and-turn-scoped-model-routing.md) | Providers integrados y routing del modelo por turno | Dirección Accepted; el estado de entregas y su aceptación se registran por separado |
| [ADR-0015](0015-separate-product-releases-from-cargo-publishing.md) | Separar releases de producto de publicación Cargo | Accepted; la publicación de Huginn sigue condicionada a la aceptación del producto |

## Record format

Use the [template](template.md) for new records. Each ADR covers a coherent decision,
its context, alternatives, consequences and follow-up evidence.
Keep implementation tasks and API specifications in separate documents linked to
the relevant ADRs.

Use Proposed for a choice still awaiting a decision, Accepted for an agreed choice,
Rejected for an evaluated choice not selected, and Superseded for a replaced decision.
Accepted decisions are replaced by a new ADR referencing the old record; preserve
the old rationale and add a supersession link rather than rewriting its history.

Number new ADRs sequentially. Do not change their identity when the product gets a name.

## Documentation authority and research

This directory is the canonical versioned decision log for the agent in Cortex.
The Obsidian rust-agent-harness-research collection remains the research home;
its DEC and R decision entries provide historical context.

Implementation: [`apps/huginn`](../../../apps/huginn/), [`core`](../../../crates/huginn/core/), [`runtime`](../../../crates/huginn/runtime/).

The repository placement and MCP trust decisions here refine earlier research
that left placement undecided or assumed stronger external-runtime isolation.
The external research notes now link to these canonical MVP decisions; their older
alternatives are preserved as research history.

## Current interactive architecture

[ADR-0010](0010-ratatui-plugin-first-interactive-terminal.md) controls interactive UI development and partially supersedes ADR-0004's TUI deferral. The Ratatui presentation plugin is the interactive implementation; the line-oriented CLI remains a supported recovery adapter. The agent core stays UI-agnostic and functional capabilities remain replaceable via plugin services. PTY tests determine necessary terminal workarounds; they are not a prerequisite for selecting Ratatui.

## Remaining decisions

- Public product name and final crate/configuration namespaces —
  **decided** by [ADR-0011](0011-huginn-product-naming.md) (Huginn).
  Historical ADRs still mention the legacy identifiers; those records
  are preserved unchanged.
- Supported real endpoint combinations and Rook compatibility.
- Release tags and public distribution after personal MVP acceptance.
- Runtime loading strategy for extensions — deferred by [ADR-0013](0013-authorable-plugin-contract-and-loading.md).

Concrete contracts and limits are in the [implementation specification](../implementation-specification.md).
ADR-0009 records the storage and configuration choices made for implementation.

## MVP validation boundary

The personal MVP targets macOS and Linux and the complete coding workflow described
in ADR-0004. Kernel lifecycle, tool-call streaming, approvals, MCP failures, context
compaction and interruption/recovery need focused acceptance checks.

Implementation follows the repository quality gates, including just ci-local.
Writing an ADR does not satisfy those gates or constitute an implemented feature.
