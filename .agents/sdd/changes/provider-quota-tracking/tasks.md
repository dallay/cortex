# Tasks: Provider Quota Tracking

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | 450–650 |
| 400-line budget risk | High |
| Chained PRs recommended | No (entrega solicitada en una PR ordinaria) |
| Suggested split | Una PR: coste → contrato backend → dashboard → verificación |
| Delivery strategy | single-pr |
| Chain strategy | single-pr |

Decision needed before apply: No
Chained PRs recommended: No
Chain strategy: single-pr
400-line budget risk: High

El usuario autorizó consolidar todo el alcance en una sola PR aunque el stack complique la entrega; esto aprueba la excepción de tamaño para esta entrega (estimado 450–650 líneas). Justificación: evitar complejidad de coordinación en tres capas y entregar el flujo integrado de extremo a extremo. Base `main`, issue/Linear #132. Se conserva el alcance técnico completo; no se crean ramas/capas superiores ni metadata de GitHub Stack. El uso Cortex no representa el total de cuenta; no inventar límites.

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1 | Costes completos, resumen backend veraz y dashboard localizado con tendencias y refresco. | PR única | Base=`main`; issue/Linear=#132. Inicio: agregado `cost_usd`; fin: criterios técnicos y verificaciones de las fases completados. |

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
- [x] 2.3 En `provider_quota.rs`, construir filas de los seis tipos de proveedor desde conexiones; exponer cobertura solo Cortex, configuración, umbrales y disponibilidad correcta de Ollama local/Cloud.
- [x] 2.4 Mapear datos all-time/24 h/7 días y siete buckets UTC a estados explícitos; aislar fallos del recorder por medida/ventana y mantener coste desconocido como null.
- [x] 2.5 Añadir tests de ruta en `crates/infrastructure/transport-axum/tests/` con recorder falso configurable y `ManageConnections`: cobertura de respuesta JSON/procedencia, recorder ausente, fallo de una consulta de resumen conservando ventanas exitosas, coste parcial representado como `null` y agregación por proveedor/conexiones. Verificado con `cargo test -p transport-axum --test provider_quota_routes` (4 tests).

## Phase 3: Dashboard Usage and Refresh

- [x] 3.1 Reflejar enums, costes nullable y contrato en `apps/rook/dashboard/src/lib/api.ts`; conservar URL de `getProvidersQuota()`.
- [x] 3.2 Ampliar `useProvidersQuota.ts` y `useProvidersQuota.spec.ts`: refresco manual/60 s, solapamientos, último dato válido, stale/recuperación, fallo inicial y cleanup al desmontar.
- [x] 3.3 Actualizar `ProvidersQuotaView.vue` y su spec: cobertura/procedencia, no disponible vs sin datos, alertas locales, cuotas de cuenta, costes/proveedores desconocidos y refresco stale.
- [x] 3.4 Mostrar tendencias de 7 días (requests/tokens/coste estimado conocido) con primitives existentes; añadir equivalente accesible y casos vacío/no disponible.
- [x] 3.5 Actualizar `apps/rook/dashboard/src/locales/en.json`, `locales/es.json` y `src/__tests__/providersI18n.spec.ts`; validar Vitest, typecheck y build.

## Phase 4: Integrated Verification

- [ ] 4.1 Contrastar ambas delta specs con suites focalizadas; `just ci-local` se ejecutó y falló en dashboard build por dependencia base `@unovis/vue` no declarada, importada por chart existente fuera del diff. No declarar CI completo. **Parcial:** suites focalizadas y typecheck pasan; lint/fmt de `just ci-local` pasan.
- [x] 4.2 Confirmado por `git diff --name-only` y lectura de `routes.rs`: no cambian `V2__usage_history.sql`, adapters, routing, DI, autorización admin ni URL `/api/providers/quota`. Cambian deliberadamente el handler/DTO y dashboard para cumplir el contrato; el test de ruta invoca handler por Router de prueba y no revalida middleware admin existente.

## Orden de implementación

Consolidar todas las fases en la misma PR ordinaria, en orden de dependencia: agregados SQLite, contrato/endpoint backend, dashboard/localización y verificación integrada. No omitir criterios técnicos al consolidar. Resolver antes de aplicar la aprobación explícita de excepción de tamaño requerida para una PR única con riesgo alto.
