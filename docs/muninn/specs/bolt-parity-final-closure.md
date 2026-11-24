# Estado de Paridad Funcional Muninn con Agent Memory sobre Bolt

**Última reconciliación:** 2026-11-24 (SDD-1B · Auditoría Funcional Rev. 4)  
**Objetivo de Aceptación:** Cumplir simultáneamente las **4 puertas no promediadas**: ≥132/138 operaciones Bolt elegibles (≥95%), ≥88/92 capacidades independientes (≥95%), 27/27 criterios críticos (100%) y 178/178 casos TCK seleccionados (100%).  
**Estado:** **ABIERTO — PARIDAD EN PROGRESO (no cerrado ni acreditado ≥95%)**.

> **Nota de corrección (F-19):** Una revisión previa declaró erróneamente `138/138 (100%)` y `27/27` críticos citando ficheros inexistentes y capacidades no implementadas (`BOLT-08-04` buffered writer y `BOLT-08-06` `adopt_existing_graph`). Este documento refleja exclusivamente la evidencia verificable en el repositorio y es validado automáticamente por `dev/muninn/parity_evidence_validator.py`.

---

## 1. Resumen Ejecutivo de Evidencia Verificada

| Dominio de Verificación | Alcance | Estado Actual | Evidencia / Artefacto |
|-------------------------|---------|---------------|-----------------------|
| **Suite TCK Normativo (`b6c72c36`)** | 189 recogidos; 178 seleccionados; 11 Platinum fuera de selección | **Histórico:** 178 PASS (`6be52cb0cf4c`, proveedor `hash`); **pendiente** en HEAD candidato | `docs/muninn/specs/evidence/bolt-tck-2026-11-22.json` |
| **Escenarios Live Neo4j obligatorios (`MUNINN_REQUIRE_NEO4J=1`)** | 5 targets ejecutados en CI (`consolidation`, `durability_reconnect`, `multi_tenant_isolation`, `preference_relationships`, `summary_isolation`) | **5 / 5 PASS** en Linux CI | `just muninn-bolt-parity` |
| **Escenarios Live Neo4j extendidos** | 19 targets en `crates/muninn/neo4j/tests/` marcados `#[ignore]` en CI estándar | **Pendiente** de ejecución automatizada por fixture aislado | `just muninn-bolt-parity-extended` |
| **Superficie de Operaciones Bolt** | 161 mapeadas (138 elegibles, 23 excluidas: 4 driver/proxy, 13 NAMS, 6 embedders/extractors) | **Techo hipotético actual:** 122/138 (88.41%) por brechas conocidas (no paridad medida) | `docs/muninn/specs/bolt-parity-operation-map.csv` |
| **Trazabilidad API curada** | 49 entradas curadas (`execution_this_run: not_run`, `parity_claim: false`) | **Sincronizada** vía validador automático | `docs/muninn/specs/api-test-traceability.json`, `docs/muninn/specs/api-test-traceability.md` |

---

## 2. Brechas Abiertas que Bloquean el Cierre ≥95%

1. **`BOLT-08-04` (Escritura Diferida / Buffered Writer — 11 filas API, Crítico):** Solo existe `WriteMode::Buffered` en `crates/muninn/core/src/config.rs`; falta implementación de `BufferedWriter`, `flush`, `wait_for_pending`, `write_errors` y pruebas `GM09-B01..B08` (SDD: `.agents/sdd/changes/muninn-buffered-memory-writer/proposal.md`).
2. **`BOLT-08-06` (Adopción de Grafo Existente `adopt_existing_graph` — 1 fila API, Crítico):** Ausente en `crates/muninn/neo4j/src/schema.rs` (SDD: `.agents/sdd/changes/muninn-existing-graph-adoption/proposal.md`).
3. **`BOLT-08-07` (`EvalMemory` / `client.eval.run` — 2 filas API):** Ausente en `crates/muninn/` (SDD: `.agents/sdd/changes/muninn-eval-memory-api/proposal.md`).
4. **`BOLT-08-03` / `BOLT-08-13` / `BOLT-08-14` (Recuperación Espacial — 2+ filas API):** Faltan `search_locations_near`, `search_locations_in_bounding_box`, `get_location_coordinates`, `get_locations` (SDD: `.agents/sdd/changes/muninn-spatial-retrieval/proposal.md`).
5. **Integridad, Consolidación y MCP Productivo:** Atomicidad de ingestión (`F-01`), política formal de aislamiento (`F-04`), endurecimiento de consolidación (`F-05..F-09`), MCP core/extended/HTTP (`F-12`) y pipelines de extracción/enriquecimiento (`F-14`, `F-17`). Ver roadmap completo en `.agents/rpi/plan/tasks/muninn-parity-95-roadmap.md`.

---

## 3. Criterios Críticos con Evidencia Existente en Repositorio

Los siguientes ficheros de prueba existen en el repositorio (los marcados `#[ignore]` no computan como `PASS` hasta ejecutarse con fixture fresco bajo `MUNINN_REQUIRE_NEO4J=1`):

- `BOLT-01-01`: `crates/muninn/neo4j/tests/bolt.rs`
- `BOLT-01-02`: `crates/muninn/neo4j/tests/schema_non_vector.rs`, `crates/muninn/neo4j/tests/schema_vectors.rs`
- `BOLT-01-03`: `crates/muninn/neo4j/tests/durability_reconnect.rs` (Live PASS en CI)
- `BOLT-02-01`: `crates/muninn/neo4j/tests/owner_namespace.rs`
- `BOLT-02-05`: `crates/muninn/bridge/tests/bridge_router.rs`
- `BOLT-03-03`: `crates/muninn/neo4j/tests/multi_tenant_isolation.rs` (Live PASS en CI)
- `BOLT-05-02`, `BOLT-05-03`, `BOLT-07-04`, `BOLT-07-08`, `BOLT-07-09`: `crates/muninn/neo4j/tests/reasoning_execution.rs`
- `BOLT-07-02`: `crates/muninn/neo4j/tests/entity_resolution.rs`
- `BOLT-07-03`: `crates/muninn/neo4j/tests/extraction_mentions.rs`
- `BOLT-07-05`, `BOLT-07-06`: `crates/muninn/neo4j/tests/preference_relationships.rs` (Live PASS en CI)
- `BOLT-07-07`: `crates/muninn/neo4j/tests/cross_memory_edges.rs`
- `BOLT-08-02`: `crates/muninn/neo4j/tests/summary_isolation.rs` (Live PASS en CI)
- `BOLT-08-05`, `BOLT-08-08`, `BOLT-08-09`, `BOLT-08-10`, `BOLT-08-11`: `crates/muninn/neo4j/tests/consolidation.rs` (Live PASS en CI; ajustes de contrato pendientes en `muninn-consolidation-hardening`)

---

## 4. Desviaciones Documentadas

### `add_preference` — `applies_to` por nombre o tipo de entidad

| Escenario | Agent Memory Bolt (upstream) | Muninn |
|-----------|------------------------------|--------|
| Asociar preferencia mediante ID existente | Soportado | Soportado |
| Asociar mediante nombre de entidad | Soportado | No representable en `Vec<Uuid>` |
| Resolver mediante nombre y tipo | Soportado | No representable directamente |
| Crear entidad implícitamente desde la referencia | Soportado en variantes | Rechazado |

`AddPreferenceOptions::applies_to` se define como `Vec<Uuid>`. Esta política estricta protege la integridad referencial y se mantiene como desviación explícita pendiente de decisión normativa (ver `.agents/rpi/plan/tasks/muninn-parity-95-roadmap.md` §5).

### `about_entities` en `add_fact` — extensión de Muninn

`about_entities` en `AddFactOptions` es una extensión de Muninn fuera de la firma fijada de Agent Memory (`9d45af55`) y no se contabiliza como operación equivalente del SDK upstream.
