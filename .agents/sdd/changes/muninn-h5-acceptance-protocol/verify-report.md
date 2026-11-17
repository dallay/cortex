# Informe de verificación — protocolo de aceptación H5

## Verificación repetida — Fase 10 (10.12–10.13)

**Fecha**: 2026-11-09  
**Cambio**: `muninn-h5-acceptance-protocol`  
**Versión**: `h5-acceptance/v1`  
**Almacén**: OpenSpec  
**Alcance**: reevaluar todos los requisitos de Fase 10 después de corregir la aplicabilidad de `acceptance_rules_and_tolerances`; el historial de Fases 1–9 se conserva abajo.

### Estado y resumen ejecutivo

**Status: success** — La implementación estructural revisada ahora exige una regla por cada gate habilitado y rechaza las reglas de gates deshabilitados, de forma coincidente en Python y JSON Schema. Las cuatro combinaciones de gates, incluidos ambos deshabilitados, están cubiertas: en este último caso `pending` con `value: null` se permite y sigue no-ready; `approved` con objeto vacío se rechaza. La suite H5 pasó con 62 tests y los checks de esquema, compilación Python, Markdown y diff pasaron.

**Veredicto técnico: PASS WITH WARNINGS para la implementación del template/validador, no para aceptación del producto.** No quedan hallazgos técnicos abiertos de Fase 10 dentro de este alcance. QA continúa `BLOCKED`; Clippy `-D warnings` sigue siendo un bloqueo previo conocido y no se volvió a ejecutar.

La revisión directa de `retrieval_h5_protocol.py`, JSON Schema, fixture, pruebas, spec y design confirma que esta corrección no requiere habilitar al menos un gate, no inventa reglas ni valores, no declara protocolo confirmatory-ready y no altera aprobación/corroboración. No se ejecutaron Docker, bridge o router productivo en proceso, DB ni mediciones. La identidad del encoder sigue siendo self-report, no atestación.

### Completitud y validación ejecutada

| Métrica | Resultado |
|---|---|
| Tareas de implementación Fase 10 | 10.1–10.13 marcadas completas en `tasks.md`; tareas bloqueantes B.1/B.2 siguen pendientes por falta de protocolo aprobado y autorización de medición, fuera de esta unidad |
| Pruebas H5 | `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_*.py' -v` — exit 0; 62 tests, `OK` |
| Compilación Python | `python3 -m py_compile dev/muninn/retrieval_h5_protocol.py dev/muninn/retrieval_h5_provenance.py dev/muninn/retrieval_h5_runner.py dev/muninn/tests/test_retrieval_h5_protocol.py dev/muninn/tests/test_retrieval_h5_provenance.py dev/muninn/tests/test_retrieval_h5_runner.py` — exit 0 |
| JSON Schema / fixture | `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.example.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; solo advertencia de deprecación de CLI |
| Markdownlint | Proposal, spec, design, tasks y apply-progress — exit 0; 0 issues |
| `git diff --check` | Exit 0 |
| Runner SDD / cobertura | `.agents/sdd/quality-runner.json` no está configurado: fallback, no enforcement determinista. Umbral de cobertura no configurado. |
| Clippy | No se ejecutó. Sigue el bloqueo conocido previo: `-D warnings` falló con 93 errores en lib y 120 en tests `all-targets`; no se presume resuelto. |

### Matriz de cumplimiento — Fase 10

| Requisito / escenario | Evidencia | Resultado |
|---|---|---|
| Envoltura cerrada `pending`/`approved`, `value` y `rationale` explícitos | `test_decision_envelopes_require_explicit_status_value_and_rationale`; `test_decision_envelope_rejects_empty_placeholder_wrong_type_and_unknown_keys`; suite H5 62/62. | ✅ Conforme |
| Esquemas estrictos por ruta: tipos, propiedades allowlisted, no objetos/listas vacíos, placeholders ni claves desconocidas | Inspección de `_VALUE_SCHEMAS`/`_validate_value` y `$defs/values`; cobertura de decisiones malformadas y fixture/schema cruzado. | ✅ Conforme |
| Hallazgos `missing`/`pending`/`malformed` ordenados por JSON path y bloqueo de readiness | `test_missing_pending_and_malformed_paths_are_separate_and_sorted` y `test_finding_report_is_deterministic_and_blocks_operator_corroboration`; implementación ordena cada colección y `confirmatory_ready` requiere las tres vacías. | ✅ Conforme |
| Aplicabilidad de campos por gate | Pruebas de campos K, labels/rubric y alcance ANN–exact según gate; `test_acceptance_rules_match_enabled_gates_in_python_and_schema` ejecuta las cuatro combinaciones y valida Python + schema. | ✅ Conforme; regla exigida solo para gate enabled y regla extra de gate disabled rechazada |
| Ambos gates deshabilitados | Prueba 10.12–10.13 confirma `pending`/`null` aceptado estructuralmente pero no-ready, sin exigir habilitar gate; `approved` con `{}` rechazado por el validador y `minProperties: 1` del schema. | ✅ Conforme; no se agrega requisito de gate habilitado |
| Fixture pendiente e incompleta, sin decisiones por defecto | Fixture contiene metadata `PENDING` y decisiones `pending` con `value: null`; `test_incomplete_example_is_not_confirmatory_or_hash_approved`; schema acepta el fixture. | ✅ Conforme; no confirmatory-ready |
| Declaración `approved` no sustituye aprobación; hash completo y corroboración operador | Inspección de `corroborate_approval`; tests de hash/corroboración y timestamps. La función requiere readiness y registro coincidente del operador. | ✅ Conforme estructuralmente; ningún protocolo concreto aprobado |
| Sin defaults, inferencias, decisión automática ni `PASS` estructural | `validate_protocol` acumula findings, no completa valores; `confirmatory_ready` se deriva solo de ausencia de findings. El runner conserva estados de bloqueo. | ✅ Conforme para el validador; no implica aceptación |
| TDD de la corrección condicional | `apply-progress.md` registra RED (fallan ambos casos de un gate por exigir regla disabled), GREEN y REFACTOR; prueba enfocada y suite completa repetidas y verdes. | ✅ Evidencia registrada y corroborada por la suite fresca |

### Coherencia con diseño y corrección del hallazgo anterior

| Decisión de diseño | Resultado | Evidencia |
|---|---|---|
| Aplicabilidad de reglas definida por la combinación de gates, con propiedades cerradas | ✅ Seguida | Python construye el esquema de reglas solo desde gates enabled; JSON Schema exige las reglas enabled y prohíbe reglas disabled. |
| Ambos gates deshabilitados no fuerzan habilitación ni fabrican una regla | ✅ Seguida | La envoltura pending conserva `value: null` y reporta pending; el esquema no obliga un gate enabled. El objeto aprobado vacío falla cerrado. |
| Validación estructural no decide los gates ni constituye aprobación | ✅ Seguida | No cambió la lógica de corroboración humana ni se añadieron criterios de aceptación. |
| Fixture incompleta y decisiones de producto pendientes | ✅ Seguida | Fixture permanece pending; valores estructurales de prueba no se incorporan al fixture. |

El hallazgo anterior — exigir reglas para gates deshabilitados — queda **cerrado** con evidencia concordante de implementación, schema y test para ambas combinaciones de gate único y para los dos gates deshabilitados. No se detectaron discrepancias residuales de Fase 10.

### Límites de assurance, riesgos y veredicto

- No se aprobaron valores de K, thresholds/umbrales, regla PASS, número de corridas, agregación, tie policy, tratamiento final de métricas undefined, corpus, labels, rúbrica ni adjudicación. Tampoco se aprobó un protocolo concreto de corrida.
- La plantilla y sus tests prueban validación estructural local; no prueban aceptación de producto, calidad de retrieval, procedencia `verified` ni una medición confirmatoria.
- La identidad del encoder es self-report; no es atestación criptográfica ni prueba qué pesos/configuración están realmente en memoria, y no resiste compromiso del bridge, host, Docker u operador.
- No se ejecutaron Docker/bridge, no se inspeccionó el router productivo en proceso, no hubo mediciones ni acceso a DB. No se reclasifica la corrida histórica descriptiva.
- QA de producto sigue `BLOCKED` según `qa-report.md`. Clippy `-D warnings` permanece como bloqueo previo conocido (93 errores lib / 120 tests all-targets); no se volvió a ejecutar ni se relajó el gate.

**PASS WITH WARNINGS** para la implementación del template/validador estructural. No es aceptación del producto, no autoriza mediciones confirmatorias y no elimina los bloqueos de QA, Clippy o procedencia operativa.

---

### Historial de verificación previo — Fases 1–9

---

### Historial de verificación previo — Fases 1–9

**Cambio**: `muninn-h5-acceptance-protocol`  
**Versión**: `h5-acceptance/v1`  
**Almacén**: OpenSpec  
**Fecha**: 2026-11-09

## Resumen ejecutivo

La implementación ejecutable de la Fase 9 coincide en lo principal con la propuesta, la especificación y el diseño: identidad allowlisted desde el encoder de `App`, fingerprint de configuración calculado durante la carga, respuesta diagnóstica autenticada y feature-gated, y rechazo estricto de identidades malformadas. Las pruebas solicitadas pasaron. La verificación no inspeccionó el router productivo en proceso ni ejecutó Docker/bridge en vivo; por tanto no acredita procedencia operativa. El self-report no es atestación criptográfica ni prueba los pesos o la memoria del proceso.

**Veredicto técnico: PASS WITH WARNINGS.** El gate Clippy con `-D warnings` falla por diagnósticos existentes del crate/sub-workspace, aunque el filtro por archivos no encontró diagnósticos en las funciones nuevas ni en `main.rs`. No se relajan lints. La procedencia del flujo descriptivo sigue `partial`; esto, la falta de un protocolo-run aprobado y los resultados de Clippy impiden aceptar el producto o autorizar mediciones confirmatorias. No se inició medición.

## Completitud

| Métrica | Valor |
|---|---:|
| Tareas ejecutables marcadas completas | 28 grupos de tareas |
| Tareas ejecutables marcadas incompletas | 0 |
| Bloqueos previstos de medición | 2 (B.1 y B.2; intencionalmente pendientes) |

Las fases 1–9 y sus tareas ejecutables están marcadas completas en `tasks.md`. B.1 y B.2 no son tareas de implementación pendientes: bloquean un protocolo-run concreto, la aceptación y cualquier medición confirmatoria hasta completar y aprobar sus campos y verificar procedencia.

## Compilación, pruebas y validaciones

**Build y pruebas Rust**: ✅ comandos completados con exit 0.

- `cargo test --manifest-path dev/muninn/bridge/Cargo.toml --package muninn-bridge-spike --lib`: 172 pasaron, 6 ignoradas.
- Mismo comando con `--features h5-diagnostic`: 174 pasaron, 6 ignoradas.
- `cargo test --manifest-path dev/muninn/bridge/Cargo.toml --package muninn-bridge-spike --bin muninn-bridge-spike`: 1 pasó, sin feature.
- Mismo comando con `--features h5-diagnostic`: 3 pasaron; incluye test de identidad de la instancia encoder y rechazo HTTP sin autenticación.

**Pruebas Python H5**: ✅ `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_*.py' -v` — exit 0; 50 pasaron.

**Checks de artefactos**: ✅

- `cargo fmt --all -- --check` — exit 0.
- `git diff --check` — exit 0.
- Markdownlint de `docs/muninn/retrieval-h5-runner.md`, `tasks.md` y `apply-progress.md` — exit 0.
- Runner de calidad SDD: `.agents/sdd/quality-runner.json` no existe; evidencia en modo fallback, no enforcement determinista.
- Cobertura: no hay umbral configurado en `config.yaml`.

**Clippy**: ⚠️ `cargo clippy --manifest-path dev/muninn/bridge/Cargo.toml --package muninn-bridge-spike --all-targets --features h5-diagnostic -- -D warnings` falla. El registro disponible informa 93 errores para lib y 120 para test `--all-targets`; el filtro de archivos modificados encuentra lints en APIs/documentación existentes de `artifact_model_stage.rs`, `embedding_port.rs` y `minilm_encoder.rs`, no en las funciones nuevas de identidad/fingerprint ni en `main.rs`. Clippy sin feature también falla por lints del crate. No se corrigieron lints ajenos ni se redujo el gate.

No se ejecutó aceptación QA. No se inició Docker, un bridge, una inspección Docker activa ni mediciones.

## Matriz de cumplimiento de especificación

| Requisito / escenarios | Evidencia ejecutada o inspeccionada | Resultado |
|---|---|---|
| Gates independientes; métricas ANN–exact y calidad absoluta, sin inferir aceptación ni PASS | Suites H5 y pruebas de calidad incluidas en la ejecución de 50; inspección de `retrieval_quality.py` y `retrieval_h5_runner.py`. K permanece argumento; MRR es descriptiva. | ✅ Conforme para framework; sin protocolo-run ni aceptación |
| ANN–exact: IDs/orden, salida exacta vacía, underfill y scores no comparados | Pruebas de métricas en suite H5; implementación usa resultados disponibles y no compara score para estas métricas. Ambas métricas son indefinidas si `Exact_topK` está vacío. | ✅ Conforme |
| Recall/nDCG y MRR | Suite H5 y funciones de calidad; fórmulas de Recall/nDCG según juicios y grados; MRR descriptiva. | ✅ Conforme; no se aprobaron etiquetas del piloto |
| Campos y aprobación previa; indefinidos, empates y valores de protocolo abiertos | Pruebas de protocolo/preflight en suite H5; `validate_protocol` mantiene campos requeridos y no introduce valores estadísticos por defecto. | ✅ Conforme; B.1/B.2 siguen bloqueadas |
| Self-report de identidad: allowlist, campos separados, hash de config cargada | Tests Rust de lib/bin y tests de sanitizer; `CandidateCpuSentenceEncoder::load` deriva perfil/hash de los buffers ya leídos y validados; no se presenta como hash de pesos/memoria. | ✅ Conforme estructural y en tests |
| Identidad de la misma instancia de encoder y respuesta autenticada | Test bin de identidad de instancia más test HTTP sin auth; el handler obtiene identidad desde `app.encoder` y une identidad con ANN/exact. | ✅ Evidencia de handler/test; ⚠️ router productivo completo no inspeccionado en proceso |
| Sanitización y correlación respuesta/protocolo/ejecución/query | Tests de sanitizer/correlación y mismatch/faltantes pasan. `run_evaluation` requiere la identidad en la misma respuesta, pero asigna explícitamente `partial` porque no recibe identidad esperada ni observaciones Docker/run independientes. | ⚠️ Parcial deliberadamente: no hay correlación operativa ni `verified` en flujo descriptivo |
| Feature gate y compatibilidad de ruta | Código registra `/h5/diagnostic/search` directamente bajo `#[cfg(feature = "h5-diagnostic")]` en `run()`; tests lib/bin pasan con y sin feature. | ✅ Contrato de compilación; ⚠️ no hubo inspección in-process de la tabla del router productivo ni runtime |
| No PASS con identidad/procedencia/protocolo incompletos; histórica no reclasificada | Tests de preflight y runner; los resultados del flujo descriptivo declaran `partial` y `measured_not_accepted`; no se modificó ni evaluó la corrida histórica. | ✅ Conforme para el bloqueo; no es una autorización de medición |

**Resumen**: comportamiento unitario y de handler cubierto por las suites repetidas; escenarios que requieren corroboración de una ejecución Docker real y el router productivo en runtime permanecen sin verificación operativa. La evidencia actual no prueba que la procedencia completa alcance `verified`.

## Correctitud estática y coherencia de diseño

| Requisito / decisión | Estado | Evidencia / límite |
|---|---|---|
| Identidad deriva del encoder que codifica la consulta | ✅ Conforme | Trait `EmbeddingPort` ofrece identidad opcional; el encoder de aplicación deriva identidad del encoder retenido y el handler consulta esa instancia. Test con encoder diferenciable. |
| Fingerprint de config y perfil | ✅ Conforme | Carga calcula SHA-256 con framing a partir de los buffers validados, retiene valores y no reabre archivos al reportar. No acredita pesos ni memoria. |
| Allowlist y separación `declared` / `loaded` | ✅ Conforme | Tipos Rust serializables y sanitizer Python exigen claves exactas, strings no vacíos y formatos/valores esperados. |
| Autenticación y feature gate | ✅ Parcialmente verificado | Registro productivo condicional visible en `run()`; tests de bin cubren build con/sin feature y rechazo sin auth. No inspección de router productivo en proceso. |
| Correlación con Docker/build/run/query externo | ⚠️ Límite funcional | Helper verifica igualdad de identidad y binding run/query si se le entregan observaciones. El flujo `run_evaluation` no tiene esas observaciones y marca self-report `partial`; no se verificó correlación operativa en una ejecución real. |
| No cambiar retrieval normal, Store ni criterios de aceptación | ✅ Conforme en alcance inspeccionado | Cambio acotado al endpoint feature-gated y framework; no se fijan K, thresholds/PASS, agregación, desempates ni política final de undefined. |

## Auditoría TDD

| Métrica | Estado |
|---|---|
| Aplicabilidad | Sí para identidad, ruta diagnóstica, parser y correlación; no aplicable a documentación por sí sola. |
| Evidencia RED→GREEN→REFACTOR | ✅ Evidencia registrada para loader, handler y sanitizer/correlación en `apply-progress.md`; esta verificación repitió las suites indicadas. |
| Limitación de atribución | La verificación valida los registros existentes de RED/GREEN/REFACTOR; no reconstruye una secuencia histórica distinta. |
| Orden de implementación | ⚠️ No se infiere solo de las pruebas verdes; el apply-progress conserva evidencia de RED explícita para los slices principales. |

## Hallazgos

### CRITICAL

Ninguno para la implementación del framework dentro del alcance verificado.

### WARNING

- Clippy con `-D warnings` no pasa por lints del crate/sub-workspace. El gate sigue siendo riesgo de integración; no se relajó.
- No hubo inspección in-process del router productivo ni ejecución live. Los tests de bin/handler no sustituyen esa evidencia.
- No hubo Docker/bridge ni observación Docker operativa. La identidad reportada es self-report y el flujo descriptivo marca procedencia `partial`; no acredita qué pesos/configuración están en memoria ni resiste compromiso del proceso, bridge, host u operador.
- El helper de correlación requiere identidad esperada y evidencia/binding externos; `run_evaluation` no los tiene y no los correlaciona, sino que conserva `partial`. Por ello no hay evidencia de procedencia `verified` en ejecución.
- El protocolo-run concreto sigue incompleto y sin aprobación: no inferir K numérico, umbrales o PASS, protocolo de corrida, agregación, desempates ni tratamiento final de métricas indefinidas. Las métricas ANN–exact vs. calidad absoluta son gates independientes; MRR sigue descriptiva.
- No se autoriza aceptación del producto ni medición confirmatoria. El informe de QA es independiente y no se usó como sustituto de esta verificación.

### SUGGESTION

Ninguna adicional dentro del alcance técnico permitido.

## Veredicto

### PASS WITH WARNINGS

Las pruebas repetidas pasan y el código inspeccionado implementa la identidad feature-gated sin ampliar claims de atestación. Los límites de Clippy y de evidencia runtime/procedencia quedan abiertos. Este veredicto técnico no declara el producto aceptado ni autoriza mediciones.

---

## Phase 11 Verification — Review-only Protocol and Corpus Candidates

**Date**: 2026-11-09  
**Change**: `muninn-h5-acceptance-protocol`  
**Version**: `h5-acceptance/v1`  
**Artifact store**: OpenSpec  
**Scope**: Phase 11 candidate protocol and independent unlabelled corpus artifacts only. Earlier phase history above is preserved.

## Executive summary

**Technical verdict: PASS WITH WARNINGS for artifact structure and provenance checks only.** The candidate protocol records the approved ANN–exact decision fields exactly: `K=10`, both required per-query metrics equal to `1.0`, one run, per-query reporting, descriptive-only macro-average, no retrospective tie-breaking, and undefined required metrics block `PASS`. Absolute relevance remains disabled. Offline validation reports `confirmatory_ready=false` and no malformed decisions.

The corpus candidate passes structural validation with one query and ten candidates. Its canonical hash is `sha256:44ef7c4ad3fbe179d2651564eff980ad4d7c6b2c315d790928a08b1b76a859d7`. All 11 source mappings were independently checked by the orchestrator against Git baseline objects at `09bb21d5fd2c847ac7df5e48f7be06eb08cec445`: mappings are unique and complete, all paths are in the eligible allowlist, and all source SHA-256 values match.

One query is a minimal review sample, not a product-ready or representative evaluation corpus. Owner review and explicit freeze remain pending. Security/sensitivity review, broader query scope, applicable comparison scope, encoder identity, operational provenance, and applicable protocol approval are also pending. `confirmatory_ready=false` does not authorize measurements. No labels, retrieval, inference, Docker, bridge, database, or measurement was run. Existing QA remains `BLOCKED`; the previously reported Clippy blocker remains and was not rerun. Do not archive.

## Completeness

| Metric | Result |
|---|---:|
| Phase 11 tasks marked complete | 7 (11.1–11.7) |
| Phase 11 tasks marked incomplete | 0 |
| Separate measurement blockers | B.1 and B.2 remain blocked; not Phase 11 artifact tasks |

The corpus candidate was constructed because the single proposed query has the required minimum of ten eligible candidates. This satisfies only the minimum structural check for that query. It does not establish corpus sufficiency, relevance, adequacy, or representativeness. User review, sensitivity classification/review, and freeze are outstanding.

## Build, tests, and artifact checks

**Build / product tests**: Not applicable to this data-and-artifact work unit. No product behavior was changed. No artificial TDD test was added.

**Offline artifact checks**: All reported checks passed.

- `validate_protocol` — exit 0; `confirmatory_ready=false`, no malformed decisions, and pending repository/approval metadata, corpus scope, ANN comparison scope, encoder identity, and provenance.
- `validate_corpus` — passed; one query with ten candidates.
- Canonical `corpus_hash` — recomputed and matched the manifest: `sha256:44ef7c4ad3fbe179d2651564eff980ad4d7c6b2c315d790928a08b1b76a859d7`.
- Baseline source verification — all 11 mappings unique and complete; every path in the eligible allowlist; every SHA-256 matched the Git object at baseline `09bb21d5fd2c847ac7df5e48f7be06eb08cec445`.
- Candidate protocol JSON Schema validation — passed; CLI deprecation warning only.
- Corpus and provenance JSON parsing — passed.
- Markdownlint — passed, 0 issues.
- `git diff --check` — passed.
- Clippy `-D warnings` — not rerun; the prior blocker remains open.
- SDD quality runner — not configured; mode remains `fallback`, not deterministic enforcement. Coverage threshold is not configured.

These checks validate artifact structure and source-byte consistency. They do not validate relevance, content suitability, security/sensitivity approval, corpus adequacy, or product behavior.

## Spec compliance matrix — Phase 11

| Requirement / scenario | Evidence / validation | Result |
|---|---|---|
| Approved ANN–exact decision fields represented exactly | Candidate JSON inspection and `validate_protocol`; K=10, both metrics exactly 1.0 per query, one run, per-query results, descriptive-only macro-average, observed ties, undefined blocks PASS | ✅ COMPLIANT — candidate decisions only |
| Absolute relevance remains disabled; no threshold or labels are inferred | Candidate JSON has `absolute_relevance=false`; labels/rubric decision remains pending | ✅ COMPLIANT |
| Protocol remains non-ready and does not authorize a run | `validate_protocol`: `confirmatory_ready=false`; approval metadata, corpus scope, ANN scope, encoder identity, and provenance remain pending | ✅ COMPLIANT |
| Independent, unlabelled corpus and required minimum | Corpus validator passed; one query, ten candidates; inspected corpus has no labels, grades, scores, rankings, relevance judgments, or retrieval outputs | ✅ COMPLIANT for artifact constraints; ⚠️ PARTIAL for evaluation scope (one query only) |
| Baseline source allowlist and immutable source hashes | Orchestrator independently recomputed all 11 mappings and source hashes from baseline Git objects; all paths allowed and hashes matched | ✅ COMPLIANT for declared source provenance |
| Pilot, C0/C1, modified runner guide, and generated/untracked material excluded | Inventory/exclusion checks reported passed; no such material is included in the candidate artifacts | ✅ COMPLIANT for exclusion checks |
| Corpus remains draft/unlabelled/unfrozen and not represented as adequate or representative | Provenance status is `draft/not frozen`; owner review and freeze are pending; one query is explicitly a minimal review sample | ✅ COMPLIANT |
| Sensitivity and owner review before freeze | Project sensitivity classification is absent; source-path and text review remains required | ⚠️ PARTIAL — review remains pending by design |
| No retrieval, inference, Docker/bridge/DB, measurement, or product acceptance | Orchestrator evidence records none performed; protocol remains not-ready | ✅ COMPLIANT |

**Compliance summary**: Artifact-only requirements pass within the stated boundaries. The corpus is not a representative or product-ready evaluation corpus. No product acceptance scenario was exercised; acceptance QA remains the separate, blocked gate.

## Correctness and design coherence

| Requirement / design decision | Status | Evidence / limitation |
|---|---|---|
| Preserve exact approved ANN–exact decision values and keep absolute relevance disabled | ✅ Implemented | Candidate JSON fields match the Phase 11 contract. |
| Keep run-specific approvals and operational inputs unresolved | ✅ Implemented | Protocol validator reports not-ready; relevant decisions and metadata remain pending. |
| Use strict corpus shape and companion provenance manifest | ✅ Implemented | `validate_corpus`, JSON parsing, hash comparison, closure and unique mapping checks passed. |
| Use only eligible immutable-baseline sources and preserve exclusions | ✅ Verified | All 11 mappings and hashes checked against baseline Git objects and allowlist. |
| Do not pad below ten candidates; do not overclaim from the minimum | ✅ Followed | One query has ten candidates; this is a minimal review sample only, not corpus adequacy evidence. |
| Keep the corpus draft, unlabelled, and not frozen pending owner review | ✅ Followed | Manifest says `draft/not frozen`; no labels or review/freeze approval asserted. |
| Do not authorize measurements or claim acceptance | ✅ Followed | `confirmatory_ready=false`; no measurement or runtime operation occurred. |

## TDD applicability

| Metric | Status |
|---|---|
| Task type | Data/document artifact work; no business/product behavior changed |
| TDD applicability | Not applicable; no artificial TDD was introduced |
| Relevant artifact validation | Passed as listed above |

## Issues and risks

### CRITICAL

None for artifact structure within this verification scope. Product acceptance and measurement remain blocked, not passed.

### WARNING

- One query is only a minimal review sample. It is not a product-ready, sufficient, or representative evaluation corpus.
- Owner review of all query/candidate text and source mappings, explicit corpus freeze, and security/sensitivity review remain pending. The project has no established sensitivity classification; MIT license context does not constitute rights or sensitivity approval.
- Broader query scope and applicable ANN comparison scope remain pending, as do encoder identity, operational provenance, and applicable approval metadata. `confirmatory_ready=false`; no measurement is authorized.
- Existing acceptance QA remains `BLOCKED`. The prior Clippy `-D warnings` blocker remains open and was not rerun.
- The independent reviewer reported no findings but did not recompute the canonical hash or source content. The orchestrator separately recomputed the corpus hash and all baseline source hashes as recorded above.

### SUGGESTION

None beyond completing the pending review and approval steps before any later freeze or measurement request.

## Verdict

### PASS WITH WARNINGS

Artifact structure, canonical hash, source mapping, and baseline source-byte checks pass. This is a technical artifact verification only. It is not product acceptance, does not establish corpus adequacy or representativeness, does not clear QA or Clippy, and does not authorize measurements. Do not archive.

---

## Verificación nueva — slice offline hasta U11 / cierre Fase 17

**Fecha**: 2026-11-14  
**Cambio**: `muninn-h5-acceptance-protocol`  
**Versión**: `h5-acceptance/v2` para la API ejecutable; los artefactos candidatos conservan su propia versión  
**Almacén**: OpenSpec  
**Alcance**: verificación técnica del slice offline ANN-only U7–U11 y las tareas de cierre 17.1–17.3. El material histórico anterior de este archivo, incluido el QA BLOCKED, se conserva sin reemplazo.

### Estado y resumen ejecutivo

**Veredicto técnico: PASS WITH WARNINGS para el slice offline**, no para aceptación H5, autorización de medición real ni cierre de los bloqueos históricos. El discovery offline de retrieval pasó: 148 tests. Las tres pruebas de compatibilidad del piloto pasaron; los ejemplos protocol/schema, la compilación Python, Markdownlint y `git diff --check` también pasaron.

Las tareas 17.1–17.3 tienen evidencia registrada: caracterización de CLI/anotaciones e informe descriptivo del piloto; error de escritura con código 2 sin promesa de persistencia; y `protocol_preflight` diagnóstico sin bridge ni PASS. Los tests de U11 cubren criterio/métricas, indefinidos, underfill/empates, procedencia por consulta, cobertura incompleta y hashes. Se preservaron QA/verify históricos y no se ejecutaron sistemas reales, DB, corpus real, mediciones, QA ni archive.

### Completitud

| Métrica | Resultado |
|---|---:|
| Tareas de implementación 12–17 (U7–U11 y cierre) | 18/18 marcadas completas |
| Tareas de implementación incompletas en este slice | 0 |
| Bloqueos de ejecución/aceptación | B.1 y B.2 continúan `[ ]`, fuera del slice técnico |

Las tareas 17.1, 17.2 y 17.3 están marcadas completas. B.1/B.2 siguen bloqueadas por protocolo-run, aprobaciones y procedencia operativa pendientes; no son omisiones de implementación de Fase 17.

### Compilación, pruebas y checks

**Build**: No ejecutado. Fase 17 excluye builds/Clippy para este slice Python; no se infiere un build aprobado.

**Pruebas**:

- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py'` — exit 0; 148 tests, `OK`.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.PilotCompatibilityTests -v` — exit 0; 3 tests, `OK`.
- `python3 -m py_compile dev/muninn/tests/test_retrieval_h5_runner.py dev/muninn/retrieval_h5_runner.py dev/muninn/retrieval_h5_protocol.py dev/muninn/retrieval_h5_provenance.py dev/muninn/retrieval_quality.py` — exit 0.

**Checks de artefactos**:

- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.example.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.candidate.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0. La CLI emitió una advertencia de deprecación.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md docs/muninn/retrieval-h5-runner.md` — exit 0; 0 incidencias.
- `git diff --check` — exit 0; sin salida.
- `.agents/sdd/quality-runner.json` no está configurado: `fallback`, no enforcement determinista. `coverage_threshold` no está configurado; no se generó cobertura.

**No ejecutado por límites explícitos**: builds, Clippy, CI completa, Docker/Compose, bridge real, base de datos, corpus real, retrieval/mediciones, inferencia, aceptación QA y archive.

### Matriz de conformidad — Fase 17/U11

| Requisito / escenario | Evidencia ejecutada o inspeccionada | Resultado |
|---|---|---|
| 17.1: CLI/piloto mantiene annotations requeridas e informe descriptivo | `PilotCompatibilityTests.test_cli_preserves_annotation_requirement_and_descriptive_pilot_report`; confirma que `run_evaluation` recibe CSV y su informe descriptivo se escribe como salida. | ✅ Conforme |
| 17.1: error al escribir informe | `PilotCompatibilityTests.test_cli_write_oserror_returns_two_without_claiming_persistence`; `Path.write_text` simula `OSError`, `main` retorna 2 y emite error. No se afirma persistencia ni atomicidad. | ✅ Conforme |
| 17.1: `protocol_preflight` sigue diagnóstico sin efectos/PASS | `PilotCompatibilityTests.test_protocol_preflight_remains_diagnostic_without_pass`; el bridge espía rechaza llamadas, `measurement_started=false`, no hay `PASS`. | ✅ Conforme |
| U11: PASS solo con criterios por consulta y procedencia completa; relevancia no evaluada, sin aceptación global | `ProtocolVerdictAndProvenanceTests.test_positive_synthetic_pass_both_metrics_perfect_and_provenance_verified`; suite completa 148/148. Los valores positivos son fixtures/dobles sintéticos. | ✅ Conforme técnicamente; no es evidencia de producto |
| U11: orden distinto falla con recall completo; exact vacío bloquea y métricas quedan null | `test_different_order_full_recall_produces_fail`; `test_empty_exact_ranking_produces_null_metrics_and_blocked_gate`. | ✅ Conforme |
| U11: underfill, scores empatados y macros descriptivos | `test_underfill_and_scores_do_not_override_sequential_ranks`; `test_macro_descriptive_averages_without_rounding_and_requires_all_defined`. | ✅ Conforme |
| U11: evidencia final adversa, identidad/run/query no correlacionados o cobertura incompleta bloquean | `test_adverse_final_provenance_blocks_gate_even_with_perfect_metrics`; `test_incomplete_per_query_evidence_coverage_blocks_with_perfect_metrics`; bloqueo con métricas perfectas. | ✅ Conforme |
| U11: hashes de protocolo/corpus se revalidan antes del veredicto | `test_protocol_or_corpus_hash_mutation_before_verdict_blocks_gate`. | ✅ Conforme |
| 17.2: discovery, contrato Python/schema, lint y diff | Discovery 148 tests; ambos JSON validados contra schema; `py_compile`, Markdownlint y `git diff --check`, todos exit 0. | ✅ Conforme |
| 17.2: superficies prohibidas sin delta nuevo de Fase 17 | `apply-progress.md` registra revisión focalizada. El status actual contiene WIP previo en Rust, fixtures, state e informes; no se atribuye el diff total a esta fase. El cierre Fase 17 está acotado a `PilotCompatibilityTests` y su registro en `apply-progress.md`; no hay evidencia de cambios nuevos de Fase 17 en Rust, Docker, C0, corpus/candidatos, plantilla o resultados. | ✅ Conforme según evidencia focalizada; la atribución histórica se limita al registro de apply |
| 17.3: evidencia nueva sin alterar reportes históricos ni estado | Se inspeccionaron las secciones nuevas de `apply-progress.md`; este informe se actualizó por append. `qa-report.md` conserva QA11 BLOCKED y `state.yaml` no se alteró en esta verificación. | ✅ Conforme; reportes históricos preservados |

**Resumen de conformidad**: las tareas 17.1–17.3 y los escenarios U11 revisados tienen evidencia runtime o checks de artefacto aprobados. Los resultados sintéticos no son medición ni aceptación real.

### Coherencia con diseño

| Decisión | Resultado | Evidencia / límite |
|---|---|---|
| API protocol-aware separada del piloto | ✅ Seguida | La caracterización confirma que CLI y `run_evaluation` conservan annotations y el informe descriptivo; `protocol_preflight` no activa ejecución. |
| Preflight fail-closed y evidencia final separada | ✅ Seguida en tests | Dobles cubren rechazo previo y requisitos posteriores; no se usó bridge operacional. |
| Reutilizar métricas, sin reimplementar fórmulas | ✅ Seguida | Casos U11 ejercitan serialización por API runner; suite completa pasó. |
| Relevancia absoluta y aceptación global fuera del slice | ✅ Seguida | Tests exigen `not_evaluated` y ausencia de aceptación global. |
| Límites operativos e historial | ✅ Seguidos | No se corrieron operaciones prohibidas. QA/verify anteriores permanecen y este informe se añadió al final. |

### Auditoría TDD

| Métrica | Estado |
|---|---|
| Aplicabilidad | Sí para regresiones/comportamiento 17.1 y escenarios U11; checks de schema/documentación se validan como artefactos. |
| RED→GREEN→REFACTOR de Fase 17 | ✅ Registro en `apply-progress.md`: RED inicial por `SystemExit: 2` no manejado por el test; test ajustado para capturar la interfaz argparse; después 3 pruebas `OK`. No cambió código de producto en esta fase. |
| Evidencia TDD U11 | ✅ Registro de RED (8 fallas antes de integrar veredicto), GREEN (9 tests) y suites posteriores en evidencia de Fase 16; pruebas actuales pasan en discovery fresco. |
| Limitación | El orden TDD se apoya en el registro de apply; la ejecución fresca prueba el estado actual, no reconstruye el orden histórico de escritura. |

### Hallazgos y riesgos

**CRITICAL**: Ninguno para el alcance offline revisado.

**WARNING**:

- No se hizo build ni Clippy por el límite de Fase 17. El fallo histórico de Clippy `-D warnings` permanece abierto y no se afirma resuelto.
- QA independiente continúa `BLOCKED`; B.1/B.2, freeze/review real, procedencia operacional, medición y aceptación H5 siguen bloqueados.
- El repositorio contiene WIP previo en varios paths. La evidencia focalizada atribuye las regresiones Fase 17 a la clase de tests y al registro de apply; el `git status` completo no representa el delta exclusivo de esta fase.
- No hay runner de calidad SDD configurado ni umbral de cobertura. No se reporta un porcentaje de cobertura.

### Veredicto

**PASS WITH WARNINGS — slice offline hasta U11/Fase 17.** La ejecución demuestra las regresiones del piloto y los criterios sintéticos revisados. Este resultado no evalúa QA de aceptación ni protocolo/corpus/procedencia real, no elimina bloqueos históricos y no autoriza mediciones. No archivar.
