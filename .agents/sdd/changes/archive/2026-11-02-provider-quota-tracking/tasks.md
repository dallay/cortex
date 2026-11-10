# Tasks: Provider Quota Tracking

## Review Workload Forecast

| Field                   | Value                                                       |
|-------------------------|-------------------------------------------------------------|
| Estimated changed lines | 750–950                                                     |
| 400-line budget risk    | High                                                        |
| Chained PRs recommended | No (entrega solicitada en una PR ordinaria)                 |
| Suggested split         | Una PR: coste → contrato backend → dashboard → verificación |
| Delivery strategy       | single-pr                                                   |
| Chain strategy          | single-pr                                                   |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: single-pr
400-line budget risk: High

El usuario autorizó consolidar todo el alcance en una sola PR aunque el stack complique la entrega; esto aprueba la excepción de tamaño para esta entrega (estimado 750–950 líneas). Justificación: evitar complejidad de coordinación en tres capas y entregar el flujo integrado de extremo a extremo. Base `main`, issue/Linear #132. Se conserva el alcance técnico completo; no se crean ramas/capas superiores ni metadata de GitHub Stack. El uso Cortex no representa el total de cuenta; no inventar límites.

### Suggested Work Units

| Unit | Goal                                                                                      | Likely PR | Notes                                                                                                                           |
|------|-------------------------------------------------------------------------------------------|-----------|---------------------------------------------------------------------------------------------------------------------------------|
| 1    | Costes completos, resumen backend veraz y dashboard localizado con tendencias y refresco. | PR única  | Base=`main`; issue/Linear=#132. Inicio: agregado `cost_usd`; fin: criterios técnicos y verificaciones de las fases completados. |

## Project Test Mode and Runners

- `.agents/sdd/config.yaml`: `strict_tdd: false`; test-first solo para lógica pura con seam adecuado (clasificación de umbrales).
- Rust: `cargo test --workspace --all-features`; dashboard: `cd apps/rook/dashboard && pnpm exec vitest run`. No hay quality runner SDD configurado.

## Phase 1: Cost Completeness

- [x] 1.1 Extender `CostBreakdown` en `crates/domain/rook-core/src/model.rs` con conteos conocidos/desconocidos, preservando semántica del total.
- [x] 1.2 Actualizar `SqliteUsageRepository::cost_breakdown` en `crates/infrastructure/audit-sqlite/src/lib.rs`: `SUM(cost_usd)`, `COUNT(cost_usd)` y conteo coincidente con filtros. Probar cero explícito, con/sin precio, mixto, ventana vacía y filtros conexión/fecha.
- [x] 1.3 Ajustar mocks `UsageRecorderPort` solo si es necesario; correr tests focalizados de audit-sqlite y crates dependientes.

## Phase 2: Truthful Backend Summary

- [x] 2.1 Extraer clasificador puro por conexión en `crates/infrastructure/transport-axum/src/handlers/provider_quota.rs`; test-first para límites warning/error y entradas no disponibles.
- [x] 2.2 Reemplazar DTO legacy en `crates/infrastructure/transport-axum/src/provider_dto.rs`: cobertura, disponibilidad, completitud de coste, `accountQuota`, umbrales agrupados, observación rate-limit, nivel de alerta, períodos y tendencias; preservar ruta y camelCase.
- [x] 2.3 En `provider_quota.rs`, construir filas de los siete tipos de proveedor desde conexiones; exponer cobertura solo Cortex, configuración, umbrales y disponibilidad correcta de Ollama local/Cloud.
- [x] 2.4 Mapear datos all-time/24 h/7 días y siete buckets UTC a estados explícitos; aislar fallos del recorder por medida/ventana y mantener coste desconocido como null.
- [x] 2.5 Añadir tests de ruta en `crates/infrastructure/transport-axum/tests/` con recorder falso configurable y `ManageConnections`: cobertura de respuesta JSON/procedencia, recorder ausente, fallo de una consulta de resumen conservando ventanas exitosas, coste parcial representado como `null` y agregación por proveedor/conexiones. Verificado con `cargo test -p transport-axum --test provider_quota_routes` (4 tests).

## Phase 3: Dashboard Usage and Refresh

- [x] 3.1 Reflejar enums, costes nullable y contrato en `apps/rook/dashboard/src/lib/api.ts`; conservar URL de `getProvidersQuota()`.
- [x] 3.2 Ampliar `useProvidersQuota.ts` y `useProvidersQuota.spec.ts`: refresco manual/60 s, solapamientos, último dato válido, stale/recuperación, fallo inicial y cleanup al desmontar.
- [x] 3.3 Actualizar `ProvidersQuotaView.vue` y su spec: cobertura/procedencia, no disponible vs sin datos, alertas locales, cuotas de cuenta, costes/proveedores desconocidos y refresco stale.
- [x] 3.4 Mostrar tendencias de 7 días (requests/tokens/coste estimado conocido) con primitives existentes; añadir equivalente accesible y casos vacío/no disponible.
- [x] 3.5 Actualizar `apps/rook/dashboard/src/locales/en.json`, `locales/es.json` y `src/__tests__/providersI18n.spec.ts`; validar Vitest y typecheck. Eliminar `ChartContainer` para evitar la dependencia innecesaria de `@unovis/vue`.

## Phase 4: Integrated Verification

- [x] 4.1 Contrastar ambas delta specs con suites focalizadas y ejecutar `just ci-local`. Evidencia: ambas delta specs contrastadas con `cargo test -p transport-axum --test provider_quota_routes` (13 tests), `cargo test -p rook-usecases`, `cargo test -p rook-core` (61 tests) y la suite del dashboard (44 archivos, 386 tests). `just ci-local` re-ejecutado tras liberar espacio: todos los gates pasaron (install, CI contract, doctor, issue-labeler, dashboard build, Markdown lint, fmt, Clippy, Huginn architecture + cargo check, Rust tests 457s, frontend typecheck/Biome/Vitest, Rust doc, cargo audit, Trivy, Gitleaks). Semgrep falló por incompatibilidad de toolchain local (`protobuf` de `semgrep==1.126.0` bajo Python 3.14 elegido por `uv`) y pasó al reejecutarse con `--python 3.12`: 19 reglas, 252 objetivos, 11 hallazgos con excepciones exactas, chequeo de no suprimidos `true`. Gates adicionales: `just ci-coverage-frontend` pasó, `just ci-coverage-rust` pasó con `--fail-under-lines 87`,
  `just ci-e2e` pasó con 90 tests. SonarCloud y el build push-only quedan fuera del conjunto local por diseño y no se reportan como passed.
- [x] 4.2 Confirmado por `git diff --name-only` y lectura de `routes.rs`: no cambian `V2__usage_history.sql`, adapters, routing, DI, autorización admin ni URL `/api/providers/quota`. Cambian deliberadamente el handler/DTO y dashboard para cumplir el contrato; el test de ruta invoca handler por Router de prueba y no revalida middleware admin existente.

## Phase 5: Upstream Capture (U1–U3, absorbed from `per-provider-quota-integration`)

Absorbed so a single change owns the whole capability. Contract in
`specs/provider-quota-capture/spec.md`. The endpoint and dashboard phases above
still do not read these observations; wiring them into the summary is U5.

- [x] 5.1 Persistence: `QuotaObservation` model and `QuotaObservationRecorderPort` in `rook-core`; `provider_quota_observations` store with indexes on runtime, connection, and `observed_at`.
- [x] 5.2 Migration: `V8__provider_quota_observations.sql`, versioned after the already-shipped `V6__oauth_client_credentials.sql` and `V7__combo_aliases.sql`.
- [x] 5.3 Parsers: documented OpenAI, Anthropic Messages, and Groq rate-limit headers, covering valid, absent, malformed, and reset forms.
- [x] 5.4 Adapters: capture headers on success and error responses before body consumption, best-effort, without altering provider outcomes.
- [x] 5.5 Canonical gate: enable capture for default and explicit canonical endpoints, disable for custom/proxy/non-HTTPS/port/foreign-host endpoints.
- [x] 5.6 Identity: restore managed runtime identity and originating connection through the recorder wrapper.
- [x] 5.7 Bounded persistence: queue with capacity 1024, non-blocking enqueue, drop counter, dedicated writer thread, drain-and-join on shutdown.
- [x] 5.8 Retention: bounded-batch deletion mirroring `usage_history` retention.
- [x] 5.9 Scope: carry `anthropic-workspace-id` as request-scoped provenance when present.
- [x] 5.10 Tests: field-level assertions for provider identity, metric, limit, remaining, and scope; canonical gate accepted/rejected matrices; identity wrapper round-trip through SQLite.
- [x] 5.11 Remove the inline `#[cfg(test)]` module from `apps/rook/src/di.rs`; cover it from the `quota_identity_tests` integration target instead.
- [x] 5.12 Wire observations into the quota summary endpoint (U5). Verified separately from Cortex-routed usage, local 429 counts, and local thresholds.

## Phase 6: Dashboard Upstream Reconciliation (U6)

- [x] 6.1 Decision of record: render captured upstream observations in a dedicated dashboard section; keep the local warning explicitly labeled as a local routed-traffic warning; never compare upstream values against local monitoring thresholds.
- [x] 6.2 API types for `upstreamObservations` in `apps/rook/dashboard/src/lib/api.ts` (availability, freshness, provenance, scope, window, limit/remaining, observation time, age, reset).
- [x] 6.3 View section `upstream-observations-<providerKind>` with metric, scope, window, limit, remaining, source, freshness, age and reset; `not_reported` renders an explicit empty state and `unavailable` renders no section.
- [x] 6.4 Localization in `en.json` and `es.json` under `providers.quota.upstream`, including the disclaimer that these are provider-reported observations and not Cortex activity or account-wide guarantees.
- [x] 6.5 Tests for `available`, `not_reported` and `unavailable` states in `ProvidersQuotaView.spec.ts`; verified with `pnpm --filter @dallay/rook-dashboard exec vitest run src/views/ProvidersQuotaView.spec.ts` (5 tests).

## Consolidation Note

`per-provider-quota-integration` was retired. Its scope (U1–U3) is Phase 5 of
this change; its design and exploration remain as reference under
`reference/`. U5 (endpoint/DTO wiring) is complete. U6 (dashboard reconciliation)
was resolved by the U6 decision of record: the dashboard renders captured upstream
observations in a dedicated section with provenance, scope/window, age and
freshness, and keeps the local warning explicitly labeled as a local
routed-traffic warning derived from recorded 429 ratios. Upstream observations
never drive the warning level and are never compared against the local
monitoring thresholds. The two designs no longer conflict: the reference
capture design is applied only to the observation display, while its
threshold-reuse proposal is rejected.

## Orden de implementación

Consolidar todas las fases en la misma PR ordinaria, en orden de dependencia: agregados SQLite, contrato/endpoint backend, dashboard/localización y verificación integrada. No omitir criterios técnicos al consolidar. La excepción explícita de tamaño para esta PR única está aprobada, como se indica en la autorización registrada arriba.
