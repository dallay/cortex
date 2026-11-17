# Tareas: protocolo de aceptación H5

## Vigencia del plan ANN-only

Las fases 12–17 contienen únicamente pendientes nuevos del slice técnico offline aprobado y prevalecen para ese alcance. Las fases 1–11, sus casillas completas, WIP y evidencias se conservan como historial; no verifican este slice. B.1/B.2, QA `BLOCKED`, aceptación H5 y Clippy siguen bloqueados independientemente. Las reglas candidatas ya registradas no aprueban un protocolo-run real.

## Previsión de carga de revisión

| Campo | Valor |
|---|---|
| Presupuesto | 400 líneas modificadas (adiciones + eliminaciones) |
| Carga estimada | Alta; framework existente más métricas y pruebas, probable >400 líneas |
| PR encadenadas recomendadas | No |
| Estrategia | `size-exception`, aprobada explícitamente por el usuario |
| Balance | Unidades por comportamiento: (1) contrato/aprobación; (2) procedencia/evaluación; (3) integración/informe; (4) métricas ANN–exact y fórmulas con pruebas; (5) identidad del encoder, respuesta diagnóstica y correlación estricta del runner; (6) envoltura pending/approved, schema y validador estructural protocol-run. |

La excepción de tamaño y permanencia en la rama `muninn` siguen aprobadas: la rama reúne varios hitos y se probará antes de integrar a `main`. No crear ramas ni PR encadenadas. Previsión: 45–60 minutos para revisar la unidad 4; 2–3 horas para el diff acumulado, según tamaño final.

Decision needed before apply: No
Chained PRs recommended: No
400-line budget risk: High

## Fases 1–7: framework original (completadas)

- [x] 1.1–1.4 Contrato estricto/versionado, hash, campos por gate, validación y pruebas en `retrieval_h5_protocol.py`.
- [x] 2.1–2.3 Aprobación previa y corroboración local del operador, sin claims criptográficos.
- [x] 3.1–3.3 Colector de procedencia por gate, observaciones locales, estados parciales/insuficientes y redacción; pruebas.
- [x] 4.1–4.3 Evaluación independiente y elegibilidad por gate, sin PASS global; pruebas de indefinidos.
- [x] 5.1–5.5 Preflight y runner antes de bridge/escritura/medición; pruebas, sin cambiar retrieval ni Store.
- [x] 6.1–6.3 Informe por gate y compatibilidad histórica descriptiva; pruebas de no reclasificación.
- [x] 7.1–7.4 Plantilla y documentación; pruebas focalizadas; sin medición confirmatoria ni cambios de API/crates/Store/datos.

## Fase 8: métricas aprobadas (unidad 4; depende de Fases 1–7)

- [x] 8.1 RED: En `dev/muninn/tests/test_retrieval_quality.py`, probar métricas por consulta: mismos IDs en orden distinto dan recall de conjunto completo y coincidencia ordenada 0; no comparar scores.
- [x] 8.2 RED: Probar top-K exacto vacío => ambas indefinidas; underfill usa elementos disponibles y comparación ordenada incluye longitud; cubrir duplicados e inputs inválidos según validadores existentes.
- [x] 8.3 GREEN: Añadir funciones puras `ExactTopKRecall@K` y `OrderedTopKMatch@K` en `dev/muninn/retrieval_quality.py`; K se recibe como parámetro, sin default/constante.
- [x] 8.4 REFACTOR: Alinear validación/nombres con convenciones existentes; repetir pruebas, sin lógica de scores.
- [x] 8.5 RED: Probar informe por gate con las dos métricas, `Recall@K`, `nDCG@K` y `MRR` descriptivo; gates independientes, sin PASS causado por métricas y sin cambios en informes históricos.
- [x] 8.6 GREEN: Integrar resultados por consulta en el gate correspondiente de `dev/muninn/retrieval_h5_runner.py`; mantener métricas como datos, no reglas PASS.
- [x] 8.7 REFACTOR: Repetir pruebas de informe y confirmar contenido/clasificación histórica intactos.
- [x] 8.8 Caracterización: En `test_retrieval_quality.py`, comprobar Recall basado en relevancia de rúbrica aprobada y nDCG con gain `2^grade-1`, descuento `log2(rank+1)`; no adoptar corpus/rúbrica piloto automáticamente.
- [x] 8.9 Si 8.8 detecta discrepancia: RED con prueba focalizada, GREEN con ajuste mínimo en `retrieval_quality.py`, REFACTOR con pruebas verdes. Si coincide, conservar fórmula y baseline sin cambio funcional.
- [x] 8.10 Ejecutar pruebas focalizadas; no medir confirmatoriamente ni fijar K, umbrales/PASS, agregación, desempates o política final de indefinidos.

## Bloqueo explícito de medición confirmatoria

- [ ] B.1 BLOQUEADA hasta completar y aprobar todos los campos aplicables del protocolo-run concreto: K numérico, reglas/tolerancias, corridas/agregación, desempates, tratamiento final de indefinidos, corpus/rúbrica/adjudicación, alcance e identidad/procedencia.
- [ ] B.2 BLOQUEADA: sin re-medición confirmatoria ni PASS hasta completar/aprobar B.1 y obtener procedencia verificable `verified` para cada gate. Esto no bloquea el framework. No inferir valores del piloto, H4 `1e-9` ni referencia Proposed.

## Fase 9: identidad verificable del encoder H5 (nueva unidad de comportamiento)

Unidad 5: respuesta de `/h5/diagnostic/search` correlacionada con el encoder usado y consumo estricto por el runner. Mantener la excepción de tamaño aprobada para la rama `muninn`; no crear ramas/PR ni commit. No ejecutar Docker/bridge en vivo ni medición confirmatoria.

- [x] 9.1 RED: En pruebas del encoder, fijar vectores conocidos del hash SHA-256 enmarcado (etiqueta y longitud de `modules.json` y `1_Pooling/config.json`); probar identidad declarada (bundle, modelo/revisión pinned, `space_id`), ID estable del perfil validado, mismatch y error de carga. Probar que se usan solo buffers ya verificados/validados y que reporte no relee archivos ni revela paths.
- [x] 9.2 GREEN: Retener en `CandidateCpuSentenceEncoder::load` la identidad allowlisted y fingerprint calculado de esos mismos buffers; retener modelo/revisión pinned y perfil validado. Implementar solo lo necesario en `minilm_encoder.rs` y `embedding_port.rs`.
- [x] 9.3 REFACTOR: Reducir duplicación sin ampliar claims; repetir pruebas de vectores, mismatch/error y no reread/path disclosure.
- [x] 9.4 RED: Con encoder falso que diferencie instancia/identidad, probar en `main.rs` que la respuesta diagnóstica obtiene identidad de la misma invocación `App.encoder` que produce ANN/exact; probar allowlist estricta, ausencia de secretos/vectores/contenido, rechazo sin auth y ausencia de llamadas adicionales a retrieval/Store.
- [x] 9.5 GREEN: Añadir `encoder_identity` a la respuesta existente autenticada detrás de `h5-diagnostic`; conservar alias, ANN/exact, validación y flujo de búsqueda sin nueva ruta ni observación aparte.
- [x] 9.6 REFACTOR: Repetir pruebas del handler/serialización y compatibilidad de alias/ANN/exact; confirmar exactamente el uso existente de búsqueda y Store.
- [x] 9.7 RED: En `test_retrieval_h5_provenance.py` y `test_retrieval_h5_runner.py`, probar forma exacta aceptada, correlación de identidad esperada con evidencia Docker/container/run y vínculo con la respuesta de búsqueda; cubrir campo faltante/desconocido, mismatch, caller substitution y ejecución no identificable: `partial`/`insufficient`, nunca PASS. Probar una sola llamada existente al bridge.
- [x] 9.8 GREEN: Implementar sanitizer estricto y correlación en `retrieval_h5_provenance.py`/`retrieval_h5_runner.py`; conservar aliases, ANN/exact y runner descriptivo normal. Mantener fallos o falta de evidencia como no elegibles para PASS.
- [x] 9.9 REFACTOR: Repetir tests H5 y compatibilidad descriptiva; confirmar sin llamada adicional al bridge ni acceso adicional a Store.
- [x] 9.10 Verificar feature contract mediante registro productivo directamente bajo `#[cfg(feature = "h5-diagnostic")]`, tests/build lib y bin con y sin feature, auth test del handler gated, `cargo fmt --all -- --check`, tests Python H5 y lint documental; ejecutar Clippy con `-D warnings` y registrar su resultado sin reparar lints ajenos al alcance. No iniciar Docker/bridge para inspección live.

  Evidencia y límites en `apply-progress.md`: builds/tests de ambas configuraciones y demás checks se repitieron. El código registra la ruta real bajo `#[cfg(feature = "h5-diagnostic")]`; los tests del bin pasan con y sin feature, y el test HTTP feature-gated confirma rechazo sin autenticación. No se inspeccionó in-process la tabla del router productivo ni se inició un servicio. Clippy con `-D warnings` falla por lints existentes del crate; el filtro focalizado no mostró diagnósticos en las nuevas funciones de identidad ni en `main.rs`. No se relajaron lints.
- [x] 9.11 Actualizar documentación técnica pertinente de bridge/runner para identidad self-report, alcance del hash, feature gate, correlación y límites; no modificar informes descriptivos ni afirmar attestation. Validar lint documental aplicable.

### Bloqueo de calidad preexistente

- Clippy con `-D warnings` se ejecutó y falla por numerosos lints en el sub-workspace bridge, principalmente módulos fuera de esta unidad. No se debilitaron lints ni se ampliaron cambios para corregir deuda no relacionada. El resultado queda como riesgo de integración; no se afirma que Clippy pase.

## Fase 10: plantilla y validador estructural protocol-run (nueva unidad 6)

La plantilla conserva decisiones de producto pendientes; esta unidad no completa ni ejecuta un protocolo-run. QA sigue `BLOCKED`; B.1/B.2 y el bloqueo de Clippy permanecen abiertos.

- [x] 10.1 RED: En `dev/muninn/tests/test_retrieval_h5_protocol.py`, probar envoltura cerrada `status` (`pending`/`approved`), `value` y `rationale`; `pending` requiere `value: null` sin defaults y `approved` requiere rationale no vacío y valor no vacío con tipo/estructura específica por ruta. Un claim `approved` en JSON no constituye aprobación; probar que `corroborate_approval` sigue exigiendo readiness, hash del protocolo completo y corroboración del operador.
- [x] 10.2 GREEN: Implementar la validación de envoltura y valores específicos por ruta en `dev/muninn/retrieval_h5_protocol.py`; mantener separadas aprobación declarada y corroborada.
- [x] 10.3 REFACTOR: Alinear helpers sin relajar el contrato; repetir pruebas de envoltura, rutas y corroboración.
- [x] 10.4 RED: Añadir pruebas de sincronía Python/JSON Schema y fixture: campos de decisión no aceptan `{}`, objetos anidados cierran `additionalProperties`, `required`/tipos y condicionales reflejan la aplicabilidad por gate; fixture queda `pending`/`null` y no confirmatory-ready. Verificar que métricas aprobadas no implican K aprobado.
- [x] 10.5 GREEN: Endurecer `dev/muninn/fixtures/retrieval-h5-protocol.schema.json` y `dev/muninn/fixtures/retrieval-h5-protocol.example.json` sin completar valores de producto ni convertir validación en decisión de gate.
- [x] 10.6 REFACTOR: Reducir divergencia entre schema, fixture y Python; repetir pruebas cruzadas y confirmar gates condicionales independientes.
- [x] 10.7 RED: Probar en `test_retrieval_h5_protocol.py` rutas `missing`/`pending`/`malformed`; rechazar objeto/lista vacíos, tipo incorrecto, placeholder escalar y clave anidada desconocida. Fijar determinismo de reporte/hash y bloqueo de readiness ante cualquier finding.
- [x] 10.8 GREEN: Implementar hallazgos deterministas por JSON path y fail-closed en `validate_protocol`; conservar `missing_fields` si es compatible, sin perder hallazgos nuevos. Mantener comprobaciones condicionales por gate y hacer que `corroborate_approval` requiera readiness más corroboración del operador.
- [x] 10.9 REFACTOR: Asegurar errores de salida seguros, sin secretos; no añadir PASS ni decisión de gate. Repetir pruebas de validador y compatibilidad del runner/schema, modificando consumidores solo si es imprescindible.
- [x] 10.10 Verificar suites de protocolo y H5 runner, comportamiento válido/inválido/pending del JSON Schema y fixture, `py_compile`, markdownlint aplicable y `git diff --check`. No ejecutar Docker, bridge ni medición confirmatoria. Registrar Clippy `-D warnings` como bloqueo existente, sin debilitarlo ni afirmar aceptación QA.
- [x] 10.11 Corregir únicamente los findings confirmados de protocol-run: timestamp RFC3339 con zona horaria y sentinel pending, corroboración fail-closed para datetimes naive/aware, `approved_gates` JSON no hashables y allowlist de K sincronizada por combinación de gates; añadir regresiones Python/schema y repetir los checks de 10.10 salvo Clippy.
- [x] 10.12 RED: Añadir pruebas Python/JSON Schema que demuestren que `acceptance_rules_and_tolerances.value` requiere reglas solo para gates habilitados y rechaza reglas de gates deshabilitados. Preservar el comportamiento existente cuando ambos gates están deshabilitados; no exigir que uno esté habilitado.
- [x] 10.13 GREEN/REFACTOR: Sincronizar en Python y JSON Schema la allowlist de reglas aplicables por gate; repetir protocolo, runner y H5 suites. No modificar reglas reales ni elegir umbrales.

## Phase 11: Protocol-run candidate and independent unlabelled corpus (new work unit)

This unit creates review-only draft artifacts. It does not approve or freeze the corpus, authorize measurement, establish corpus adequacy or representativeness, or change QA status. Keep all earlier phases and blockers unchanged. QA remains `BLOCKED`; do not archive this change.

- [x] 11.1 Create `dev/muninn/fixtures/retrieval-h5-protocol.candidate.json` as a separate review-only candidate using the existing closed protocol structure. Record `K=10`, `ann_exact=true`, and require both `ExactTopKRecall@10=1.0` and `OrderedTopKMatch@10=1.0` for every query; record one run, per-query reporting, descriptive-only macro-average, observed ties without retrospective tie-breaking, and undefined required metrics blocking `PASS`. Keep `absolute_relevance=false`; leave encoder, corpus/freeze, provenance, review and approval metadata unresolved. The candidate must remain not ready and must not authorize a run.
- [x] 11.2 Inventory eligible sources only from the immutable tracked Markdown baseline at commit `09bb21d5fd2c847ac7df5e48f7be06eb08cec445`. Exclude modified `docs/muninn/retrieval-h5-runner.md`, all H5 pilot documents/data/results, C0/C1 hypothetical labels, and untracked or generated material. Record the eligible source allowlist and any coverage blocker in the corpus provenance/inventory artifact; do not modify source documents.
- [x] 11.3 Draft a new independent, versioned, unlabelled corpus only if every proposed query has at least 10 eligible candidates. Create `dev/muninn/fixtures/retrieval-h5-corpus-candidate.json` in the exact `validate_corpus` shape, with independent query wording and stable opaque IDs; create the separate strict `dev/muninn/fixtures/retrieval-h5-corpus-candidate.provenance.json` with `draft/not frozen` status, baseline commit, exact source paths and source-byte SHA-256 values, ID-to-source mappings, and canonical corpus hash. Include no labels, grades, scores, rankings, relevance judgments, or adequacy claims.
- [x] 11.4 If any query cannot meet the 10-candidate minimum from eligible sources, STOP corpus construction: write only a blocker/inventory note in the designated provenance/inventory artifact. Do not write a partial corpus, pad candidate sets, or describe an incomplete artifact as ready.
- [x] 11.5 Verify every source path against the baseline allowlist and verify source hashes, root MIT license applicability, and absence of sensitive text. Record that project sensitivity classification is absent and that owner manual review of source paths and text remains required. Do not infer license or sensitivity approval from these checks.
- [x] 11.6 Add or run offline deterministic artifact checks only: validate any complete corpus with `dev/muninn/retrieval_quality.py::validate_corpus`, recompute and compare its canonical `corpus_hash`, and check manifest closure, unique IDs, source-path/hash consistency, exclusions, and minimum candidate count. If the corpus is blocked, validate the inventory/blocker note instead. No retrieval, model, Docker, bridge, database, or measurement execution; no artificial TDD for data/document-only work.
- [x] 11.7 Update `.agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md` with artifact paths, inventory and validation evidence, unresolved owner review/freeze and provenance details, and explicit non-readiness. Preserve QA as `BLOCKED` and do not archive. Do not modify source docs or `dev/muninn/retrieval_h5_runner.py`.

### Phase 11 review boundary and acceptance guard

The deliverables are the protocol candidate plus either a complete corpus and strict provenance manifest or only a blocker/inventory note. Keep the review as one coherent, data-and-artifact work unit; retain the existing approved `size-exception` and do not create branches or PR chains. The prior 400-line budget risk remains High. Decision needed before apply: No. Chained PRs recommended: No. 400-line budget risk: High. No measurement is authorized, and corpus sufficiency/representativeness is unproven.

## Previsión nueva: integración offline ANN-only

- Presupuesto: 400 líneas modificadas, incluyendo tests/schema/docs. Riesgo alto por tres módulos, contrato y regresiones; previsión >400 líneas.
- Estrategia: `size-exception` histórica explícitamente aprobada para `muninn`; sin ramas, PR ni commits nuevos autorizados aquí. Su razón sigue siendo integrar y probar los hitos juntos antes de `main`.
- Dividir revisión/implementación en unidades dependientes: U7 contrato (12), U8 temporalidad (13), U9 preflight seguro (14), U10 ejecución aislada (15), U11 veredicto (16), cierre (17). Cada unidad incluye comportamiento, tests y documentación pertinente; termina verificable y admite reversión selectiva sin borrar WIP previo.
- Medir el diff de cada unidad en apply. Si supera 400 líneas, subdividirla por escenario completo con tests antes de continuar; nunca reducir validación, cobertura ni procedencia para cumplir el presupuesto. Si cambia el alcance de la excepción, detener y consultar.

Decision needed before apply: No
Chained PRs recommended: No
400-line budget risk: High

## Ejecución futura y límites

Todas las casillas siguientes están **PENDIENTES NUEVAS**. Dependencia: 12 → 13 → 14 → 15 → 16 → 17. Paths abreviados a continuación son relativos a `dev/muninn/`; la guía es `docs/muninn/retrieval-h5-runner.md`. Fixtures completas solo dentro de tests, independientes del piloto/candidato real. Apply registra comandos, salida y RED→GREEN→REFACTOR en `apply-progress.md`, sin reconstruir evidencia histórica.

Runner existente para apply/verify, no para esta fase:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover \
  -s dev/muninn/tests -p 'test_retrieval*.py'
```

En cada ciclo ejecutar primero el test focalizado con `unittest`, luego la suite pertinente; conservar `jsonschema` por subprocess y declarar bloqueo si falta. Sin modelos, Docker, bridge vivo, DB real, mediciones, C0, freeze/aprobación, aceptación ni archive.

## Fase 12 — U7: contrato v2 cerrado

- [x] 12.1 RED: En `tests/test_retrieval_h5_protocol.py`, contrastar Python/schema para v2: tokens exactos del diseño, K=10, threshold=1.0, run_count=1, hash y search_config; missing/pending/extras, booleanos, representación numérica de enteros, NaN/infinito y variantes inválidas. Caracterizar v1 en sus cuatro combinaciones.
- [x] 12.2 GREEN: En `retrieval_h5_protocol.py` y `fixtures/retrieval-h5-protocol.schema.json`, separar ramas cerradas v1/v2 reutilizando wrappers/hash/corroboración; sin defaults, interpretación de prosa ni migración/aprobación heredada.
- [x] 12.3 REFACTOR: Repetir contrato cruzado; probar que cambiar cualquier campo/versión invalida aprobación. Mantener plantilla y candidato v1 intactos; documentar compatibilidad en la guía.

## Fase 13 — U8: elegibilidad previa, procedencia final intacta

- [x] 13.1 RED: En `tests/test_retrieval_h5_provenance.py`, probar preflight sin evidencia futura, falta/conflicto de cada registro previo y rechazo de claims; conservar exigencia final de resultados, binding, identidad loaded, configuración completa y finished_at.
- [x] 13.2 GREEN: Añadir `assess_preflight_provenance` en `retrieval_h5_provenance.py`, compartiendo validación interna de registros; sin `verified` final, baseline público configurable ni finalización ficticia en `collect_observations`.
- [x] 13.3 REFACTOR: Repetir procedencia/correlación; preservar firma y clasificación de `assess_gate_provenance`. Documentar límite temporal y self-report, no attestation.

## Fase 14 — U9: autorización y scope antes de efectos

- [x] 14.1 RED: En `tests/test_retrieval_h5_runner.py`, usar fixtures v2 independientes (dos consultas, ≥10 candidatos cada una) y proveedor falso explícito; cubrir los cuatro registros cerrados y bindings enumerados abajo, campos ausentes/extra, tipos, placeholders, conflictos, mutación de inputs y dict plano rechazado. Cubrir protocolo ausente/inválido/pending/v1/gates no soportados, corroboración discordante, corpus alterado/missing/extra/duplicado, freeze draft/discrepante, inventario contaminado, aislamiento no acreditado, plan incompleto/duplicado, ID compartido contado por par, límite insuficiente y override inválido; admitir revisión de corpus distinta de la del protocolo y dominios numéricos U7 sin coerción.
  - Positivo: `preflight={status: "eligible", findings: []}`, ambos gates exactamente `{status: "not_evaluated", results: {}}`, `measurement_started=false`, sin evidencia futura. Negativo: preflight `blocked` con findings seguros no vacíos, ANN–exact exactamente `{status: "blocked", results: {}}`, relevancia exactamente `{status: "not_evaluated", results: {}}`, `measurement_started=false`.
  - En ambos caminos, spies exigen cero `Bridge.post`, `after_response`, `after_run`, métricas, lectura de annotations/helpers de relevancia, creación de directorios y escrituras; centinela intacto. `before_run` se invoca una vez tras validación/corroboración local, nunca ante rechazo local previo; recibe copias independientes y no deriva observaciones copiando expectativas.
- [x] 14.2 GREEN: En `retrieval_h5_runner.py`, añadir `run_protocol_evaluation` y `ProtocolRunEvidence` con firmas del diseño; reutilizar validadores/hash/corroboración y `assess_preflight_provenance` U8 junto con helpers privados puros especializados. Exigir `LocalObservations`, copiar entradas/observaciones y devolver únicamente el informe intermedio en memoria de 14.1.
  - Envolturas exactas: `value/source/method/observed_at`, solo `conflict` opcional; metadatos strings no blancos sin normalizar, conflicto booleano false o ausente. Valores cerrados recursivamente; IDs/referencias no placeholders, hashes `sha256:` + 64 hexadecimales minúsculos; sin nueva política temporal de `observed_at`.
  - `corpus_freeze_review.value`: exactamente `corpus_id/version/corpus_hash/status/review_reference`, status `frozen`; ID/versión/hash ligados al scope, ID/hash al corpus validado; revisión independiente de contenido/source paths, sin run ID ni alterar el corpus.
  - `storage_scope.value`: exactamente `run_id/bridge_container_id/storage_id/isolation/observed_candidates/planned_candidates`; storage opaco, aislamiento `run-exclusive-disposable-v1`, inventario global observado `[]`, plan no vacío de pares cerrados `query_id/candidate_id`, únicos e iguales a corpus/scope. Contar cada par, incluso ID compartido; total ≤ límite aprobado, sin ajuste ni limpieza; snapshot/token solos no acreditan exclusividad.
  - `effective_config.value`: exactamente `run_id/retrieval_path/candidate_scope/search_config`; ruta `/h5/diagnostic/search`, scope `same-query-candidates-v1`, configuración cerrada `limit/threshold` y override con dominios/valores aprobados U7, sin booleanos, no finitos ni configuración loaded fabricada.
  - `operator_corroboration.value`: exactamente `protocol_sha256/commit/review_reference/corroborated_at/corroborated_by`, igual al `operator_record`; reutilizar exclusivamente `corroborate_approval` y su temporalidad, sin ampliar el objeto.
  - Bindings conjuntos: hash completo calculado = `protocol_hash.value` = hash del operador; revisión observada/operador = revisión del protocolo y commit del operador = commit revisado; hash observado/scope/freeze = corpus calculado; identidad esperada validada = aprobada; run observado/storage/config = argumento; container storage = observación externa. No sustituir discrepancias por expectativas ni reducir requisitos U8/finales.
- [x] 14.3 REFACTOR: Repetir matriz y regresiones de U7/U8, piloto y `protocol_preflight`; conservar categorías/paths del diseño, findings `{path, reason}` seguros, ordenados y sin duplicados, y grupos estructurales `missing/pending/malformed`. Documentar API separada en `docs/muninn/retrieval-h5-runner.md`: U9 retorna sin efectos en éxito/fallo, sin `gate_result`, PASS, `verified` final ni ejecución; U10/U11 siguen pendientes. No añadir `output`, CLI, flag `preflight_only`, fallback piloto, evidencia futura ni implementar U10 anticipadamente.

Nota de avance fase 14: U9 implementa el port/API, los registros especializados y el preflight
sin efectos. Las matrices y regresiones quedan verificadas en `apply-progress.md`, incluida la
revisión formal independiente. El cierre se limita al contrato offline U9; no acepta H5 ni autoriza
la fase U10.

## Fase 15 — U10: ejecución sintética y bindings por consulta

- [x] 15.1 RED: En `tests/test_retrieval_h5_runner.py`, generar dos consultas con ≥10 candidatos cada una y fake independiente; candidate ID compartido, sesiones inequívocas, UUID duplicado, aliases/ranks extra/duplicados y respuesta inválida. Hacer fallar CSV/annotations/compose_report/evaluate_query si se invocan.
- [x] 15.2 GREEN: En `retrieval_h5_runner.py`, insertar todos los candidatos y efectuar exactamente una búsqueda por consulta con configuración exacta; mapa por `(query_id, candidate_id)`, sanitización y callbacks before_run/after_response/after_run del diseño, sin petición extra de identidad, reordenar ni reintentar.
- [x] 15.3 REFACTOR: Repetir flujo/bindings; probar excepciones tras primer efecto: blocked, cobertura parcial, measurement_started=true, sin rollback destructivo ni finalización inventada. Mantener informe en memoria y UUID/textos fuera del reporte público.

## Fase 16 — U11: criterio, cobertura y procedencia final

- [x] 16.1 RED: En `tests/test_retrieval_h5_runner.py`, probar PASS exclusivamente sintético, orden distinto/recall completo→FAIL, exact vacío→blocked/null, underfill/ties/scores; macro sin redondeo ni omisión de indefinidos. Falta/conflicto de cada evidencia final, otra consulta/run/identidad/hash o cobertura incompleta bloquean aun con métricas perfectas.
- [x] 16.2 GREEN: En `retrieval_h5_runner.py`, reutilizar serializadores ANN con quality_queries=[], `gate_result` y procedencia final por consulta; revalidar hashes/bindings/cobertura total, sin sobrescribir con última respuesta ni completar loaded/config desde expectativas. Relevancia queda `{status: "not_evaluated", results: {}}`, sin aceptación global.
- [x] 16.3 REFACTOR: Repetir escenarios; separar razones de criterio/procedencia, conservar K/per_query/protocolo/corpus/run y resumen descriptivo con conteo de indefinidos; sin reimplementar fórmulas en `retrieval_quality.py`. Completar límites de la guía.

## Fase 17 — cierre focalizado y regresiones

- [x] 17.1 Caracterización antes/después: En `tests/test_retrieval_h5_runner.py`, preservar piloto `run_evaluation`/CLI con annotations obligatorias e informes descriptivos; `protocol_preflight` sigue diagnóstico sin PASS. Probar OSError de escritura piloto→código 2 sin claim de persistencia; no exigir atomicidad inexistente.
- [x] 17.2 Ejecutar discovery indicado y contrato Python/schema; revisar diff focalizado y `git diff --check`, lint documental aplicable y ausencia de cambios nuevos en Rust/Docker/C0/corpus/candidato/plantilla/resultados históricos. No ejecutar builds ni Clippy para este slice Python.
- [x] 17.3 Registrar evidencia nueva y checks omitidos en `apply-progress.md`; preservar WIP de la guía, verify/QA históricos y estado. Entregar a sdd-verify solo como contrato offline; QA/aceptación, procedencia real, corpus/freeze y archive siguen bloqueados independientemente.
