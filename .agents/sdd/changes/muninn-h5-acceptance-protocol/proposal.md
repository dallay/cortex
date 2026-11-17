# Proposal: H5 Acceptance Protocol

## Intent

Define a candidate H5 acceptance protocol that keeps ANN–exact equivalence separate from absolute retrieval relevance. Record the user-approved ANN–exact criteria and run treatment, and define a new, independent, unlabelled corpus candidate for human review. This proposal does not claim product readiness, protocol approval for measurement, or acceptance of any prior result.

The existing six-query/twelve-candidate pilot and its hypothetical C0/C1 cases are excluded from the candidate corpus and from retrospective acceptance. No measurement is authorized by this change.

## Scope

### In Scope

- Record a candidate ANN–exact gate at `K=10`: for every query, require `ExactTopKRecall@10=1.0` and `OrderedTopKMatch@10=1.0` for `PASS`.
- Record one run, per-query metric reporting, and macro-averages as descriptive summaries only. Do not use macro-averages to override a failed per-query criterion.
- Do not break ties retrospectively. An undefined required metric blocks `PASS`.
- Keep absolute relevance disabled. No product utility threshold has been approved; any future activation requires a separately approved threshold and human labels.
- Define a separate, versioned and frozen, unlabelled corpus candidate assembled from eligible project material for independent human review. Its proposed artifact contains query text, candidate text, source-path provenance, and a corpus hash. Keep the corpus in draft status: do not approve it, label it, or treat it as sufficient or representative.
- Require user review and an explicit freeze decision for the corpus. Keep human labeling and adjudication as separate, later steps; the candidate process is two blind reviewers followed by adjudication.
- Keep the candidate protocol non-ready until the exact frozen corpus is identified, applicable labels exist if absolute relevance is later enabled, encoder and provenance are verified, commit and review metadata are recorded, and all applicable thresholds and approvals are complete.
- Preserve the existing descriptive-only status of prior results and existing QA/Clippy blockers. Do not perform measurements or claim acceptance.

### Out of Scope

- Creating or labeling the corpus, freezing it, deciding its sufficiency or representativeness, or performing human review/adjudication in this proposal phase.
- Enabling absolute relevance or selecting a product utility threshold, relevance rubric, or acceptance criteria for that gate.
- Running measurements, changing retrieval behavior, or treating existing pilot or hypothetical C0/C1 cases as acceptance evidence.
- Claiming readiness, product acceptance, or confirmation that the candidate corpus has adequate size or coverage.
- Modifying code, tests, configuration, reports, SDD specs/design/tasks/state, or any file other than this proposal.

## Approach

Represent the candidate protocol and corpus as reviewable, versioned artifacts in later change phases. The protocol records the approved ANN–exact criteria and run handling, while retaining explicit gates for all inputs and approvals that remain applicable. Required per-query metrics must be defined for every query; an undefined value prevents `PASS`. Ties are evaluated as observed and are not retrospectively resolved to improve agreement. The macro-average is descriptive, not an alternative acceptance path.

The corpus candidate is a new independent collection of unlabelled query/candidate text drawn from eligible, versioned Markdown material under `docs/muninn/`. Record source paths and a hash of the corpus contents. Exclude `docs/muninn/retrieval-h5-runner.md` because it has a pre-existing modification. The available inventory is 29 versioned documents; this inventory does not establish that the candidate is large or representative enough for `K=10`. Do not reuse the existing six-query/twelve-candidate pilot or hypothetical C0/C1 cases.

The user must review the draft and explicitly approve its frozen version before it can serve as a protocol input. Keep labels and adjudication out of this draft corpus artifact. If absolute relevance is later proposed, require a separately approved utility threshold, human labels, and adjudication before enabling that gate. The candidate protocol remains not ready until the exact corpus, required labels (if applicable), encoder/provenance, commit and review metadata, thresholds, and all applicable approvals are recorded. This is a definition of candidate gates, not a readiness determination or authorization to measure.

## Affected Areas

| Area                                                            | Impact                     | Description                                                                                                                                          |
|-----------------------------------------------------------------|----------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------|
| `.agents/sdd/changes/muninn-h5-acceptance-protocol/proposal.md` | Modified                   | Record the candidate protocol decisions, corpus-review boundary, exclusions, and non-readiness conditions.                                           |
| Future protocol and corpus artifacts for this change            | Proposed, not created here | Define a versioned protocol candidate and a draft unlabelled corpus artifact with query/candidate text, source-path provenance, and corpus hash.     |
| `docs/muninn/`                                                  | Read-only source material  | Candidate corpus preparation may use eligible versioned Markdown; exclude the modified runner guide. No source document is changed by this proposal. |
| Existing H5 pilot/results and runner                            | No change                  | Preserve them as descriptive evidence only; they are not the candidate corpus or acceptance evidence.                                                |

## Risks

| Risk                                                                           | Mitigation                                                                                                              |
|--------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------|
| A small or unrepresentative corpus is mistaken for adequate K=10 coverage.     | Keep the corpus unapproved and draft until user review; make no sufficiency or representativeness claim.                |
| Existing pilot content or hypothetical cases leak into the independent corpus. | Explicitly exclude the six-query/twelve-candidate pilot and C0/C1 cases.                                                |
| Descriptive aggregation obscures per-query failures.                           | Require both ANN–exact metrics to equal 1.0 for every query; label macro-averages descriptive only.                     |
| Missing values, unresolved ties, or missing provenance are treated as success. | Do not break ties retrospectively; block `PASS` on undefined required metrics or incomplete applicable protocol inputs. |
| A relevance gate is enabled without an approved product utility threshold.     | Keep absolute relevance disabled until a threshold, labels, and adjudication are separately approved.                   |

## Rollback Plan

This phase changes only this proposal. If the candidate decisions or corpus boundary change, revise or withdraw this proposal with explicit user direction before later phases use it. No corpus, code, configuration, measurements, or runtime state is created or changed here.

## Dependencies

- Explicit user review and approval are required to freeze the corpus candidate.
- Any later absolute-relevance gate depends on a separately approved product utility threshold, labels, and adjudication.
- Later protocol readiness depends on the exact corpus version, encoder/provenance, commit and review metadata, applicable thresholds, and approvals.

## Success Criteria

- [ ] The proposal records `K=10`, both per-query ANN–exact metrics at `1.0`, one run, descriptive per-query and macro reporting, no retrospective tie-breaking, and undefined-required-metric blocking.
- [ ] Absolute relevance remains disabled because no product utility threshold is approved.
- [ ] The corpus candidate is defined as new, independent, versioned, frozen only after user review, and unlabelled at draft stage, with query/candidate text, source-path provenance, and corpus hash.
- [ ] Two blind reviewers and subsequent adjudication are specified as the candidate labeling process, separate from corpus preparation and freeze.
- [ ] The existing pilot and hypothetical C0/C1 cases are excluded, and no claim is made that the available material is sufficient or representative for `K=10`.
- [ ] The protocol is explicitly not ready until all exact corpus, applicable labels, encoder/provenance, commit/review metadata, threshold, and approval inputs are present.
- [ ] No corpus is created or labeled, no measurement is run, and no readiness or product-acceptance claim is made in this phase.

## Enmienda autorizada: slice técnico offline ANN-only

### Autorización y continuidad

El usuario autorizó explícitamente incorporar al SDD existente un slice técnico offline que conecte el protocolo con un flujo ANN–exact sin annotations cuando `gates.ann_exact=true` y `gates.absolute_relevance=false`. Esta enmienda amplía el alcance de las fases posteriores; no autoriza una medición real, un protocolo-run ni la aceptación de H5. En esta fase solo se modifica `proposal.md` y se preserva el WIP previo.

La ampliación reabre `spec -> design -> tasks -> apply` con TDD para el comportamiento ejecutable. Prevalece sobre las exclusiones anteriores solo en cuanto al cambio técnico futuro del runner y sus pruebas. No modifica ahora esos artefactos ni `state.yaml`: el estado registrado sigue siendo `qa / blocked / next: qa`. `qa-report.md` y `verify-report.md` conservan su evidencia histórica; no verifican ni aceptan este slice nuevo. Los blockers anteriores, incluido Clippy con `-D warnings`, permanecen abiertos. No aceptar H5 ni archivar el cambio.

### Alcance acotado para las fases posteriores

- Añadir un flujo protocol-aware ANN–exact que no lea, requiera ni fabrique annotations, CSV, etiquetas, rúbrica o adjudicación cuando `absolute_relevance=false`. Ese gate permanece `not_evaluated`; no recibe métricas de relevancia ni `PASS` y no se deriva aceptación global de H5.
- Ejecutar un preflight local fail-closed antes de cualquier interacción con el bridge, inserción de candidatos o creación/escritura de artefactos de medición. Bloquear ante protocolo ausente, inválido, pendiente, no aprobado/corroborado, hash incoherente, corpus/scope incompatible o contrato ejecutable no soportado. No usar la ruta piloto como fallback para eludir el bloqueo.
- Mantener coherentes el hash canónico del protocolo completo y su aprobación corroborada, el corpus/hash y los IDs de consultas/candidatos, la configuración aprobada, la identidad esperada y la procedencia externa aplicable. Correlacionar después la identidad allowlisted de cada respuesta con esa misma consulta/run y evidencia externa; ausencia o discrepancia bloquea `PASS`. El self-report no sustituye evidencia independiente ni constituye atestación.
- Separar en spec/design los requisitos verificables antes de interactuar de la evidencia que solo existe después de responder el bridge. El preflight no puede fabricar resultados, identidad observada, `finished_at` ni evidencia de finalización para declararse elegible. La comprobación posterior no permite omitir la autorización previa ni aceptar procedencia `partial`/`insufficient`.
- Reutilizar las métricas y helpers existentes; no rehacer sus fórmulas, serialización, validación o evaluación de procedencia. Conservar `K=10`, ambas métricas iguales a `1.0` por consulta, una corrida, macro-promedios solo descriptivos, ranking observado sin desempate retrospectivo e indefinidos requeridos bloqueando `PASS`.
- Probar el flujo mediante bridge falso y fixtures sintéticas independientes del corpus candidato y del piloto. Aplicar Red -> Green -> Refactor en la fase apply: camino ANN-only sin annotations, bloqueo previo con cero llamadas/escrituras, discrepancias de hashes/IDs/identidad/procedencia, criterio incumplido o indefinido, contrato no soportado y no regresión descriptiva. Una prueba positiva sintética demuestra el contrato técnico, no aprobación ni aceptación de datos reales.
- Preservar la invocación y los informes del piloto descriptivo actual, sus annotations y su clasificación histórica. La nueva ruta no reclasifica resultados ni modifica el piloto, C0 o product retrieval.

### Contrato mínimo pendiente de spec/design

`acceptance_rules_and_tolerances.value.ann_exact` contiene hoy `criterion` en prosa y `threshold` numérico; `run_count_and_aggregation.value` contiene políticas en texto. La validación estructural no ejecuta esas reglas. No inferir comparadores, métricas, conjunción por consulta, agregación, desempates ni tratamiento de indefinidos a partir de texto libre, ni ejecutar prosa como código.

Antes de apply, spec/design deben fijar un contrato explícito mínimo y cerrado para reconocer únicamente la política autorizada: las dos métricas ANN–exact y su K, igualdad de ambas al umbral aprobado por cada consulta, una corrida, resumen macro no gobernante, ranking observado e indefinidos bloqueantes. Reutilizar campos existentes cuando sea posible y definir cómo se reconoce inequívocamente cada política soportada; rechazar toda variante desconocida o contradictoria antes de interactuar. Si la estructura actual no puede expresarlo sin ambigüedad, documentar el cambio mínimo de contrato y su compatibilidad en spec/design; no inventarlo en implementación ni ampliar a un motor genérico de reglas. Ningún valor implícito sustituye un campo pendiente o una aprobación.

### Áreas afectadas previstas y reversión

- `dev/muninn/retrieval_h5_runner.py`: integración futura de la ruta protocol-aware, preflight y reporte ANN-only; mantener separado el piloto descriptivo existente.
- `dev/muninn/tests/test_retrieval_h5_runner.py`: pruebas futuras con bridge falso, fixtures sintéticas y aserciones de cero interacciones/escrituras cuando se bloquea.
- `dev/muninn/retrieval_h5_protocol.py`, `dev/muninn/fixtures/retrieval-h5-protocol.schema.json` y `dev/muninn/tests/test_retrieval_h5_protocol.py`: solo si spec/design justifican ajustes mínimos para expresar/validar el contrato soportado y su coherencia. No rehacer helpers ni completar el candidato con aprobaciones ficticias.
- `docs/muninn/retrieval-h5-runner.md`: documentación futura del flujo y sus límites, preservando su WIP previo. `retrieval_quality.py` y `retrieval_h5_provenance.py` se reutilizan; no se propone reescribirlos.

La reversión actual retira únicamente esta enmienda mediante edición selectiva, sin revertir el WIP anterior. La reversión del slice futuro retira solo su integración y pruebas específicas, manteniendo el piloto descriptivo, las métricas/helpers y toda evidencia histórica; no borra corpus, informes ni datos persistentes.

### Exclusiones y criterios de éxito adicionales

No ejecutar builds, tests ni mediciones en esta fase. El slice posterior se verifica solo offline con dobles: sin modelos/inferencia real, Docker, bridge vivo, base de datos real ni corpus aprobado. No congelar/aprobar el corpus candidato, resolver C0, activar relevancia absoluta, aceptar H5 ni archivar. Las autorizaciones operativas y el freeze/review real siguen pendientes.

- [ ] Spec/design resuelven el contrato ejecutable mínimo sin interpretar prosa libre y tasks define su implementación TDD acotada.
- [ ] Las pruebas sintéticas del slice posterior demuestran ANN-only sin annotations y fail-closed antes de cualquier interacción/escritura de medición, sin debilitar las comprobaciones posteriores de identidad/procedencia/hash.
- [ ] El piloto continúa descriptivo; la evidencia histórica y los blockers permanecen intactos. La corrección técnica offline no se presenta como readiness, autorización de medición ni aceptación H5.
