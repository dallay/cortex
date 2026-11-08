# Informe QA — `provider-quota-tracking`

## 1. Identidad y veredicto

- **Cambio:** `provider-quota-tracking` (absorbe `per-provider-quota-integration`, U1–U3 como fase 5).
- **Modo:** `openspec`. **Fase:** `qa`. **Fecha:** 2026-11-02.
- **Ejecución:** `fallback`; no existe `.agents/sdd/quality-runner.json` ni un ejecutor QA con envelopes versionados en el repositorio.
- **Veredicto QA:** **PASS WITH WARNINGS** para los escenarios de aceptación aplicables.
- **Gate Archive:** **AUTORIZADO CON ADVERTENCIAS**. No quedan escenarios acceptance-relevant en estado `BLOCKED` o `NOT TESTED`, ni hallazgos CRITICAL/P0/P1 abiertos.

Los escenarios de aceptación aplicables tienen evidencia conductual reejecutada en esta fase. La única advertencia aceptada es la incompatibilidad de toolchain del gate local de Semgrep, ya resuelta reejecutando el escaneo con Python 3.12. Este informe no declara aceptación del operador.

## 2. Fuentes y handoff técnico

Se leyeron los artefactos requeridos:

- `proposal.md`
- `specs/provider-quota-capture/spec.md`, `specs/provider-usage-quota/spec.md`, `specs/providers-ui/spec.md`
- `design.md`
- `tasks.md`
- `verify-report.md`
- `state.yaml`
- `apply-progress.md`
- `.agents/sdd/config.yaml`
- `.agents/AGENTS.md`, `.agents/contexts/rook.md`, `.agents/contexts/dashboard.md`
- `docs/rook/architecture.md` (contrato de elegibilidad de enrutamiento, para separar esa superficie de la advertencia del dashboard)

**Handoff de Verify:** `verify-report.md` informa **PASS WITH WARNINGS**. V-01 (falla de espacio en disco) está resuelto mediante `cargo clean` sobre artefactos regenerables; V-02 (incompatibilidad de toolchain de Semgrep) está resuelto reejecutando el gate con `--python 3.12`. No quedan hallazgos abiertos de esa fase.

QA no repitió `fmt`, Clippy, `cargo doc`, cobertura, trivy, gitleaks ni la CI completa: son controles técnicos entregados por Verify y no aceptación observable. QA sí reejecutó las suites enumeradas en la matriz.

## 3. Target, entorno, permisos y límites

- **Target:** captura y presentación de cuotas upstream de proveedor, y el resumen de uso por proveedor que las expone.
- **Entorno:** worktree local Linux, workspace Cargo, suite Vitest del dashboard, Docker disponible para E2E. Los providers y stores son fixtures controlados; no hay llamadas de red a proveedores reales.
- **Permisos y datos:** no se usaron credenciales externas ni datos persistentes del usuario. No hubo reset de datos. No hubo commit, push ni publicación en esta fase.
- **Límite de la observación:** la captura se ejercita con respuestas de proveedor controladas. No se realizó una llamada real a un proveedor externo, por no existir credenciales autorizadas en el entorno; la ausencia de esa verificación no altera el contrato porque la captura depende de respuestas ya observadas por Cortex.

## 4. Inventario de capacidades

| Capacidad                              | Estado       | Selección y motivo                                                                                                                |
|----------------------------------------|--------------|-----------------------------------------------------------------------------------------------------------------------------------|
| Captura de cuotas upstream (U1–U3)     | Tested       | Es el origen de los datos de la feature; sin ella la sección de observaciones no tendría insumo.                                  |
| Resumen de uso por proveedor (U5)      | Tested       | Expone la separación entre uso enrutado, 429 enrutados, umbrales locales y observaciones upstream.                                |
| Dashboard de cuota (fase 3 y U6)       | Tested       | Es la superficie visible donde el usuario puede confundir señales; es donde la feature se consuma.                                |
| Elegibilidad de enrutamiento por cuota | Out of scope | Superficie de decisión distinta, con contrato propio (`QuotaEligibilityPolicy`). La regla 9 del diseño la excluye explícitamente. |

## 5. Matriz de escenarios de aceptación

| ID    | Escenario                                                                       | Criterio observable                                                                                              | Evidencia                                                                                                                                          | Estado |
|-------|---------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------|--------|
| QA-01 | La captura no altera el historial de uso local                                  | La escritura de una observación no modifica `usage_history`                                                      | `cargo test -p audit-sqlite` — `quota_observation_round_trips_values_nulls_and_runtime_identity` y los tests de `latest_observations` por serie    | PASS   |
| QA-02 | La observación no se deriva del uso local                                       | No hay derivación desde tokens, coste o frecuencia 429                                                           | `cargo test -p rook-core --test quota_tests` — `provider_scoped_observation_does_not_prove_connection_exhaustion` (10 tests)                       | PASS   |
| QA-03 | La serie más reciente gana y la evidencia vencida no prueba agotamiento         | `latest_observations` conserva la observación más reciente por serie y `stale_evidence_is_unknown_not_exhausted` | `cargo test -p audit-sqlite` (21 tests) y `cargo test -p rook-core --test quota_tests`                                                             | PASS   |
| QA-04 | Las cabeceras del proveedor se capturan con procedencia                         | OpenAI y Anthropic exponen sus cabeceras de cuota al adaptador                                                   | `cargo test -p providers-openai --test quota_headers` (1) y `-p providers-anthropic --test quota_headers` (3)                                      | PASS   |
| QA-05 | El resumen expone cobertura solo de Cortex                                      | La respuesta declara `coverage: cortex_routed` y el origen                                                       | `cargo test -p transport-axum --test provider_quota_routes` — `get_provider_quota_returns_json_with_explicit_usage_coverage_and_source` (13 tests) | PASS   |
| QA-06 | Las observaciones upstream se exponen separadas del uso, los 429 y los umbrales | Campo `upstreamObservations` con procedencia, alcance, ventana, edad y frescura                                  | `provider_quota_routes` — `get_provider_quota_exposes_upstream_observations_separately_with_provenance_and_age`                                    | PASS   |
| QA-07 | La ausencia de observaciones no se presenta como salud o ausencia de límite     | `not_reported` explícito                                                                                         | `provider_quota_routes` — `get_provider_quota_marks_absent_upstream_observations_not_reported`                                                     | PASS   |
| QA-08 | Un fallo de lectura de observaciones se marca `unavailable` sin perder lo demás | Fallo aislado de una consulta                                                                                    | `provider_quota_routes` — `get_provider_quota_marks_failed_upstream_observation_read_unavailable`                                                  | PASS   |
| QA-09 | Coste parcial o desconocido no se serializa como cero                           | `costUsd: null` con `costAvailability`                                                                           | `provider_quota_routes` — `get_provider_quota_reports_partial_and_unknown_cost_without_losing_provenance`                                          | PASS   |
| QA-10 | Ollama local no tiene cuota de cuenta                                           | `not_applicable` y sin valores fabricados                                                                        | `provider_quota_routes` — `get_provider_quota_marks_ollama_account_quota_not_applicable`                                                           | PASS   |
| QA-11 | La ventana renderiza la sección de observaciones con procedencia                | Sección dedicada con métrica, alcance, ventana, límite, restante, fuente, frescura, edad y reset                 | `vitest src/views/ProvidersQuotaView.spec.ts` — `renders upstream observations with provenance when available`                                     | PASS   |
| QA-12 | Sin observaciones capturadas se muestra un estado vacío honesto                 | Estado vacío explícito, sin valores de cuota                                                                     | `vitest` — `renders not_reported state when no upstream observations exist`                                                                        | PASS   |
| QA-13 | Con disponibilidad `unavailable` no se renderiza sección de observaciones       | La sección no aparece y no se afirma ausencia de señal                                                           | `vitest` — `does not render the upstream section when unavailable`                                                                                 | PASS   |
| QA-14 | La advertencia local sigue siendo sobre tráfico enrutado                        | Etiqueta explícita de advertencia local sobre tráfico enrutado                                                   | `vitest` — aserción de etiqueta en la celda de estado y `providers.quota.localWarningLabel` en `en.json`/`es.json`                                 | PASS   |
| QA-15 | El coste parcial se etiqueta como desconocido sin subtotal numérico             | Sin `$1.25` en la celda de coste                                                                                 | `vitest` — `labels partial cost as unknown without showing a numeric subtotal`                                                                     | PASS   |
| QA-16 | Las tendencias muestran siete días                                              | Siete puntos de tendencia con coste estimado                                                                     | `vitest` — `renders seven days of trend usage with cost when available`                                                                            | PASS   |
| QA-17 | Un refresco fallido conserva el último resultado y marca estado degradado       | Datos retenidos y estado `stale` visible                                                                         | `vitest src/composables/useProvidersQuota.spec.ts` — `keeps the last good result and exposes stale state after refresh failure`                    | PASS   |
| QA-18 | La recoveries limpia el estado degradado                                        | Tras un refresco correcto no queda marca de datos viejos                                                         | `vitest` — `clears stale state after a successful recovery`                                                                                        | PASS   |

Suite reejecutada en esta fase: `rook-core` 10 tests, `transport-axum` 13 tests, `audit-sqlite` 21 tests, cabeceras de proveedor 4 tests, dashboard 9 tests (5 de la vista y 4 del composable). Total 57 tests, todos en verde.

## 6. Hallazgos

| ID      | Severidad | Descripción                                                                                                                                                                                                                                                                                                             | Estado                                                                              |
|---------|-----------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------|
| QA-F-01 | INFO      | `just ci-semgrep` falla localmente porque `uv run --no-project` selecciona Python 3.14 y el `protobuf` fijado por `semgrep==1.126.0` falla al importar. CI no se ve afectado (instala con `pip` sobre el Python del runner). El escaneo pasa al fijar `--python 3.12`. No es un hallazgo del producto ni de la feature. | Aceptado como advertencia no bloqueante; no se modificó el recipe del repositorio   |
| QA-F-02 | INFO      | No se ejecutó una llamada real a un proveedor externo para observar cabeceras de cuota. La captura se verificó con respuestas controladas.                                                                                                                                                                              | Aceptado como advertencia no bloqueante; sin credenciales autorizadas en el entorno |
| QA-F-03 | INFO      | SonarCloud y el build push-only no se ejecutaron: requieren `SONAR_TOKEN` y servicio remoto, o trigger de push.                                                                                                                                                                                                         | Fuera del conjunto local por diseño; no se reportan como aprobados                  |

Sin hallazgos CRITICAL, P0 ni P1.

## 7. Trazabilidad con las specs

| Requisito (spec)                                                                                                                              | Escenarios QA                                          | Estado |
|-----------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------------------|--------|
| `provider-quota-capture`: separación de observaciones y uso local                                                                             | QA-01, QA-02, QA-03, QA-04                             | PASS   |
| `provider-usage-quota`: cobertura, periodos, coste desconocido, señales upstream, límite local                                                | QA-05, QA-06, QA-07, QA-08, QA-09, QA-10               | PASS   |
| `providers-ui`: dashboard real, cobertura, tendencias, umbrales distintos, señales capturadas, refresco degradado, Ollama sin cuota de cuenta | QA-11, QA-12, QA-13, QA-14, QA-15, QA-16, QA-17, QA-18 | PASS   |

## 8. Conclusión

Las tres capacidades de la feature tienen evidencia conductual reejecutada y todos los escenarios de aceptación aplicables están en PASS. No quedan escenarios acceptance-relevant en `BLOCKED` o `NOT TESTED`. El gate de archivo queda autorizado con advertencias, condicionado a que `verify-report.md` y este `qa-report.md` existan y a la sync de las delta specs a las specs principales.
