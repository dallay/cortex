# Evidencia de implementación — protocolo de aceptación H5

## Fase 17 — cierre focalizado y regresiones

### Alcance y cambios

Se implementaron exclusivamente las tareas 17.1–17.3. En `dev/muninn/tests/test_retrieval_h5_runner.py` se añadieron tres pruebas de caracterización: la CLI sigue requiriendo `--annotations` y entrega el informe descriptivo devuelto por `run_evaluation`; un `OSError` de escritura del informe produce código 2 y un mensaje de error, sin afirmar que el reporte persistió; `protocol_preflight` no llama al bridge, marca `measurement_started=false` y no emite `PASS`. No se modificó lógica de producción porque la caracterización confirmó el comportamiento existente. No se exige atomicidad de escritura.

TDD: el test enfocado inicial fue **RED** (3 tests; un error `SystemExit: 2` esperado de argparse al faltar `--annotations`, pero la prueba no manejaba correctamente esa interfaz; ninguna falta de producto). Se ajustó el test para capturar `SystemExit` y afirmar `code == 2`; el ciclo enfocado pasó después (**GREEN/REFACTOR**, 3 tests `OK`). Se mantuvieron intactos los tests existentes y los cambios de producción acumulados.

### Comandos y salidas reales

- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest dev.muninn.tests.test_retrieval_h5_runner.PilotCompatibilityTests -v` — primera ejecución exit 1; `Ran 3 tests`, `FAILED (errors=1)`. `test_cli_preserves_annotation_requirement_and_descriptive_pilot_report` recibió `SystemExit: 2` porque argparse termina así al faltar un argumento obligatorio; se corrigió la aserción para capturar el comportamiento esperado.
- El mismo comando focalizado, tras el ajuste — exit 0; `Ran 3 tests`, `OK`.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py'` — exit 0; `Ran 148 tests in 57.870s`, `OK`.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.example.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; sin errores de validación; advertencia de deprecación de la CLI `jsonschema`.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.candidate.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; sin errores de validación; misma advertencia de deprecación.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md docs/muninn/retrieval-h5-runner.md` — exit 0; `Summary: 0 issues in 0 files`.
- `git diff --check` — exit 0, sin salida.
- `git status --short -- dev/muninn/bridge dev/muninn/compose.h5.yaml dev/muninn/fixtures docs/muninn/retrieval-h5-local-results.md .agents/sdd/changes/muninn-h5-acceptance-protocol/state.yaml .agents/sdd/changes/muninn-h5-acceptance-protocol/verify-report.md .agents/sdd/changes/muninn-h5-acceptance-protocol/qa-report.md` — exit 0. La salida muestra WIP previo en Rust, state, fixtures y QA/verify históricos. No se modificó ninguno de esos paths en esta fase. Tampoco hay nuevos cambios Docker, C0, corpus candidato, protocolo candidato/plantilla ni resultados históricos; `git status` no diferencia antigüedad, por lo que la afirmación de no cambio se limita al diff creado en esta fase, conocido por los archivos escritos aquí.
- Revisión focalizada de `git diff -- dev/muninn/tests/test_retrieval_h5_runner.py` — realizada. El fragmento de diff mostró los cambios de regresiones acumulados previos; el cambio de Fase 17 está acotado a la clase `PilotCompatibilityTests`, al final del archivo. `git diff --check` cubrió higiene del diff completo.

### Límites y checks omitidos

No se ejecutaron builds, Clippy, CI completa, servicios, Docker/Compose, bridge, base de datos, corpus real, mediciones, inferencia ni aceptación H5. No se actualizó `state.yaml`; se preservaron los reportes históricos `verify-report.md` y `qa-report.md`, el WIP de la guía y el resto del árbol dirty. Las suites sintéticas no son evidencia operativa ni de aceptación. Fase 17 queda disponible para `sdd-verify`; QA/aceptación, B.1/B.2, freeze/review del corpus, procedencia real y archive continúan bloqueados independientemente.

## Enmienda de métricas aprobadas

Se implementaron las métricas ANN–exact por consulta y el informe serializado por gate. El runner integra evaluaciones ya disponibles; no ejecuta retrieval ni contacta el bridge desde esta ruta. K es argumento obligatorio y no hay estado PASS en el serializador. El informe preflight conserva `not_evaluated` cuando no recibe resultados. Las fórmulas existentes Recall/nDCG coinciden con la spec y se caracterizaron sin cambiar la lógica. MRR se etiqueta como descriptiva.

### Evidencia TDD observada

| Slice                                      | RED observado                                                                                                                                                                                                                                                                        | GREEN / REFACTOR observado                                                                                                                                                                                                                | Límite de evidencia                                                                                                                                                                                                                        |
|--------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Métricas ANN–exact 8.1–8.4                 | `PYTHONPATH=dev/muninn python3 -m unittest dev.muninn.tests.test_retrieval_quality.RetrievalQualityTests.test_ann_exact_metrics_compare_ids_not_scores_and_keep_set_and_order_distinct -v` — exit 1; `ImportError` porque aún faltaba `evaluate_ann_exact_query`; un test ejecutado. | La primera ejecución de la suite de calidad falló: 28 tests, un fallo de expectativa de underfill (`1.0` esperado frente a `0.5` observado). Tras corregir la expectativa, la suite de calidad acabó `OK` (28) y la del runner `OK` (16). | El registro original no conserva RED separado para cada caso de lista vacía, underfill e input inválido. La expectativa fallida fue una corrección del test, no un RED de producto. No se afirma evidencia RED individual para esos casos. |
| Integración de métricas al informe 8.5–8.7 | `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolPreflightTests.test_integrated_gate_report_serializes_supplied_metrics_and_preserves_blocking_states -v` — exit 1; `ImportError` porque faltaba `integrate_protocol_gate_metrics`.      | Mismo comando — exit 0, 1 test `OK`. Tras refactor: suite H5 exit 0, 45 tests `OK`; suite calidad exit 0, 28 tests `OK`; `git diff --check` exit 0.                                                                                       | Esta secuencia sí evidencia RED→GREEN→REFACTOR para la integración. No hubo bridge real ni medición.                                                                                                                                       |
| Caracterización de Recall/nDCG 8.8–8.9     | No aplica como nueva conducta si la fórmula existente coincide; se ejecutó una prueba de caracterización.                                                                                                                                                                            | La prueba confirma Recall con relevancia de etiquetas juzgadas y nDCG con ganancia `2^grade-1`, descuento `log2(rank+1)` e ideal sobre candidatos juzgados. No se cambió la lógica.                                                       | Las etiquetas del piloto no se aprobaron ni se usaron para un protocolo-run.                                                                                                                                                               |

Las suites frescas de verificación posteriores a esta enmienda deben registrarse en `verify-report.md`. La evidencia limitada de los subcasos iniciales se declara explícitamente; no se reconstruyen logs ni se atribuye retroactivamente un orden TDD que no consta.

## Alcance implementado

Se implementaron las tareas ejecutables 1.1–7.4. El colector local consulta Docker Engine mediante `inspect` de solo lectura y calcula hashes de bytes locales. Registra la ejecución, los contenedores, la imagen y los mounts, pero no convierte en evidencia los claims del caller sobre encoder o configuración. Como `inspect` y los hashes no establecen qué encoder/configuración quedó cargado, esos campos obligatorios quedan ausentes; el preflight informa procedencia `partial` por gate y bloquea `PASS`. Una ejecución no identificable o evidencia contradictoria produce `insufficient`.

La elegibilidad usa una ruta independiente por gate. Un resultado de regla ausente o indefinido bloquea elegibilidad porque el protocolo-run no tiene una política de tratamiento aprobada. El preflight informa por separado protocolo, procedencia, evidencia ausente/contradictoria y resultados no evaluados. No consulta el bridge ni inicia una medición. Los informes históricos descriptivos permanecen intactos.

No se cambiaron retrieval, Store, ranking, filtros, scope, threshold ni fallback. No se iniciaron servicios ni mediciones. El colector Docker se probó con fakes, no contra un Docker Engine activo.

## Evidencia TDD

Los siguientes RED y GREEN de la primera unidad constan en el registro de la sesión de implementación. Los códigos de salida se incluyen solo cuando quedaron observados explícitamente.

| Tareas y comportamiento                                    | RED observado                                                                                                                                                                                                                                                                                                                                              | GREEN observado                                                                                                                         | Refactor y comprobación posterior                                                                                                                                                                                            |
|------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| 1.1–2.3: protocolo, hash y corroboración                   | `PYTHONPATH=dev/muninn python3 -m unittest dev/muninn/tests/test_retrieval_h5_protocol.py -v` — código 1; importación fallida porque faltaba `retrieval_h5_protocol`. `PYTHONPATH=dev/muninn python3 -m unittest dev/muninn/tests/test_retrieval_h5_protocol.py -k approval -v` — código 1; `TypeError` por interfaz de corroboración aún no implementada. | El test `-k approval` terminó `OK` (1 test). La suite de protocolo terminó `OK` (3 tests).                                              | La sesión marcó 1.4 y 2.3 completas y ejecutó la suite final de calidad (22 tests `OK`). La salida no conserva un comando REFACTOR separado por subtarea. La verificación fresca final consta en «Comandos de verificación». |
| 3.1–3.3: evidencia por gate                                | `PYTHONPATH=dev/muninn python3 -m unittest dev/muninn/tests/test_retrieval_h5_provenance.py -v` — código 1; importación fallida porque faltaba `retrieval_h5_provenance`.                                                                                                                                                                                  | La suite enfocada de procedencia terminó `OK` (3 tests).                                                                                | La sesión marcó 3.3 completa. La salida no conserva un comando REFACTOR separado. La suite fresca final incluye los tests del colector y sus límites.                                                                        |
| 4.1–4.3: evaluación independiente y resultados indefinidos | `PYTHONPATH=dev/muninn python3 -m unittest dev/muninn/tests/test_retrieval_quality.py -k gate_pass -v` — código 1; no existía `gate_result`.                                                                                                                                                                                                               | El test enfocado terminó `OK` (1 test); después, la suite de calidad terminó `OK` (22 tests).                                           | 4.3 se completó en la segunda unidad con RED/GREEN observado para resultados indefinidos; la suite actual se volvió a ejecutar.                                                                                              |
| 5.1–5.5: preflight y runner                                | `PYTHONPATH=dev/muninn python3 -m unittest dev/muninn/tests/test_retrieval_h5_runner.py -k incomplete_run_protocol -v` — código 1; no existía `protocol_preflight`.                                                                                                                                                                                        | El test enfocado terminó `OK` (1 test). La sesión informó la suite H5 final con 36 tests `OK`.                                          | 5.5 se marcó completa; la salida de la primera unidad no conserva un comando REFACTOR separado. La segunda unidad observó RED/GREEN para integración y volvió a ejecutar la suite completa.                                  |
| 3.2: colector local Docker y hashes                        | `PYTHONPATH=dev/muninn python3 -m unittest dev/muninn/tests/test_retrieval_h5_provenance.py -v` — código 1; 5 tests, 2 errores porque `collect_observations` no existía. Una regresión posterior también dio RED: 6 tests, 1 fallo al clasificar una ejecución no identificable.                                                                           | El comando enfocado terminó `OK`; la sesión informó 5 tests después de implementar el colector y 6 después de ajustar la clasificación. | La suite fresca final incluye los tests de colector, conflictos y clasificación. No se invocó Docker Engine real.                                                                                                            |
| 4.3: bloqueo ante reglas indefinidas                       | `PYTHONPATH=dev/muninn python3 -m unittest dev/muninn/tests/test_retrieval_quality.py -v` — código 1; 24 tests, 1 fallo y 1 error por el tratamiento indefinido y el evaluador aún ausente.                                                                                                                                                                | El comando enfocado terminó `OK` con 24 tests.                                                                                          | La suite fresca final de calidad terminó `OK` con 24 tests. No se seleccionó una política estadística.                                                                                                                       |
| 5.4 y 6.1–6.3: integración e informe                       | La suite enfocada del runner dio RED: 13 tests, 2 errores por entradas de informe aún no soportadas. Una regresión de hash conflictivo dio RED: 15 tests, 1 fallo.                                                                                                                                                                                         | El runner enfocado terminó `OK` con 15 tests.                                                                                           | La suite H5 fresca final terminó `OK` con 43 tests. La prueba confirma que el informe histórico en memoria no cambia; el documento histórico también se verificó intacto.                                                    |

La primera unidad no conservó una salida REFACTOR individual para cada tarea 1.4, 2.3, 3.3 y 5.5. Las casillas se marcaron completas durante esa implementación y las suites posteriores se ejecutaron sobre el estado final. Esta limitación de granularidad queda visible para revisión; no se inventan comandos ni conteos ausentes.

## Comandos de verificación

- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_*.py' -v` — código 0; 43 tests aprobados.
- `PYTHONPATH=dev/muninn python3 -m unittest dev/muninn/tests/test_retrieval_quality.py -v` — código 0; 24 tests aprobados.
- `python3 -m py_compile dev/muninn/retrieval_h5_protocol.py dev/muninn/retrieval_h5_provenance.py dev/muninn/retrieval_h5_runner.py dev/muninn/retrieval_quality.py` — código 0.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.example.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — código 0; el esquema y el ejemplo validaron. La CLI mostró una advertencia de deprecación.
- `pnpm exec markdownlint-cli2 docs/muninn/retrieval-h5-runner.md .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md` — código 0; 0 incidencias.
- `git diff --check` — código 0.
- `git diff --exit-code -- docs/muninn/retrieval-h5-local-results.md` — código 0; el informe histórico no cambió.
- `git diff --exit-code -- .agents/sdd/changes/muninn-h5-acceptance-protocol/state.yaml` — código 0 en la implementación; el estado lo actualiza el orquestador.

La verificación fresca del orquestador volvió a ejecutar las suites: 43 tests H5 y 24 tests de calidad pasaron. No se inició Docker, bridge, base de datos ni medición de retrieval.

## Bloqueos y límites restantes

El framework no puede establecer qué encoder/configuración quedó cargado en el bridge con las observaciones locales permitidas. Por eso, la procedencia de cada gate afectado sigue `partial` y no puede recibir `PASS`. Las decisiones concretas del protocolo-run, incluido el tratamiento de resultados indefinidos, siguen sin aprobarse. B.1 y B.2 permanecen bloqueadas; no se declara aceptación del producto.

## Fase 9 — identidad del encoder H5

Se implementó la identidad allowlisted derivada de la instancia conservada en `App.encoder`. La carga lee los dos buffers de configuración verificados, los valida con el predicado de perfil existente, calcula un SHA-256 con framing determinista (longitud BE de etiqueta, etiqueta, longitud BE de contenido, contenido; orden fijo de archivo) y retiene identidad de bundle, modelo, revisión, perfil y fingerprint. `space_id` se obtiene del mismo adapter. El handler incluye identidad en la misma respuesta autenticada y feature-gated que ANN/exact; no se añadió llamada de búsqueda, Store, bridge, endpoint, logging ni ruta normal.

El runner exige objeto de respuesta y grupos exactos, tipos string y formatos/valores allowlisted. `run_inputs.encoder_identity` ahora valida la identidad esperada con la misma allowlist y forma parte del hash del protocolo. El preflight toma esa expectativa del protocolo validado y marca conflicto si una observación suministrada intenta reemplazarla. La correlación explícita rechaza mismatch de identidad, run o query. El modo descriptivo de `run_evaluation` no recibe la identidad aprobada del protocolo ni observaciones Docker/run correlacionadas; por eso cada self-report recibido se marca `partial` y `attestation: false`. Caller claims no sustituyen la respuesta ni inspect. El reporte conserva el modo descriptivo `measured_not_accepted`.

### Evidencia TDD exacta

| Slice                                | RED observado (comando y salida)                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | GREEN / REFACTOR observado                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
|--------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| A — framing y perfil de loader       | `cargo test --manifest-path dev/muninn/bridge/Cargo.toml --lib minilm_encoder_tests::configuration_fingerprint_uses_framed_labels_and_lengths` — exit 101; `error[E0425]: cannot find function validated_profile_id`; `error[E0425]: cannot find function configuration_sha256` (cuatro referencias ausentes).                                                                                                                                                                                             | `cargo test --manifest-path dev/muninn/bridge/Cargo.toml --lib minilm_encoder::tests::configuration_fingerprint_uses_framed_labels_and_lengths` — exit 0; `running 1 test`; `1 passed; 0 failed`. `cargo test --manifest-path dev/muninn/bridge/Cargo.toml --lib minilm_encoder::tests::accepts_only_the_declared_transformer_mean_pooling_normalize_profile` — exit 0; `1 passed; 0 failed`. El test fija el digest `bb9e624626926c9acf924bc7856227c51a4e2222cb159280a81153ad35063e79` y comprueba que invertir los buffers cambia el digest. |
| B — respuesta de la misma instancia  | `cargo test --manifest-path dev/muninn/bridge/Cargo.toml --bin muninn-bridge-spike --features h5-diagnostic diagnostic_identity_comes_from_the_encoder_instance_that_encoded_the_query` — exit 101 al retirar temporalmente el helper; `error[E0425]: cannot find function diagnostic_embedding`.                                                                                                                                                                                                          | El mismo comando después de implementar: exit 0; `running 1 test`; `1 passed; 0 failed`. El spy solo devuelve identidad después de que esa instancia haya codificado. El test inspecciona grupos/keys permitidos y ausencia de query/path/vector.                                                                                                                                                                                                                                                                                              |
| C — sanitizer y procedencia          | `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_provenance.ProvenanceTests.test_encoder_response_identity_requires_exact_allowlist_and_external_correlation -v` — exit 1; `ImportError: cannot import name 'sanitize_encoder_identity'`. `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.RunnerTests.test_diagnostic_response_requires_identity_in_the_same_allowlisted_response -v` — exit 1; `AssertionError: ValueError not raised`. | `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_provenance test_retrieval_h5_runner -v` — exit 0; `Ran 26 tests`; `OK`.                                                                                                                                                                                                                                                                                                                                                                                          |
| C — identidad esperada del protocolo | `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_protocol.ProtocolTests.test_protocol_requires_a_strict_expected_encoder_identity_for_confirmatory_readiness -v` — exit 1; `ValueError: run_inputs has unknown or missing properties`.                                                                                                                                                                                                                                        | Mismo comando junto al test de sustitución: exit 0; `Ran 2 tests`; `OK`. El preflight preserva la identidad del documento de protocolo y señala como conflicto un expected identity caller-supplied distinto.                                                                                                                                                                                                                                                                                                                                  |

Una primera ejecución de GREEN usó filtros de módulo incorrectos y reportó `running 0 tests`; no cuenta como RED ni como evidencia de comportamiento. Se corrigió el filtro y se repitió con los nombres observados por `-- --list`.

### Comandos ejecutados y resultados

- `cargo test --manifest-path dev/muninn/bridge/Cargo.toml --package muninn-bridge-spike --lib` — exit 0; 178 tests, 172 passed, 6 ignored, 0 failed, sin feature.
- `cargo test --manifest-path dev/muninn/bridge/Cargo.toml --package muninn-bridge-spike --lib --features h5-diagnostic` — exit 0; 180 tests, 174 passed, 6 ignored, 0 failed, con feature.
- `cargo test --manifest-path dev/muninn/bridge/Cargo.toml --package muninn-bridge-spike --bin muninn-bridge-spike` — exit 0; 1 passed, 0 failed, sin feature.
- `cargo test --manifest-path dev/muninn/bridge/Cargo.toml --package muninn-bridge-spike --bin muninn-bridge-spike --features h5-diagnostic` — exit 0; 3 passed, 0 failed, con feature; incluye rechazo HTTP sin auth.
- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_*.py' -v` — exit 0; `Ran 50 tests`; `OK`.
- `python3 -m py_compile dev/muninn/retrieval_h5_protocol.py dev/muninn/retrieval_h5_provenance.py dev/muninn/retrieval_h5_runner.py dev/muninn/tests/test_retrieval_h5_protocol.py dev/muninn/tests/test_retrieval_h5_provenance.py dev/muninn/tests/test_retrieval_h5_runner.py` — exit 0, sin salida.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.example.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; solo mostró la advertencia `The jsonschema CLI is deprecated and will be removed in a future version.`
- `cargo fmt --all -- --check` — exit 0, sin salida. `rustfmt --check --edition 2021 dev/muninn/bridge/src/artifact_model_stage.rs dev/muninn/bridge/src/embedding_port.rs dev/muninn/bridge/src/main.rs dev/muninn/bridge/src/minilm_encoder.rs dev/muninn/bridge/src/minilm_encoder_tests.rs` — exit 0, sin salida. Se revirtieron solo cambios de formato incidentales en tres archivos Rust que estaban limpios antes de ejecutar el formatter del workspace separado.
- `pnpm exec markdownlint-cli2 docs/muninn/retrieval-h5-runner.md .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md` — exit 0; `Summary: 0 issues in 0 files`.
- `git diff --check` — exit 0, sin salida.
- `cargo clippy --manifest-path dev/muninn/bridge/Cargo.toml --package muninn-bridge-spike --all-targets --features h5-diagnostic -- -D warnings` — exit 101; la compilación reporta 93 errores en el target lib y 120 en test, entre ellos `missing_errors_doc`, `float_cmp`, `implicit_hasher`, `result_unit_err` y lints en `storage.rs`. El mismo comando sin feature también falló por lints del crate. No se relajaron lints ni se tocaron módulos ajenos al alcance. Este gate no está aprobado.

### Pendiente y límites de verificación

- 9.10 se completó como verificación del contrato de compilación: el registro productivo de `/h5/diagnostic/search` está directamente bajo `#[cfg(feature = "h5-diagnostic")]` en `run()` (líneas 643–644); las suites lib/bin compilaron y pasaron con y sin feature. El test HTTP usa un router in-process de ruta protegida y confirma rechazo sin autenticación, pero no construye el router productivo completo. No se inició servicio ni se creó una abstracción para inyectar `Store` solo para inspeccionar la tabla de rutas.
- Clippy con `-D warnings` se ejecutó focalizado para el bin con feature y falló porque el target lib registra 93 errores; la ejecución previa `--all-targets` informó 120 errores en tests. Ejemplos incluyen `missing_errors_doc`, `float_cmp`, `implicit_hasher`, `result_unit_err` y lints en `storage.rs`. El filtro por archivos modificados mostró diagnósticos en APIs/documentación existentes de `artifact_model_stage.rs`, `embedding_port.rs` y `minilm_encoder.rs`, pero ninguno en las funciones nuevas de identidad/fingerprint o en `main.rs`. No se corrigieron lints ajenos ni se relajó `-D warnings`; Clippy no pasa y queda como riesgo de integración.
- No se ejecutaron builds o pruebas nuevas durante esta continuación. Los comandos de Rust, Python, schema, format y Markdownlint listados arriba corresponden a la primera ejecución de Apply y deben repetirse en verificación.
- No se ejecutaron Docker, bridge en vivo, inspección Docker, mediciones ni búsqueda real. No se modificaron `state.yaml`, `verify-report.md`, `qa-report.md`, proposal, spec ni design.
- El self-report no es crypto attestation, no prueba pesos ni memoria cargados, y este cambio no acepta el producto ni desbloquea medición confirmatoria.

## Phase 10 — strict protocol template and structural validator

### Implemented scope

Decision fields now use a closed `status` / `value` / `rationale` envelope. `pending` requires a null value and a non-placeholder rationale. `approved` values are checked against route-specific object/array/scalar shapes and closed property allowlists. Required metric K keys and the labels/rubric and ANN–exact scope fields remain conditional on their corresponding gate. Metrics definitions do not supply K values. Findings are sorted deterministically into `missing`, `pending`, and `malformed` groups with JSON paths; any finding blocks `confirmatory_ready`. The legacy `missing_fields` list remains available and contains paths from all three groups. The runner preserves that field and also exposes the grouped findings. An approved JSON declaration is not operator corroboration; approval still requires the complete protocol hash and the existing operator record.

The example fixture remains pending and is not confirmatory-ready. No run-specific product or human values were entered. No metrics, identity self-report, gates, normal runner behavior, or historical reports were changed. No Docker, bridge, database, live measurement, or confirmatory run was started. This implementation does not claim product acceptance.

### TDD evidence

| Slice                                                         | RED                                                                                                                                                                                                                                                                                                                                                                                                                                         | GREEN                                                                                                                                                                                                                                                                                                                                                                                          | REFACTOR                                                                                                                                                                                                                                |
|---------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Decision envelope and finding behavior (10.1–10.3, 10.7–10.9) | `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_protocol.ProtocolTests.test_decision_envelopes_require_explicit_status_value_and_rationale -v` — exit 1; `KeyError: 'pending'` because the validator did not expose pending findings.                                                                                                                                                                         | The focused case was corrected to assert JSON paths, then passed in the final protocol suite (10 tests, `OK`). Approved values missing metric keys, malformed envelopes, placeholders, empty values, and nested unknown properties are covered.                                                                                                                                                | The complete protocol suite passed (10 tests); runner compatibility suite passed (19 tests). `corroborate_approval` tests reject absent operator evidence and a mismatched complete-protocol hash.                                      |
| Schema, fixture, and gate applicability (10.4–10.6)           | `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_protocol.ProtocolTests.test_json_schema_matches_pending_template_and_strict_approved_shapes -v` — exit 1 during schema iteration; first the pending fixture did not validate against the then-current schema. After that was corrected, the test exposed that Python raised `ValueError: approved_at must be an ISO-8601 timestamp` for the pending template. | The same focused test passed after schema correction and pending metadata handling. It validates the pending fixture through both schema and Python, a representative structurally approved JSON document, rejection of `{}`, and rejection of an unknown nested property. The representative values exist only in test memory and do not populate the fixture or constitute product approval. | The schema gate-conditional test passed with ANN–exact enabled and absolute relevance disabled, omitting labels/rubric and quality-metric K values. The full protocol suite passed after the schema and Python structures were aligned. |

### Commands run and exact results

- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_protocol.py' -v` — exit 0; 10 tests, `OK`.
- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_runner.py' -v` — exit 0; 19 tests, `OK`.
- `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_protocol.ProtocolTests.test_json_schema_matches_pending_template_and_strict_approved_shapes -v` — exit 0; 1 test, `OK` (fixture pending-state/schema checks and an approved structural-only sample).
- `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_protocol.ProtocolTests.test_json_schema_matches_pending_template_and_strict_approved_shapes test_retrieval_h5_protocol.ProtocolTests.test_gate_specific_fields_are_required_only_for_enabled_gate -v` — exit 0; 2 tests, `OK`.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.example.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; emitted only the CLI deprecation warning recommending `check-jsonschema`.
- `python3 -m py_compile dev/muninn/retrieval_h5_protocol.py dev/muninn/retrieval_h5_runner.py dev/muninn/tests/test_retrieval_h5_protocol.py dev/muninn/tests/test_retrieval_h5_runner.py` — exit 0; no output.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md` — exit 0; `Summary: 0 issues in 0 files`.
- `git diff --check` — exit 0; no output.

Clippy `-D warnings` remains the pre-existing bridge blocker recorded above. It was not run again or weakened in this phase. No broad builds were run.

### Reviewer-confirmed protocol-run corrections

Only the four confirmed protocol-run findings were addressed:

- `repository_review.approved_at` accepts the explicit `PENDING` fixture sentinel as pending metadata. Non-placeholder values must match a timestamp structure with a timezone in Python and JSON Schema. Python rejects naive and malformed timestamps; the schema declares `date-time` format and constrains the lexical structure.
- `corroborate_approval` returns `False` when either timestamp is naive, before comparing timestamps.
- `approved_gates` validates each member as a string before membership and duplicate checks, so JSON arrays/objects produce a controlled `ValueError`.
- JSON Schema rejects K metrics for disabled gates. When both gates are disabled, approved `metrics_and_k` is disallowed, matching the existing Python validator's malformed classification. Pending decisions remain valid, and no gate-enable requirement was added.

#### TDD evidence for the reviewer corrections

| Finding                             | RED observed                                                                                                                             | GREEN / REFACTOR observed                                                                                                                                        |
|-------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Timestamp structure and timezone    | The initial focused run below exited 1: Python accepted a naive timestamp and the schema accepted arbitrary/malformed timestamp strings. | The focused regression set passed (4 tests); the full protocol suite passed (14 tests). The fixture sentinel remains schema-valid and Python reports it pending. |
| Naive/aware corroboration           | The initial focused run below exited 1 with `TypeError: can't compare offset-naive and offset-aware datetimes`.                          | Focused regression passed; the full protocol suite passed (14 tests).                                                                                            |
| Non-string `approved_gates` members | The initial focused run below exited 1 with `TypeError` for list/dict items during set membership.                                       | Focused regression passed for both JSON list and object; full protocol suite passed (14 tests).                                                                  |
| Per-gate metric K allowlist         | The initial focused run below exited 1: schema accepted K for a disabled gate while Python classified the path as malformed.             | Regression covers both one-gate-enabled combinations and both-gates-disabled; focused set and full protocol suite passed.                                        |

The exact RED command was `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_protocol.ProtocolTests.test_approved_at_requires_timezone_but_pending_sentinel_remains_schema_valid test_retrieval_h5_protocol.ProtocolTests.test_corroborate_approval_fails_closed_for_naive_aware_timestamps test_retrieval_h5_protocol.ProtocolTests.test_approved_gates_rejects_non_string_json_items_as_value_error test_retrieval_h5_protocol.ProtocolTests.test_metric_k_allowlist_matches_gate_combination_in_python_and_schema -v`. It ran four tests and exited 1 with the failures above. The passing focused invocation was repeated after the fixes. The broader protocol and runner suites were then run without filters.

#### Checks repeated for these corrections

- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_protocol.py' -v` — exit 0; 14 tests, `OK`.
- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_runner.py' -v` — exit 0; 19 tests, `OK`.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.example.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; fixture accepted. CLI deprecation warning only.
- `python3 -m py_compile dev/muninn/retrieval_h5_protocol.py dev/muninn/tests/test_retrieval_h5_protocol.py` — exit 0.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md` — exit 0; `Summary: 0 issues in 0 files`.
- `git diff --check` — exit 0; no output.

No changes were made to acceptance values, `verify-report.md`, `qa-report.md`, `state.yaml`, proposal, spec, or design. No Docker, services, database, bridge, measurements, or Clippy command were run. No product acceptance is claimed.

### Phase 10 residual — canonical metadata sentinel

JSON Schema now accepts metadata (`protocol_id`, `protocol_version`, `commit`, and `review_reference`) only as the exact `PENDING` sentinel or a non-placeholder string. The fixture uses `PENDING` for all five metadata fields, including `approved_at`. Python continues to classify these fixture values as pending, and the fixture is not confirmatory-ready. The schema rejects `pending whatever`; approved `approved_at` values still require the existing RFC3339/date-time pattern with a timezone.

#### TDD evidence

- **RED:** `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_protocol.ProtocolTests.test_metadata_uses_only_the_exact_pending_sentinel -v` — exit 1; schema unexpectedly accepted `pending whatever` for `protocol_id`.
- **GREEN:** The focused metadata and timestamp tests passed (2 tests). The complete protocol suite passed (15 tests), the runner suite passed (19 tests), and the full H5 suite passed (61 tests).
- **REFACTOR:** No additional refactor was needed. The required focused suites were repeated after the schema, fixture, and regression test changes.

#### Commands run for the residual

- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_protocol.py' -v` — exit 0; 15 tests, `OK`.
- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_runner.py' -v` — exit 0; 19 tests, `OK`.
- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_*.py' -v` — exit 0; 61 tests, `OK`.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.example.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; fixture accepted. The CLI emitted its deprecation warning.
- `python3 -m py_compile dev/muninn/retrieval_h5_protocol.py dev/muninn/tests/test_retrieval_h5_protocol.py dev/muninn/retrieval_h5_runner.py dev/muninn/tests/test_retrieval_h5_runner.py` — exit 0; no output.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md` — exit 0; 0 issues.
- `git diff --check` — exit 0; no output.

No decision values or task checkboxes changed. QA/verify reports and `state.yaml` remain untouched. No Docker, bridge, database, measurement, or Clippy command was run.

### Phase 10 residual — leading whitespace before `PENDING`

The JSON Schema `nonPlaceholder` pattern rejected `pending ...` when it began at character zero, but accepted a leading-space variant such as `protocol_id: " PENDING"`. Python classified the value as pending, while the schema allowed it as a non-placeholder. The schema regex now allows optional whitespace before the `pending` placeholder alternative so this near-sentinel is rejected. The contractual exact `PENDING` value remains accepted through `pendingMetadata` and is still classified as pending by Python. No other placeholder values or policy changed.

#### TDD evidence

| Behavior                                                                                         | RED                                                                                                                                                                                                                                                                                             | GREEN / REFACTOR                                                                                                                                                                                                                                                                                                          |
|--------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Reject leading whitespace before the pending metadata sentinel, while preserving exact `PENDING` | `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_protocol.ProtocolTests.test_metadata_uses_only_the_exact_pending_sentinel -v` — exit 1; the schema returned success for `protocol_id: " PENDING"`. The test also asserts Python classifies that value as pending. | The same focused command — exit 0; 1 test, `OK`. The existing test confirms exact fixture `PENDING` metadata remains schema-valid and appears in Python's pending findings. The full protocol suite passed (15 tests), the runner suite passed (19 tests), and full H5 passed (61 tests). No further refactor was needed. |

#### Commands run for the residual

- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_protocol.py' -v` — exit 0; 15 tests, `OK`.
- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_runner.py' -v` — exit 0; 19 tests, `OK`.
- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_*.py' -v` — exit 0; 61 tests, `OK`.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.example.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; fixture accepted. The CLI emitted its deprecation warning.
- `python3 -m py_compile dev/muninn/retrieval_h5_protocol.py dev/muninn/tests/test_retrieval_h5_protocol.py dev/muninn/retrieval_h5_runner.py dev/muninn/tests/test_retrieval_h5_runner.py` — exit 0; no output.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/proposal.md .agents/sdd/changes/muninn-h5-acceptance-protocol/specs/retrieval-acceptance/spec.md .agents/sdd/changes/muninn-h5-acceptance-protocol/design.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md` — exit 0; 0 issues.
- `git diff --check` — exit 0; no output.

The regression test demonstrates task 10.11's schema behavior. `tasks.md` was already complete and was not changed. No reports, `state.yaml`, or other decision artifacts were changed. No Docker, bridge, database, live measurement, or Clippy command was run. No product acceptance is claimed.

### Phase 10 — acceptance rules follow enabled gates (10.12–10.13)

The Python value schema now requires rule properties only for enabled gates. The JSON Schema enforces the same exact applicability: enabled gates require their rule, disabled gates reject their rule, and unknown properties remain disallowed. With both gates disabled, a pending decision with `value: null` remains valid but not ready; an approved empty object remains invalid. No gate activation, criterion, rule, or threshold was selected. Structural sample values are confined to tests.

#### TDD evidence

| Stage    | Evidence                                                                                                                                                                                                                                                                                                                                                                                                                                |
|----------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| RED      | `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_protocol.ProtocolTests.test_acceptance_rules_match_enabled_gates_in_python_and_schema -v` — exit 1; both single-gate-enabled cases failed because Python reported the disabled rule as required. The new regression exercises all four gate combinations, Python and schema acceptance/rejection, and the disabled-gates pending/empty-approved boundary. |
| GREEN    | The focused test passed after making the Python rule shape gate-dependent and adding exact enabled/disabled conditions to JSON Schema. It covers required enabled rules, omitted disabled rules, rejection of disabled-rule extras, both-gates-enabled acceptance, and both-gates-disabled pending versus approved-empty behavior.                                                                                                      |
| REFACTOR | Repeated the focused test and all broader checks below after aligning the pre-existing gate-applicability test sample with the stricter disabled-rule contract. No behavior beyond the specified gate allowlist changed.                                                                                                                                                                                                                |

#### Commands run and exact results

- `PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_protocol.ProtocolTests.test_acceptance_rules_match_enabled_gates_in_python_and_schema -v` — exit 0; 1 test, `OK`.
- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_protocol.py' -v` — exit 0; 16 tests, `OK`.
- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_runner.py' -v` — exit 0; 19 tests, `OK`.
- `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_*.py' -v` — exit 0; 62 tests, `OK`.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.example.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; fixture accepted. CLI deprecation warning only.
- `python3 -m py_compile dev/muninn/retrieval_h5_protocol.py dev/muninn/tests/test_retrieval_h5_protocol.py dev/muninn/retrieval_h5_runner.py dev/muninn/tests/test_retrieval_h5_runner.py` — exit 0; no output.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md` — exit 0; 0 issues.
- `git diff --check` — exit 0; no output.

No `verify-report.md`, `qa-report.md`, proposal, spec, design, fixture, or decision values were changed. No Docker, bridge, database, measurement, Clippy, commit, branch, or PR operation was performed. No product acceptance is claimed.

## Phase 11 — review-only protocol and corpus candidates

### Artifacts created

- `dev/muninn/fixtures/retrieval-h5-protocol.candidate.json` records the user-approved candidate decisions: `ann_exact=true`, `absolute_relevance=false`, ANN–exact K=10, both ANN–exact metrics exactly 1.0 per query, one run, per-query reporting, descriptive-only macro-average, no retrospective tie-breaking, and required undefined metrics block PASS. Quality-metric K and thresholds remain non-applicable because absolute relevance is disabled. Corpus scope, encoder identity, provenance, and repository review metadata remain pending. The structural validator reports `confirmatory_ready=false`; the candidate is not authorization to measure.
- `dev/muninn/fixtures/retrieval-h5-corpus-candidate.json` is an independent, unlabelled draft with one natural-language query and ten candidate summaries in the exact `validate_corpus` shape.
- `dev/muninn/fixtures/retrieval-h5-corpus-candidate.provenance.json` maps the query and ten candidate IDs to exact baseline source paths and SHA-256 hashes. It is marked `draft/not frozen` and records canonical corpus hash `sha256:44ef7c4ad3fbe179d2651564eff980ad4d7c6b2c315d790928a08b1b76a859d7`.
- `dev/muninn/fixtures/retrieval-h5-corpus-candidate.inventory.md` records the baseline allowlist, exclusions, license context, sensitivity limitation, and owner-review requirement.

### Inventory and limitations

The inventory found 29 tracked Markdown files directly under `docs/muninn/` at baseline `09bb21d5fd2c847ac7df5e48f7be06eb08cec445`; 21 remain eligible after the specified exclusions. All eleven query/candidate source mappings use eligible baseline files. The selected text was drafted as neutral summaries from project docs, not copied from pilot queries or passages. No labels, grades, relevance judgments, rankings, scores, search outputs, or measurements were created. Ten candidates meet the structural minimum for the single draft query; this does not establish corpus adequacy, representativeness, or relevance. Project sensitivity classification is absent. Root MIT license context is recorded but is not treated as rights approval. Owner review of every text and mapping is pending.

The corpus candidate's sources include documents that describe different Muninn features. The set is intentionally unjudged; its usefulness and query/source assignments remain for owner review. Do not treat the candidate set as a relevance ordering or frozen evaluation corpus.

### Offline checks performed

- `validate_protocol` accepted the candidate structure with no malformed decisions and returned `confirmatory_ready=false`. It reports pending repository/run inputs and absent recorded gate approval.
- `validate_corpus` accepted the corpus. The canonical corpus hash recomputed as `sha256:44ef7c4ad3fbe179d2651564eff980ad4d7c6b2c315d790928a08b1b76a859d7`, matching the provenance manifest. Candidate count: 10 for the single query.
- Source manifest checks confirmed exact closed manifest keys, unique complete item mappings (1 query + 10 candidates), unique IDs, all paths in the eligible baseline allowlist, and source-byte SHA-256 values matching Git objects at the baseline commit.
- JSON parse and schema validation, Markdown lint, and `git diff --check` results are recorded after the final artifact checks below.

Exact check outcomes:

- `PYTHONPATH=dev/muninn python3` offline validation script — exit 0; protocol validator, corpus validator/hash, manifest closure, complete unique mappings, baseline eligibility, source hashes, exclusion checks, and minimum candidate count passed.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.candidate.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; candidate conforms to the existing schema. The CLI emitted its deprecation warning.
- `python3 -m json.tool` on each corpus/provenance JSON — exit 0.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md dev/muninn/fixtures/retrieval-h5-corpus-candidate.inventory.md` — exit 0; 0 issues.
- `git diff --check` — exit 0.

The minimum candidate-count condition was met for the one draft query, so task 11.4's stop condition did not apply. This is a structural coverage check only, not a corpus adequacy or representativeness finding.

No retrieval/search, model inference, Docker, bridge, database, measurement, annotation, or live provenance operation was run. `verify-report.md`, `qa-report.md`, `state.yaml`, source documentation, the existing runner, and pilot assets were not modified. QA remains `BLOCKED`; the candidate corpus is not frozen and the protocol is not ready or executable.

### Orchestrator independent verification

- Re-ran an offline Python check using `validate_protocol`, `validate_corpus`, and `corpus_hash`: exit 0. The protocol reports `confirmatory_ready=false`, has no malformed fields, and retains pending approval, corpus scope, ANN comparison scope, encoder identity, and provenance. The corpus has one query with ten candidates and hash `sha256:44ef7c4ad3fbe179d2651564eff980ad4d7c6b2c315d790928a08b1b76a859d7`.
- Recomputed all 11 query/candidate source mappings against baseline Git objects at `09bb21d5fd2c847ac7df5e48f7be06eb08cec445`: every mapping was unique and complete, every source path was in the inventory allowlist, and every SHA-256 matched. Exit 0.
- `jsonschema -i dev/muninn/fixtures/retrieval-h5-protocol.candidate.json dev/muninn/fixtures/retrieval-h5-protocol.schema.json` — exit 0; deprecation warning only.
- Parsed corpus/provenance JSON with `python3 -m json.tool` — exit 0.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/proposal.md .agents/sdd/changes/muninn-h5-acceptance-protocol/specs/retrieval-acceptance/spec.md .agents/sdd/changes/muninn-h5-acceptance-protocol/design.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md dev/muninn/fixtures/retrieval-h5-corpus-candidate.inventory.md .agents/rpi/plan/tasks/muninn-h5-retrieval-context.md` — exit 0; 0 issues.
- `git diff --check` — exit 0; no output.
- Independent review: no findings. Reviewer scope did not independently recompute hashes or inspect baseline source content; the orchestrator checks above covered source mappings and hashes.
- Final report hygiene: the first Markdownlint run that included the newly appended verification section found a second H1 in `verify-report.md`. Changed that section heading to H2, then reran Markdownlint across proposal/spec/design/tasks/apply/verify/QA/inventory/RPI files — exit 0; 0 issues. Final `git diff --check` — exit 0.

## Fase 12 — U7: contrato v2 cerrado (offline)

Se completaron solo 12.1–12.3. Rama `muninn`, sin branch/base nuevos, commits ni PR.
Se conservó el WIP inicial mediante ediciones selectivas. El presupuesto se aplica al delta
nuevo, no al diff acumulado: 400 líneas; no se invoca la excepción histórica para esta unidad.
El contrato comparte wrappers, hash y corroboración; especializa cinco valores v2 cerrados.
V1 conserva sus cuatro combinaciones (tests existentes de reglas/gates, incluido ambos disabled),
la plantilla y el candidato. La aprobación sintética no aprueba ni ejecuta documentos reales.

### Evidencia TDD y comandos ejecutados

Todos los comandos unittest usaron `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn`.
El runner y `jsonschema` estaban operativos; no se instalaron dependencias.

- Baseline: `python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_protocol.py'`
  — exit 0, 16 tests, 17.171 s.
- RED: mismo comando tras añadir tres tests v2 y antes de implementar — exit 1,
  19 tests, 15.251 s, `FAILED (errors=11)`. Fallo esperado:
  `ValueError: unsupported protocol schema_version`; no fallo de entorno/sintaxis.
- GREEN inicial: mismo comando — exit 1, 19 tests, 51.665 s, un fallo:
  Python rechaza NaN pero el CLI `jsonschema` acepta NaN para threshold acotado.
  Se reprodujo separadamente con schema number mínimo 0/máximo 1: CLI exit 0.
  NaN/Infinity no son JSON; el harness usa ahora `allow_nan=False` y exige `ValueError`
  para esos inputs, no simula un resultado del subprocess. Los casos JSON continúan
  contrastándose con el CLI real. No se afirma rechazo de NaN por ese CLI.
- GREEN: mismo comando focalizado — exit 0, 19 tests, 46.559 s.
- Regresión amplia: `python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py'`
  — exit 0, 93 tests, 49.801 s.
- REFACTOR: formato legible de ramas/schema y guía de compatibilidad; mismo discovery amplio
  — exit 0, 93 tests, 49.402 s. Se conservan fórmula/piloto/procedencia anteriores.
- `pnpm exec markdownlint-cli2 docs/muninn/retrieval-h5-runner.md` — exit 0, 0 issues.
- `git diff --check` — exit 0, sin salida.

Los tests cubren tokens, K/threshold/corridas, representación integral (`10.0`/`1.0`),
booleanos/no finitos, dominios/search_config/hash, campos missing/pending/extras y
mutación de cada hoja/versión contra corroboración previa. Prosa informativa no se ejecuta.
La igualdad del corpus_hash con un corpus y el flujo ejecutable pertenecen a unidades futuras.

No se modificaron state.yaml, informes QA/verify, corpus/candidato/plantilla, Rust ni Docker.
No se ejecutaron builds, Clippy, modelos, red, bridge vivo, DB, C0 ni mediciones.
No se ejecutó CI completa; QA/H5, freeze/review y Clippy histórico siguen bloqueados.
Fases 13–17 permanecen pendientes. Próximo paso: revisión de U7 y luego U8/fase 13.

Delta U7 final: 380 líneas (adiciones + eliminaciones), seis archivos, incluyendo este registro
y tres casillas; bajo 400. `git diff --no-index --numstat` contra copias previas en el temporal
aprobado permitió medir también los archivos WIP no tracked. Lint final de tasks/apply/guía:
`pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/{tasks,apply-progress}.md docs/muninn/retrieval-h5-runner.md`
— exit 0, 0 issues; `git diff --check` — exit 0. No se subdividió U7 por no superar presupuesto.

## Corrección U7 tras revisión independiente — pendiente nueva revisión

Se verificaron los tres hallazgos antes de corregir, con `receiving-code-review` y TDD.
Este delta es una subunidad correctiva de U7: gates v2 cerrados, hash sin sufijo y rechazo
numérico total. No inicia U8 ni fases 13–17. El conteo anterior de 380 líneas describe
solo la primera entrega; no se invoca la excepción histórica para acumular correcciones.
El delta correctivo se revisa separado (tres archivos ejecutables/tests y este registro).

### RED observado

Se añadieron tres regresiones en `test_retrieval_h5_protocol.py` antes de modificar producción:

- `test_v2_gate_matrix_is_closed_without_changing_v1`
- `test_v2_corpus_hash_rejects_trailing_newline_in_both_validators`
- `test_v2_bounded_fields_reject_huge_integers_without_exception`

Comando focalizado: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest`
seguido de los tres nombres completos `test_retrieval_h5_protocol.ProtocolTests.<nombre>` anteriores.
Resultado RED: exit 1, 3 tests, 0.872 s, `FAILED (failures=4, errors=12)`.
Tres combinaciones v2 no soportadas devolvían ready; el hash con salto final pasaba schema;
los doce casos ±`10**400` en seis campos acotados lanzaban `OverflowError` desde `math.isfinite`.
Repro adicional de las cuatro combinaciones: Python ready=True en todas; schema exit 0 solo
para ANN=true/relevancia=false, exit 1 para las otras tres. Los hallazgos aplican.

### GREEN y regresión

Python informa ahora `malformed` en `gates.<gate>` para combinaciones v2 no soportadas;
el schema expresa explícitamente la misma pareja. V1 y sus tests de cuatro combinaciones
no cambian. El hash schema exige exactamente 71 caracteres. La validación numérica acotada
comprueba el rango antes de convertir floats y no pasa enteros enormes a `math.isfinite`.

- Mismo comando focalizado: exit 0, 3 tests, 7.357 s.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_protocol.py'`
  — exit 0, 22 tests, 54.204 s.
- El primer discovery amplio compartió límite de 120 s con ambos comandos anteriores;
  fue interrumpido por timeout (exit 143), no se contabiliza como éxito.
- Reejecución independiente: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py'`
  — exit 0, 96 tests, 56.364 s.
- `git diff --check` — exit 0, sin salida.

Sin cambios en tasks (12.1–12.3 implementadas; revisión no equivale a implementación), estado,
QA/verify, candidatos, corpus, C0 ni otros WIP. No se ejecutaron builds/CI/Clippy/modelos/red,
Docker, bridge vivo, DB o mediciones. U7 corregida queda pendiente de nueva revisión independiente;
no se declara revisada ni aceptada y no habilita U8 automáticamente.

Lint del registro correctivo: `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md`
— exit 0, 0 issues. `git diff --check` final — exit 0. La subunidad correctiva queda
acotada a tres regresiones, dos ajustes Python, dos restricciones schema y este registro;
su tamaño permanece inferior al presupuesto de 400 líneas.

## Fase 13 — U8: elegibilidad previa, procedencia final intacta

Se implementaron solo 13.1–13.3 tras el PASS independiente de U7 comunicado por el usuario.
Se cargaron `sdd-apply` y `test-driven-development`; se revisaron tareas/spec/design vigentes,
instrucciones y WIP. Sin nuevas ramas, PR ni commits; se preservó la rama `muninn`.

`assess_preflight_provenance(gate, observations)` exige `LocalObservations`, comprueba el
baseline previo derivado del final y devuelve `eligible`/`blocked`, no autorización ni `verified`.
El chequeo de registros se comparte internamente; `assess_gate_provenance` conserva firma,
clasificación, baseline completo y correlación. `collect_observations` permanece intacto.
Los cinco registros posteriores no se fabrican. `effective_config` previo exige observación
externa disponible, sin completar su parte dependiente de respuesta; configuración final completa
sigue obligatoria. Igualdad de valores con protocolo/corpus/run y autorización son responsabilidad
de U9 futura, no se confunden con presencia de registros. `LocalObservations` no es atestación.

### Comandos y evidencia TDD observada

Todos los unittest usaron `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn`.

- Baseline: `python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_provenance.py'`
  — exit 0, 8 tests, 0.004 s.
- RED: mismo comando tras añadir tres tests y antes de producción — exit 1,
  11 tests, 0.006 s, `FAILED (errors=3)`: `ImportError: cannot import name 'assess_preflight_provenance'`.
  Es la API requerida aún ausente, no falta de runner/dependencia.
- GREEN mínimo: mismo comando — exit 0, 11 tests, 0.022 s.
- REFACTOR/documentación: mismo comando — exit 0, 11 tests, 0.023 s.
- Regresión amplia: `python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py'`
  — exit 0, 99 tests, 54.128 s; incluye protocolo/schema subprocess y U7.

Las nuevas pruebas sintéticas verifican ausencia de evidencia futura sin mutación, falta/conflicto
y cada metadato requerido de los 16 registros previos, rechazo de claims planos, gate desconocido,
redacción, requisitos exclusivos de relevancia y baseline final sin finished_at/resultados/binding/
identidad loaded/configuración. Registros futuros no sustituyen un comienzo ausente. Los tests
previos de collector/correlación permanecen y pasan; no se ejecuta Docker real.

No se implementaron fases 14–17 ni se cambiaron estado, informes QA/verify, corpus/candidatos,
C0, aprobaciones o código runner/Rust. Sin builds amplios, CI, Clippy, modelos, red, bridge vivo,
DB o mediciones. No hay aceptación H5; QA, freeze/review y bloqueos históricos siguen abiertos.
Próximo paso: revisión independiente U8 antes de U9; estas pruebas no verifican el flujo futuro.

Checks finales: `pnpm exec markdownlint-cli2 docs/muninn/retrieval-h5-runner.md .agents/sdd/changes/muninn-h5-acceptance-protocol/{tasks,apply-progress}.md`
— exit 0, 0 issues. `git diff --check` — exit 0. Delta U8: 188 líneas modificadas (adiciones + eliminaciones) en cinco archivos, medido con `git diff --no-index --numstat`
contra copias previas U8 en temporal aprobado; bajo 400, sin excepción ni subdivisión necesaria.

## Corrección U8 — tipos de metadatos y conflict (pendiente revisión independiente)

Se cargaron `receiving-code-review` y `test-driven-development`. Se verificó el helper compartido:
aceptaba metadatos truthy no string y flags conflict no booleanos. Se añadieron regresiones antes
de cambiar producción. Solo se corrigió U8; no se implementan U9/fases 14–17.

`source`, `method` y `observed_at` requieren strings con contenido tras `strip()`; no se modifican
los valores ni se impone formato timestamp o política temporal. `conflict` ausente sigue permitido;
si está presente debe ser booleano y True bloquea. `value` conserva su validación y polimorfismo.
Ambas APIs usan el helper endurecido sin cambiar baselines ni correlación final.

### Evidencia observada y comandos

Comando focalizado:
`PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest`
seguido de estos dos nombres completos:

- `test_retrieval_h5_provenance.ProvenanceTests.test_record_metadata_types_and_conflict_flag_fail_closed_in_both_apis`
- `test_retrieval_h5_provenance.ProvenanceTests.test_valid_record_metadata_preserves_values_and_optional_boolean_conflict`

RED: exit 1, 2 tests, 0.009 s, `FAILED (failures=19)`: casos malformados devolvían `eligible`.
GREEN: mismo comando, exit 0, 2 tests, 0.009 s. Las regresiones comprueban también rechazo
final, whitespace vacío, flag opcional/False/True y preservación de valores válidos polimórficos.

- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_provenance.py'`
  — exit 0, 13 tests, 0.031 s.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py'`
  — exit 0, 101 tests, 59.390 s.

Se preservaron todos los WIP. Sin cambios en tasks, state, QA/verify, corpus/candidatos, C0,
aprobaciones o PR. Sin builds/CI/Clippy, modelos, red, Docker, bridge vivo, DB ni mediciones.
Esta subunidad correctiva es menor de 400 líneas; no invoca la excepción histórica.
U8 queda pendiente de nueva revisión independiente; no se declara aprobada ni aceptación H5.

Checks del registro: `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md`
— exit 0, 0 issues. `git diff --check` — exit 0, sin salida.

## Fase 14 — U9 bloqueada antes de código por contrato de evidencia pendiente

Se recibió PASS independiente de U8. Se cargaron `sdd-apply` y `test-driven-development`
y se leyeron tareas 14.1–14.3, spec y diseño vigentes, runner y contratos de observaciones.
No se modificó código ni se marcaron tareas. No hay evidencia RED/GREEN nueva.

Ambigüedad material: el diseño exige un registro freeze/review cerrado con identidad/version/hash,
estado congelado y referencia de revisión, y evidencia de almacenamiento desechable aislado e
inventario para ese run. No especifica nombres de registros, propiedades exactas, tipo/token del
estado congelado ni representación del inventario. Tampoco concreta qué estructura comparar en
`effective_config` previo y `operator_corroboration` para la correspondencia protocolo/corpus/run.
Los helpers actuales comprueban registros, pero no definen esos valores: sus fixtures usan strings
sintéticos y no pueden convertirse en política normativa. Inventar esas formas en apply fijaría un
contrato de autorización que el usuario exige resolver antes de implementación.

Se requiere aclarar en diseño el contrato cerrado de esos registros y sus comparaciones, incluyendo
si el inventario representa almacenamiento vacío previo, candidatos previstos o ambos. La API final
está definida, pero también falta fijar la salida intermedia sin efectos al superar U9 antes de U10 (estado eligible de preflight frente a gate aún no evaluado). No se implementa flujo futuro para
resolverlo implícitamente. Tras aclarar, comenzar U9 con RED observable y límite de 400 líneas.

Sin cambios de estado/QA/verify, corpus/candidatos, C0, aprobaciones o WIP previo. Sin red, Docker,
modelos, bridge vivo, DB, mediciones, builds, CI ni Clippy. No se ejecutaron suites porque no hay
cambio ejecutable; no se afirma regresión nueva ni aceptación H5. Próximo paso: aclaración del
contrato de evidencia U9 por responsable de spec/design; después reanudar apply, no U10.

## U9-A — subunidad local verificable; fase 14 todavía parcial

El bloqueo anterior quedó resuelto upstream. Se cargaron `sdd-apply` y TDD, leyendo enmienda
spec 675–850, diseño 437–630 y tareas 14 actualizadas antes de código. La matriz completa de
U9 más cuatro validadores especializados/API excedería previsiblemente 400 líneas; se separó
por comportamiento sin excepción histórica. Esta entrega cubre solo la frontera local pura:
protocolo/corroboración, corpus/scope/pares, límite y override. El helper privado devuelve
contexto interno, no `eligible` público ni autorización. No hay API parcial que simule U9 completa.

Se reutilizan validadores/hash/corroboración existentes. Fixtures: dos consultas sintéticas con
diez candidatos cada una, IDs compartidos con texto coherente y aprobación exclusivamente de test.
Los spies rechazan filesystem, bridge HTTP, piloto, métricas y relevancia en ambos caminos locales.
No se alteran v1/piloto/U7/U8 ni se copian observaciones esperadas como evidencia.

### Evidencia RED/GREEN observada

- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolLocalPreflightTests`
  RED: exit 1, 2 tests, 0.019 s, 23 errores por import del helper requerido aún ausente.
- Mismo comando tras implementación mínima: exit 1, 2 tests, 0.073 s, 22 errores:
  el resultado interno incluía un set vacío no serializable en pruebas negativas. Se cambió
  la representación de pares a lista ordenada, sin cambiar los pares ni ajustar el criterio.
- Mismo comando GREEN: exit 0, 2 tests, 0.050 s (22 escenarios negativos con subtests).
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py'`
  — exit 0, 103 tests, 55.718 s; incluye contrato/schema subprocess, U7/U8 y piloto.

### Pendientes explícitos

14.1–14.3 conservan casillas abiertas: faltan API/port, proveedor sintético, cuatro registros
cerrados, bindings de evidencia, callbacks/spies/centinela de la API pública y sus informes exactos.
Falta completar matriz numérica/especializada y paths públicos seguros según diseño. El helper
interno conserva grupos estructurales originales; la futura API no debe exponer copias de trabajo
ni claves desconocidas sin comprobar la política de paths. No se afirma U9 lista ni autorizada.
Próximo paso: revisar U9-A y continuar U9-B por evidencia/API; no U10.

No se modificaron tasks (ninguna tarea completa), state, QA/verify, corpus/candidatos reales,
C0 o aprobaciones. Sin PR, modelos, red, Docker, bridge vivo, DB, mediciones, builds/CI/Clippy.
Se preservó el WIP. No se declara aceptación H5.

Checks de cierre U9-A: `git diff --check` — exit 0, sin salida.
`pnpm exec markdownlint-cli2 docs/muninn/retrieval-h5-runner.md .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md`
— exit 0, 0 issues. Delta de esta subunidad medido con `git diff --no-index --numstat`
contra copias previas en el temporal `h5-u9a`: 248 líneas añadidas/eliminadas en cuatro archivos (242 antes de estas seis líneas de registro); no es el tamaño del WIP acumulado.

## U9-B1 — port/API y registros especializados; matriz final pendiente

U9-A recibió PASS independiente del usuario. Se cargaron `sdd-apply` y TDD y se leyeron
spec 675–850, diseño 437–630 y fase 14 vigente. Se preservó todo WIP previo.
Se implementaron `ProtocolRunEvidence` y `run_protocol_evaluation`: validación local U9-A,
adquisición única con copias separadas, exigencia de LocalObservations, baseline U8 intacto,
cuatro envolturas/valores cerrados y bindings. Ambos resultados terminan sin efectos ni U10.
Informe público por selección explícita; no publica working copies, observaciones ni excepciones.

### Comandos y evidencia observada

- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolEvidencePreflightTests`
  RED: exit 1, 2 tests, 70 errores, 0.086 s: API ausente. GREEN: exit 0, 2 tests, 0.177 s.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolEvidencePreflightTests.test_public_protocol_findings_do_not_expose_unknown_dotted_keys`
  RED adicional: exit 1, 1 test, 0.060 s: clave desconocida con puntos filtrada en path.
  Corrección: resolver contenedor por estructura, no separar el último segmento textual.
- Se añadieron caracterizaciones de envolturas, campos, tipos no JSON, booleanos, no finitos,
  enteros enormes, conflicto opcional falso y orden del plan tras el primer GREEN.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolEvidencePreflightTests test_retrieval_h5_runner.ProtocolLocalPreflightTests`
  — exit 0, 6 tests, 0.421 s, incluye regresión del path y matriz de subtests.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py'`
  — exit 0, 107 tests, 56.803 s; incluye U7/U8 y piloto.

Las fixtures se construyen antes del callback y no derivan observaciones de sus argumentos.
Spies comprueban cero bridge/callbacks posteriores/lecturas/escrituras/mkdir/métricas/piloto;
centinela intacto, inputs/observaciones inmutados. Fallos locales omiten before_run; adquisición
OSError/ValueError/RuntimeError y dict plano bloquean sin mensaje crudo. Freeze usa revisión distinta.

14.1–14.3 siguen abiertas: falta completar matriz pública de todos los rechazos locales U9-A,
aserciones exhaustivas de categoría/path y determinismo/duplicados, mutación tras retorno y bindings
válidos pero discordantes campo por campo. No se afirma cobertura completa ni cierre de U9.
Esta división mantiene código, pruebas y documentación juntos dentro de 400 líneas; no usa excepción.
No se modificaron U8/final, candidatos/corpus reales/C0/PR/aprobaciones/state/QA/verify.
Sin modelos/red/Docker/mediciones/builds/Clippy/CI. Próximo: revisión independiente U9-B1 y cierre
de matriz U9-B2, no U10. No hay aceptación H5 ni autorización real.

Checks finales: `git diff --check` — exit 0. Markdownlint de guía, tasks y este registro:
`pnpm exec markdownlint-cli2 docs/muninn/retrieval-h5-runner.md .agents/sdd/changes/muninn-h5-acceptance-protocol/{apply-progress.md,tasks.md}`
— exit 0, 0 issues. Delta por `git diff --no-index --numstat` frente a copias previas `h5-u9b`:
360 líneas añadidas/eliminadas en cinco archivos (355 antes de estas cinco líneas).

## Apply progress — muninn-h5-acceptance-protocol

### Alcance y límites

Aplicación incremental de U9/fase 14. Se conserva la implementación U9 existente; esta continuación añade únicamente tests públicos en `dev/muninn/tests/test_retrieval_h5_runner.py`. No se marcan tareas 14.1–14.3: la matriz literal completa aún requiere comprobación y revisión independiente. U10/U11, ejecución, autorización real, corpus/candidatos reales y C0 permanecen bloqueados.

Las fases previas del historial U7/U8 y las subunidades previas de U9 se documentaron en el registro de apply anterior. La evidencia más reciente antes de esta subunidad fue U9-B2: suite `test_retrieval_h5_runner.py` con 33 tests y descubrimiento `test_retrieval*.py` con 115 tests, ambos OK. Se mantienen los informes históricos QA/verify y el estado existente.

## U9 — pruebas de cobertura adicionales

### Cambios y evidencia TDD

Se añadió cobertura pública para los gaps comunicados tras la revisión PASS:

- `test_local_rejection_matrix_keeps_exactly_nine_candidates_and_valid_hashes`: en ambos turnos deja exactamente una query con 9 candidatos y la otra con 10; actualiza el scope/hash del corpus y el hash canónico del protocolo para aislar el rechazo del mínimo por query. Exige rechazo local exacto y cero adquisiciones.
- `test_specialized_conflict_deduplicates_identical_u8_finding`: la envoltura especializada en conflicto produce el mismo finding del validador U8 y la salida pública contiene una sola fila exacta.
- `test_public_protocol_findings_preserve_structural_groups`: casos missing, pending y malformed confirman grupos públicos con paths/reasons y cero adquisición; compara con el validador estructural. Para malformed se usa un tipo inválido de campo conocido, sin cambiar el path original mediante remapeo de claves desconocidas.
- `test_valid_specialized_metadata_is_preserved_without_time_policy`: los cuatro registros especializados admiten whitespace periférico en metadatos y `observed_at` no timestamp no vacío; el resultado es eligible y el test no introduce normalización ni política temporal.
- Se corrigió la aserción de identidad del proveedor para comparar sus argumentos con los objetos originales (`caller_protocol`/`caller_corpus`), en lugar de comparar con copias profundas. El snapshot independiente sigue usándose para verificar ausencia de mutación.
- `invoke` ahora verifica explícitamente la forma exacta de `preflight` y ambos estados de gate, tanto en positivo como negativo. No desactiva ni debilita ningún requisito.

TDD/evidencia RED→GREEN:

| Slice                                 | RED observado                                                                                                                                                                                                                               | GREEN observado                                                                                                     | Producción   |
|---------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------|--------------|
| Mínimo local 9 y grupos estructurales | Primera ejecución focalizada: 2 fallos de test. El setup inicial asumía siempre query 0 como la de 9, y malformed elegía una clave extra cuya ruta pública se redacciona intencionadamente al contenedor. No fueron defectos de producción. | Tests ajustados a la query parametrizada y a un tipo inválido conocido; ejecución focalizada posterior: 4 tests OK. | Sin cambios. |
| Dedupe de findings                    | El test explícito resulta observable con conflicto de envoltura que coincide con el finding U8.                                                                                                                                             | Resultado público: una única fila exacta.                                                                           | Sin cambios. |
| Metadatos sin regla temporal          | No se encontró fallo de comportamiento; la prueba es de caracterización de aceptación contractual.                                                                                                                                          | Eligible con espacios conservados y valor de tiempo arbitrario no vacío.                                            | Sin cambios. |

### Comandos y resultados

- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolEvidencePreflightTests.test_local_rejection_matrix_keeps_exactly_nine_candidates_and_valid_hashes test_retrieval_h5_runner.ProtocolEvidencePreflightTests.test_specialized_conflict_deduplicates_identical_u8_finding test_retrieval_h5_runner.ProtocolEvidencePreflightTests.test_public_protocol_findings_preserve_structural_groups test_retrieval_h5_runner.ProtocolEvidencePreflightTests.test_valid_specialized_metadata_is_preserved_without_time_policy -v` — primera ejecución exit 1 (2 fallos de aserción del test); tras corregir solo el setup/test, exit 0, 4 tests OK.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolEvidencePreflightTests.test_provider_inputs_and_returned_observations_do_not_alias_callers_or_report -v` — exit 0, 1 test OK.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_runner.py' -v` — exit 0, 37 tests OK.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py' -v` — exit 0, 119 tests OK.
- `python3 -m py_compile dev/muninn/tests/test_retrieval_h5_runner.py dev/muninn/retrieval_h5_runner.py dev/muninn/retrieval_h5_protocol.py dev/muninn/retrieval_h5_provenance.py` — exit 0.
- `git diff --check` — exit 0.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md docs/muninn/retrieval-h5-runner.md` — exit 0, 0 issues.

## Estado restante de la matriz literal

No marcar tasks hasta revisar como mínimo la cobertura pública U9 requerida en 14.1–14.3 y contrastar todos los criterios MUST de spec/design:

- ✅ Rechazo local aislado con 9 candidatos, hash coherente, scope y operador coherentes; cubierta query 0 y query 1.
- ✅ Dedupe real U8/especializado por conflicto idéntico.
- ✅ API pública preserva missing/pending/malformed, con paths/reasons.
- ✅ Metadatos especializados válidos, sin normalización y sin política temporal nueva.
- ✅ Forma exacta de preflight/gates y results positivos/negativos comprobada en API invoke.
- ✅ Identidad comparada con argumentos originales.
- ⚠️ Confirmar mediante inspección de matriz que cada categoría/path local y especializada, fuente de error/edge estructural, no-efectos y mutación de inputs está cubierta sin falsos positivos. No se ha re-auditado exhaustive checklist completa en esta subunidad.
- ⚠️ Suites de `test_retrieval*.py`, protocol/provenance, sintaxis/compilación, `git diff --check` y Markdownlint pendientes para el cierre de esta subunidad.

No se ejecutaron CLI/flags, corpus/candidatos reales, C0, approvals, cambios de state/QA/verify, modelos, red, Docker/bridge vivo, DB, mediciones, builds, Clippy ni CI. No se modificó producción, protocolo U7/U8 o los reportes históricos.

### U9 — continuación de cobertura confirmada (snapshot recuperado)

Se continuó solo sobre tests públicos `raw`/`invoke` en `test_retrieval_h5_runner.py`; no hubo cambios de producción. Se recuperó historial de apply hasta el snapshot anterior que termina en la línea 613. No se reconstruyen subsecciones anteriores no presentes en ese snapshot. La afirmación previa de que `raw` conservaba spies completos de filesystem/métricas/piloto no era correcta: esos spies estaban instalados en `invoke`, pero no en `raw`. La continuación actualiza el harness `raw` para compartir explícitamente el instalador de spies; no se afirma prefix byte-exact del archivo actual tras la edición de headings. U9 permaneció parcial hasta la revisión formal final; tras cerrar el último hallazgo, el revisor
recomendó completar 14.1–14.3 únicamente para el contrato offline. Esto no equivale a aceptar H5
ni autoriza U10.

### Casos añadidos y evidencia

- `assert_blocked_u9_report` verifica el contrato exacto en la ruta `raw` para cada rechazo: preflight `{status: blocked, findings: nonempty}`, ANN–exact `{status: blocked, results: {}}`, relevancia `{status: not_evaluated, results: {}}` y `measurement_started=false`. `invoke` también verifica el mismo contrato. En la revisión formal se encontró que `raw` no instalaba inicialmente los spies comunes; la continuación posterior documenta y verifica la corrección.
- Cada registro obligatorio retornado por `ProvenanceTests.prior_observations()` se elimina o marca conflictivo individualmente. La API pública debe bloquearlo con finding seguro del registro; se permiten otros findings derivados de binding que también falte. La fixture es sintética y no sustituye la comprobación U8.
- Corpus con consulta/candidato ausente, extra o duplicado se prueba mediante `run_protocol_evaluation`. Para aislar las discrepancias, los cambios de scope reciben hash de protocolo y observaciones de hash/corroboración coherentes. Las seis variantes bloquean antes del proveedor. Un duplicado imposible de hashear de forma válida en `corpus_hash` no falsifica el contrato: el duplicado se expresa en el scope, manteniendo el corpus validado.
- El proveedor captura las copias internas en `before_run`; se comprueba igualdad exacta con los argumentos originales. Un registro especializado preserva metadatos con espacios periféricos. No se añadió API pública ni se modifica/norma ninguna entrada. La observación de que el runner duplica los inputs antes del callback es indirecta por la frontera pública; no se inspecciona memoria interna por API adicional.

RED/GREEN: los primeros comandos focalizados fallaron por aserciones/setup del test, no por defecto productivo confirmado: se esperaba un único finding cuando bindings secundarios válidos también reportan mismatch; las variantes extra inicialmente priorizaban límite de scope; y la copia de protocolo de la fixture no era aprobable tras alterarla. Se corrigió el aislamiento/aserciones, sin cambiar producción. No se reclama RED de producto ni defecto reproducido. Las variantes finales pasan como caracterización/regresión contractual.

### Comandos y reconciliación de checks

- RED inicial de tres tests focalizados (`test_each_required_u8_record_missing_or_conflicting_blocks_public_preflight`, `test_corpus_missing_extra_duplicate_block_after_recomputing_protocol_hashes`, `test_provider_copy_preserves_input_values_without_normalization`) — exit 1 por 9 fallos de expectativas y 2 errores de setup; no eran fallos de producción. Tras aislar condiciones y ajustar assertions, mismo comando — exit 0, 3 tests OK.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_runner.py' -v` — exit 0; 40 tests OK.
- Pendientes al inicio de esta continuación: suites `test_retrieval*.py`, compilación Python, diff y Markdownlint (snapshot anterior, líneas 600–613). Se ejecutarán y reconciliarán abajo; no se usa el recuento histórico de 37/119 como evidencia de esta edición.

No se modifican `retrieval_h5_runner.py`, U7/U8, state/QA/verify, corpus/candidato/C0/approvals ni documentación funcional. Sin CLI/flags, modelos, red, Docker/bridge vivo, DB, mediciones, builds, Clippy ni cambios de estado. No se marcan tareas 14.1–14.3.

### Reconciliación final de esta continuación

- Discovery `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py' -v` — exit 0; 122 tests OK (incluía las 3 regresiones presentes en ese snapshot). Esto reemplaza para ese snapshot el total previo de 119, no los resultados históricos.
- Suite runner sin filtro — exit 0; 40 tests OK en ese snapshot.
- `python3 -m py_compile dev/muninn/tests/test_retrieval_h5_runner.py dev/muninn/retrieval_h5_runner.py dev/muninn/retrieval_h5_protocol.py dev/muninn/retrieval_h5_provenance.py` — exit 0, sin salida.
- Primera ejecución de Markdownlint — exit 1, un MD025 por el encabezado H1 recuperado en medio del apply-progress parcial. Se subordinó solo ese encabezado y los títulos de esta continuación; no se reconstruyeron subsecciones faltantes. Reejecución `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md docs/muninn/retrieval-h5-runner.md` — exit 0, 0 issues.
- `git diff --check` — exit 0, sin salida.

## U9 — correcciones de regresión y harness raw

### Cambios y evidencia

- Se añadieron regresiones públicas en `run_protocol_evaluation` para cuerpo de corpus con query duplicada y, por separado, candidate duplicado. Ambos casos esperan exactamente `[{"path": "corpus", "reason": "corpus_mismatch"}]`, `before_run` cero veces y el shape bloqueado de U9. El hash/corroboración y los registros de evidencia mantienen sus valores válidos originales: la validación del corpus debe fallar antes de calcular su hash; no se inventó hash para un cuerpo inválido.
- `raw` e `invoke` comparten `install_no_effect_spies`: bridge (asserted en el mock), callbacks posteriores, helpers de piloto/annotations/compose, serializadores/métricas/gates y filesystem reads/writes/mkdir. En `raw` no se toman snapshots recursivos; siguen cubiertos inputs cíclicos/profundos.
- No se modificó código de producción. El primer test de corpus duplicado pasó al añadirse (caracterización de una conducta existente, no falla de producción). Un intento exploratorio de verificar el spy con ruta inexistente falló por `FileNotFoundError`, así que se descartó; no se reporta como RED. Los ciclos TDD precedentes en este slice cubrieron harness/cobertura, no un defecto productivo.

### Checks actuales, en orden cronológico

- Suite runner sin filtro: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_runner.py' -v` — exit 0; 42 tests OK.
- Discovery completo: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py' -v` — exit 0; 124 tests OK.
- `python3 -m py_compile dev/muninn/tests/test_retrieval_h5_runner.py dev/muninn/retrieval_h5_runner.py dev/muninn/retrieval_h5_protocol.py dev/muninn/retrieval_h5_provenance.py` — exit 0.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md docs/muninn/retrieval-h5-runner.md` — exit 0, 0 issues.
- `git diff --check` — exit 0.

Límite de recuperación: el historial disponible se recuperó solo hasta el snapshot, no se reconstruyen secciones faltantes. No se afirma que el contenido recuperado sea un prefix byte-exact del archivo actual tras editar encabezados. U9 sigue parcial; tareas 14.1–14.3 continúan abiertas hasta nueva revisión formal. No se ejecutó state/QA/verify, U10, CLI/flags, corpus/candidatos reales/C0, aprobación/PR, modelos/red/Docker/DB/mediciones/builds/Clippy.

Quedan residuales de U9: matriz completa de paths y razones, mutación/conflicto/categoría para combinaciones todavía no individualizadas, y revisión formal final. No se hace claim de cierre U9, aprobación del protocolo, QA, medición ni acceptance.

## U9 reviewer final finding — `raw` harness correction

### Status

Corrected only the `ProtocolEvidencePreflightTests.raw` harness and added a focused regression test. No production code, tasks, state, QA, Verify, corpus, approval, or CLI files were changed for this correction.

### Executive summary

Earlier text in this log (U9, lines 649–650) incorrectly stated that `raw` and `invoke` both installed `install_no_effect_spies`. The actual `raw` implementation had omitted the installer. This entry corrects that historical statement without replacing or deleting it. The earlier discovery result of 124 tests is historical evidence for its prior snapshot only; it is not represented as the current total.

`raw` now installs the existing shared spies inside `ExitStack`, using the same installer as `invoke`. It does not recursively snapshot inputs. Its filesystem sentinel is created before entering the patched context and read only after the context exits. Bridge `post` and post-response/after-run callbacks remain asserted independently.

The regression test drives `run_protocol_evaluation` through `raw` with a blocked protocol evidence record (`corpus_freeze_review.status = "draft"`). Its fake `before_run` calls the real imported `gate_result` while the `raw` spies are active. The test requires the gate spy's `AssertionError`. Thus the spy check is exercised during real raw-run execution, not by calling a patched function directly from the test body.

### TDD evidence

- RED: temporarily omitted `self.install_no_effect_spies(stack, patch)` from `raw` and ran `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolEvidencePreflightTests.test_raw_installs_effect_spies_around_real_blocked_execution -v`. Exit 1; the assertion reported `AssertionError not raised`, confirming the missing installer was the cause.
- GREEN: restored the installer and reran the focused test. Exit 0; 1 test passed.
- REFACTOR/current validation: runner suite and retrieval discovery passed below. No recursive snapshots were added.

### Effect coverage and sentinel boundary

The existing installer covers pilot execution, annotations, report composition, gate/metric serializers and helpers, protocol preflight, and filesystem open/read/write/mkdir calls. The `raw` harness separately asserts no bridge `post`, `after_response`, or `after_run`. Sentinel setup and post-context verification use actual filesystem operations outside the patched context; no `unittest.mock.patch` state is inspected. No false-positive issue was observed.

### Artifacts and exact verification

- `dev/muninn/tests/test_retrieval_h5_runner.py` — corrected `raw`; added `test_raw_installs_effect_spies_around_real_blocked_execution`.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolEvidencePreflightTests.test_raw_installs_effect_spies_around_real_blocked_execution -v` — exit 0; 1 test passed after GREEN.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_runner.py' -v` — exit 0; 43 tests passed.
- `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py' -v` — exit 0; 125 tests passed. The 124 count above is retained as historical prior-snapshot evidence.
- `python3 -m py_compile dev/muninn/tests/test_retrieval_h5_runner.py` — exit 0.
- `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md` — exit 0; 0 issues.
- `git diff --check` — exit 0; no output.

### Next recommended

Independent review may reassess this harness finding. Do not infer U9 closure, mark tasks, or advance U10 from this correction alone.

### Risks and boundaries

The test proves that this harness activates and observes the existing spies for an intentionally attempted gate effect while blocked evidence is being acquired. It does not establish production readiness or acceptance. No production code was touched. No task checkbox, state, QA/Verify record, corpus/C0, approval, U10, or CLI work was performed.

## Fase 15 — U10: ejecución sintética y bindings por consulta

### Alcance y objetivos

Se completaron las tareas 15.1, 15.2 y 15.3 de la fase 15 (U10). Esta entrega añade la ejecución sintética aislada para el camino en el que el preflight resulta `eligible`:

- Inserción de candidatos mediante `bridge.post("/add_message", ...)`.
- Mapeo de aliases por consulta `(query_id, candidate_id)`, aislando los UUIDs asignados y rechazando cualquier UUID duplicado en toda la corrida.
- Codificación inequívoca de sesión mediante prefijo de longitud: `_session_id(run_id, query_id) -> f"muninn-h5-v2:{len(run_id)}:{run_id}:{len(query_id)}:{query_id}"`.
- Ejecución de exactamente una búsqueda diagnóstica por consulta vía `bridge.post("/h5/diagnostic/search", ...)`.
- Sanitización de respuestas con `sanitize_diagnostic`, validando rangos secuenciales, unicidad de aliases y correspondencia de identidad de encoder.
- Invocación de callbacks de evidencia: `evidence.after_response(run_id=..., query_id=..., diagnostic=...)` por consulta y `evidence.after_run(run_id=...)` al finalizar.
- Comportamiento fail-closed: tras el primer efecto, cualquier excepción de transporte, inserción, búsqueda, sanitización o callbacks produce un informe con `measurement_started = True`, `gates.ann_exact.status = "blocked"` y hallazgo seguro `{"path": "bridge_response", "reason": "response_invalid"}`, sin intentos de rollback destructivo ni filtración de UUIDs o texto crudo.
- Spies verifican cero llamadas a lectura de CSV/annotations, `compose_report` o `evaluate_query`.

### Comandos y evidencia TDD observada

1. RED (15.1):
    - Se añadió `FakeBridge` configurable y la suite `ProtocolExecutionTests` (11 tests) en `dev/muninn/tests/test_retrieval_h5_runner.py`.
    - Comando: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolExecutionTests`
    - Resultado: exit 1, 11 tests ejecutados, `FAILED (failures=10, errors=1)`. El error correspondió a la ausencia de `_session_id` y las 10 fallas correspondieron a la falta de ejecución de efectos tras preflight eligible.

2. GREEN (15.2):
    - Se implementó `_session_id` y el bloque de ejecución U10 dentro de `run_protocol_evaluation` en `dev/muninn/retrieval_h5_runner.py`.
    - Comando: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolExecutionTests`
    - Resultado: exit 0, 11 tests OK en 0.026 s.

3. REFACTOR y suites completas (15.3):
    - Se actualizó el arnés `invoke` en `ProtocolEvidencePreflightTests` para contemplar la transición de preflight eligible a ejecución con `FakeBridge` y `LocalObservations`, manteniendo inalterada la verificación de los 17 escenarios bloqueados (sin efectos y con `measurement_started = False`).
    - Comando runner: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner`
    - Resultado: exit 0, 54 tests OK en 0.944 s.
    - Comando discovery completo: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py'`
    - Resultado: exit 0, 136 tests OK en 55.817 s.
    - Compilación: `python3 -m py_compile dev/muninn/tests/test_retrieval_h5_runner.py dev/muninn/retrieval_h5_runner.py dev/muninn/retrieval_h5_protocol.py dev/muninn/retrieval_h5_provenance.py` — exit 0.

### Límites e invariantes preservados

- Delta de la fase 15 contenido dentro del presupuesto (~310 líneas entre runner y tests).
- Sin modificaciones a código Rust, Docker/Compose, C0, corpus real, `state.yaml` ni reportes históricos de QA/verify.
- No se emitieron escrituras en disco ni salidas persistidas; el reporte se mantiene en memoria.
- U11 (criterio ANN, serialización de métricas de gate y procedencia final) permanece pendiente para la fase 16.

## Fase 16 — U11: criterio, cobertura y procedencia final

### Alcance y objetivos

Se implementaron las tareas 16.1, 16.2 y 16.3 de la fase 16 (U11). Esta entrega completa la evaluación final del gate `ann_exact` y procedencia correlacionada tras la ejecución de búsquedas diagnósticas:

- Validación de cobertura total de consultas del corpus.
- Re-verificación de hashes canónicos de protocolo y corpus antes de emitir veredicto para detectar mutaciones concurrentes.
- Evaluación de métricas ANN por consulta (`ExactTopKRecall@10` y `OrderedTopKMatch@10`) mediante `serialize_protocol_gate_metrics`.
- Agregación macro descriptiva: media aritmética exacta de cada métrica únicamente si todas las consultas están definidas; de lo contrario `value: null` y conteo explícito en `undefined_count`.
- Verificación exhaustiva de procedencia final por consulta con `assess_gate_provenance("ann_exact", observations)`: cualquier registro faltante, conflictivo, binding erróneo (`response_run_id`, `response_query_id`) o mismatch de identidad clasifica como `insufficient` y bloquea el gate (`status: "blocked"`).
- Determinación del estado del gate con `gate_result`:
    - `PASS` si todas las métricas son 1.0 y la procedencia es `verified`.
    - `FAIL` si todas las métricas están definidas pero alguna es != 1.0 (e.g. orden distinto con recall completo).
    - `blocked` si alguna métrica es indefinida (e.g. exact vacío), cobertura incompleta o procedencia no verificada.
- `absolute_relevance` permanece inalterado en `{"status": "not_evaluated", "results": {}}` y nunca se emite un campo de aceptación global.

### Comandos y evidencia TDD observada

1. RED (16.1):
    - Se añadió la suite `ProtocolVerdictAndProvenanceTests` en `dev/muninn/tests/test_retrieval_h5_runner.py`, cubriendo PASS sintético, orden distinto -> FAIL, exact vacío -> blocked, underfill y empates de scores, macro descriptivo con indefinidos, procedencia adversa, cobertura de evidencia incompleta -> blocked, mutación de hashes y relevancia no evaluada.
    - Estado RED confirmado antes de la integración del veredicto: `FAILED (failures=8)` al devolver `ann_exact_gate` en estado no evaluado.
    - La regresión `test_incomplete_per_query_evidence_coverage_blocks_with_perfect_metrics` mantiene respuestas diagnósticas perfectas de ambas consultas, omite la evidencia final de una consulta a través del callback `after_response` y verifica el bloqueo con el predicado de cobertura real (sin patch). Específicamente cubre cobertura de evidencia por consulta; no simula que el bridge omita una llamada diagnóstica.

2. GREEN (16.2):
    - Se implementó la agregación de métricas, comprobaciones de cobertura, revalidación de hashes, evaluación de procedencia por consulta y llamada a `gate_result` en `run_protocol_evaluation`.
    - Comando focalizado: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest test_retrieval_h5_runner.ProtocolVerdictAndProvenanceTests`
    - Resultado: exit 0, 9 tests OK en 0.065 s.

3. REFACTOR y suites completas (16.3):
    - Saneamiento de `docs/muninn/retrieval-h5-runner.md` eliminando párrafos residuales obsoletos de U9 que contradecían U11.
    - Suite completa del runner: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn:dev/muninn/tests python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_runner.py'`
    - Resultado: exit 0, 63 tests OK en 0.921 s.
    - Discovery completo de retrieval: `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval*.py'`
    - Resultado: exit 0, 145 tests OK en 55.139 s.
    - Compilación: `python3 -m py_compile dev/muninn/tests/test_retrieval_h5_runner.py dev/muninn/retrieval_h5_runner.py dev/muninn/retrieval_h5_protocol.py dev/muninn/retrieval_h5_provenance.py dev/muninn/retrieval_quality.py` — exit 0.
    - Linter Markdown: `pnpm exec markdownlint-cli2 .agents/sdd/changes/muninn-h5-acceptance-protocol/apply-progress.md .agents/sdd/changes/muninn-h5-acceptance-protocol/tasks.md docs/muninn/retrieval-h5-runner.md` — exit 0, 0 issues.
    - Git diff check: `git diff --check` — exit 0.

### Límites e invariantes preservados

- El estado `PASS` en `ann_exact` es estrictamente una demostración de cumplimiento del contrato técnico offline sobre dobles sintéticos; no constituye certificación de hardware ni aceptación H5.
- Cero escrituras en disco o salidas persistidas; el reporte se mantiene en memoria.
