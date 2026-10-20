# Architecture Decision Records (ADRs)

Los ADRs documentan decisiones significativas de arquitectura para el proyecto Cortex.

## ADRs generales

> Los ADRs específicos de Huginn están en [docs/huginn/adr/](../huginn/adr/README.md).

Por ahora no hay ADRs generales pendientes. Los ADRs de productos y arquitectura
específicos de cada componente viven en sus respectivos directorios:

- [ADRs de Huginn](../huginn/adr/) — Decisiones sobre el agente de coding
- [ADR-0015: Separación de releases de producto y publicación Cargo](../huginn/adr/0015-separate-product-releases-from-cargo-publishing.md) — Política de distribución

## Formato de ADR

Los ADRs siguen el formato estándar:

```markdown
# ADR-XXXX: Título descriptivo

- **Estado:** [Aceptado|Rechazado|Propuesto|Deprecated]
- **Fecha:** YYYY-MM-DD
- **Alcance:** Contexto del ADR

## Resumen

Descripción breve del problema y la decisión.

## Contexto

El problema o situación que motiva esta decisión.

## Decisión

La decisión tomada y su justificación.

## Consecuencias

- Positivas
- Negativas
- Neutrales
```

Ver [template.md](template.md) para la plantilla completa.
