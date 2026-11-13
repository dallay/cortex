# Tareas: protocolo de aceptación H5

## Previsión de carga de revisión

| Campo | Valor |
|---|---|
| Presupuesto | 400 líneas modificadas (adiciones + eliminaciones) |
| Carga estimada | Alta: multiárea, Docker, runner y pruebas; probable >400 líneas |
| PR encadenadas recomendadas | No |
| Estrategia canónica | `size-exception`, aprobada explícitamente por el usuario |
| Excepción de tamaño | Aprobada: se permite superar las 400 líneas con la justificación indicada abajo |
| Balance | Mantener las unidades de trabajo (1) contrato/aprobación; (2) procedencia/evaluación; (3) integración/informe. Cada unidad incluye pruebas; no usar stack ni cadena de ramas. |

Justificación de la excepción: esta rama (`muninn`) contiene desarrollos de varios hitos. El usuario quiere mantener todo el trabajo en una sola rama por ahora y probarlo ampliamente antes de mergear a `main`. Por ello, la excepción permite exceder el presupuesto de 400 líneas; no implica crear ramas adicionales, encadenar PR ni crear una PR ahora.

Decision needed before apply: No
Chained PRs recommended: No
400-line budget risk: High

La estrategia `size-exception` queda aprobada por el usuario y resuelve la decisión antes de `apply`. El presupuesto sigue siendo 400 líneas; la excepción autoriza excederlo por la justificación documentada, manteniendo el plan organizado por unidades de trabajo.

## Fase 1: Contrato del protocolo (unidad 1)

- [ ] 1.1 RED: En `dev/muninn/tests/test_retrieval_h5_protocol.py`, probar esquema estricto, hash canónico/versionado, campos por gate y rechazo de ausencias/defaults.
- [ ] 1.2 GREEN: Crear `dev/muninn/retrieval_h5_protocol.py` y `dev/muninn/fixtures/retrieval-h5-protocol.schema.json`; no elegir valores de protocolo-run.
- [ ] 1.3 RED/GREEN: Probar e implementar rechazo de propiedades desconocidas y de opciones caller que reduzcan el baseline obligatorio.
- [ ] 1.4 REFAC: Simplificar validación; repetir pruebas.

## Fase 2: Aprobación y corroboración local (unidad 1; depende de Fase 1)

- [ ] 2.1 RED: Probar commit/hash/revisión del contenido aprobado, aprobación previa y corroboración del operador; claims sin corroborar bloquean.
- [ ] 2.2 GREEN: Registrar evidencia de proceso local en `retrieval_h5_protocol.py`, sin firmas ni claims criptográficos.
- [ ] 2.3 REFAC: Separar validación/corroboración; repetir pruebas.

## Fase 3: Colector de procedencia (unidad 2; depende de Fase 1)

- [ ] 3.1 RED: Probar en `dev/muninn/tests/test_retrieval_h5_provenance.py` inspect, hashes/IDs, mounts, config, corpus/run ID y evidencia ausente/contradictoria por gate.
- [ ] 3.2 GREEN: Crear `dev/muninn/retrieval_h5_provenance.py`; baseline fijo. Encoder/config cargados no establecidos => `partial`, nunca PASS.
- [ ] 3.3 REFAC: Separar observaciones de claims; retener origen/método/tiempo y redactar secretos; repetir pruebas.

## Fase 4: Evaluación independiente (unidad 2; depende de Fases 1–3)

- [ ] 4.1 RED: Probar en `dev/muninn/tests/test_retrieval_quality.py` independencia de gates y PASS solo con aprobación, protocolo completo, `verified` y regla satisfecha.
- [ ] 4.2 GREEN: Adaptar `dev/muninn/retrieval_quality.py` a entradas aprobadas; distinguir bloqueado/descriptivo/no evaluado/fallo; sin PASS global.
- [ ] 4.3 REFAC: Centralizar elegibilidad por gate; probar tratamiento preinscrito de resultados indefinidos.

## Fase 5: Preflight y runner (unidad 3; depende de Fases 1–4)

- [ ] 5.1 RED: Probar en `dev/muninn/tests/test_retrieval_h5_runner.py` que preflight incompleto bloquea medición, consulta al bridge, escrituras y PASS.
- [ ] 5.2 GREEN: Integrar componentes en `dev/muninn/retrieval_h5_runner.py`; gates/preflight preceden bridge, escritura y medición.
- [ ] 5.3 RED: Probar estados por gate y bloqueo si encoder/config no se establecen.
- [ ] 5.4 GREEN: Ajustar integración sin cambiar retrieval, Store, ranking, filtros, scope, threshold ni fallback.
- [ ] 5.5 REFACTOR: Simplificar flujo; repetir pruebas.

## Fase 6: Informe y compatibilidad (unidad 3; depende de Fase 5)

- [ ] 6.1 RED: Probar informe por gate, evidencia/faltantes, redacción y ausencia de PASS global.
- [ ] 6.2 GREEN: Implementar informe en `retrieval_h5_runner.py`; preservar corrida histórica descriptiva/no preinscrita/no aceptada.
- [ ] 6.3 REFAC: Probar que no se sobrescriban ni reclasifiquen informes históricos.

## Fase 7: Plantilla, documentación y verificación (depende de Fases 1–6)

- [ ] 7.1 Crear `dev/muninn/fixtures/retrieval-h5-protocol.example.json` como plantilla no ejecutable, con decisiones por corrida pendientes y sin valores aprobados.
- [ ] 7.2 Documentar gates, trust model, evidencia y bloqueos en `docs/muninn/retrieval-h5-runner.md`; no modificar `retrieval-h5-local-results.md`.
- [ ] 7.3 Ejecutar pruebas con el patrón documentado: `PYTHONPATH=dev/muninn python3 -m unittest discover -s dev/muninn/tests -p 'test_retrieval_h5_*.py' -v`; ejecutar también `test_retrieval_quality.py`.
- [ ] 7.4 Confirmar que no cambian APIs, crates, Store ni datos; no medir confirmatoriamente.

## Bloqueo explícito de medición confirmatoria

- [ ] B.1 BLOQUEADA hasta aprobar protocolo-run concreto: métricas/K, reglas/tolerancias, número/agregación, corpus, adjudicación, alcance ANN–exact e identidad/procedencia.
- [ ] B.2 Sin re-medición ni PASS hasta B.1 y `verified`; esto no bloquea el framework. No inferir del piloto, H4 `1e-9` o referencia Proposed.
