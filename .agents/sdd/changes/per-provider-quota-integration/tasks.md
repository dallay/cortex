# Tareas: Per-Provider Quota Integration

## Previsión de carga de revisión

- Estimación (incluye SDD): U1 250–350, U2 180–260 (completas); U3 220–350; U4 180–300; U5 300–390; U6 280–380 líneas.
- Seis unidades; U1–U2 completas, U3–U6 pendientes. Riesgo: medio.
- Entrega: `feature-branch-chain`, autorizada; sin GitHub Stack metadata.

Decisión requerida antes de aplicar: No
PR encadenadas recomendadas: Sí
Estrategia de cadena: `feature-branch-chain`
Riesgo del límite de 400 líneas: Medio

**Stop/replan:** si el diff total de una unidad (incluido SDD) se estima o alcanza 400 líneas, detenerse y subdividir/replanificar; no size-exception. Commits locales firmados; no push/PR.

### Unidades de trabajo sugeridas

Rama de integración/tracker: `feature/dallay-404-per-provider-quota-integration-real-data-not-mock` → `main`. Issue/Linear: `DALLAY-404`. Cada unidad se basa en su predecesora inmediata.

| U | Alcance; estimación | Rama → base | Orden/dependencia |
|---|--------------------|-------------|-------------------|
| 1 | Persistencia; 250–350, completa | `feature/dallay-404-quota-storage` → rama de integración | 1; firmado `fb691f1` |
| 2 | Parseo de headers OpenAI/Anthropic/Groq; 180–260, completa | `feature/dallay-404-quota-capture-api` → `feature/dallay-404-quota-storage` | 2; U1; firmado `dcdfd92` |
| 3 | Captura normal/errores HTTP, registro en mejor esfuerzo, `ProviderId` runtime; 220–350 | `feature/dallay-404-quota-capture-wiring` → `feature/dallay-404-quota-capture-api` | 3; U2; rama actual limpia |
| 4 | Captura streaming antes de consumir el body y aislamiento; 180–300 | `feature/dallay-404-quota-capture-streaming` → `feature/dallay-404-quota-capture-wiring` | 4; U3 |
| 5 | Endpoint/DTO, vigencia y alertas por conexión; 300–390 | `feature/dallay-404-quota-api` → `feature/dallay-404-quota-capture-streaming` | 5; U4 |
| 6 | Dashboard/pruebas y reconciliación de especificación UI; 280–380 | `feature/dallay-404-quota-dashboard-spec` → `feature/dallay-404-quota-api` | 6; U5 |

No añadir plumbing redundante de `ConnectionId`. Mantener cada diff en su rama y ≤400 líneas.

## Fase 1: Persistencia — U1 (completa)

- [x] 1.1 Añadir modelo/puerto recorder, V6, persistencia/retención SQLite y wiring DI/sweep en archivos aprobados. Sin backfill ni cambios en `usage_history`.

## Fase 2: Parseo de headers — U2 (completa)

- [x] 2.1 Parsear headers documentados OpenAI, Anthropic Messages y Groq; probar valores válidos, ausentes, malformados y reset.

## Fase 3: Captura HTTP — U3

- [x] 3.1 Capturar respuestas normales/errores HTTP solo con endpoints canónicos por defecto; conectar registro en mejor esfuerzo sin alterar resultados y probar identidad con el `ProviderId` runtime existente.

## Fase 4: Streaming — U4

- [ ] 4.1 Capturar headers streaming antes de consumir el body; permitir URLs canónicas OpenAI/Anthropic/Groq (incluidas URLs predeterminadas explícitas del UI); probar conservación del stream y aislamiento de la captura/fallos del recorder.

## Fase 5: API — U5

- [ ] 5.1 Mapear uso local y observaciones upstream/edad/expiración, y conteos 429 en `crates/infrastructure/transport-axum/src/{handlers/provider_quota.rs,provider_dto.rs}`; probar no-disponible≠cero, vigencia, umbrales y ausencia de alertas 429.

## Fase 6: Dashboard/spec — U6

- [ ] 6.1 Actualizar `apps/rook/dashboard/src/{lib/api.ts,composables/useProvidersQuota.ts,views/ProvidersQuotaView.vue,locales/{en,es}.json}`; separar local/upstream; probar datos obsoletos, mixtos y ausentes.
- [ ] 6.2 Reconciliar `.agents/sdd/specs/providers-ui/spec.md`; retirar placeholder/follow-up obsoleto sin cambiar semánticas.

## Fase 7: Verificación/entrega

- [ ] 7.1 Ejecutar proveedores (`cargo test -p providers-openai -p providers-anthropic -p providers-groq`), app/handlers (`cargo test -p rook -p transport-axum`) y dashboard `cd apps/rook/dashboard && pnpm exec vitest run && pnpm exec vue-tsc --noEmit`.
- [ ] 7.2 Ejecutar `just ci-local`; sin Docker E2E, ejecutar `just ci-ci` e informar que no se ejecutó (`just test-e2e` requerido para verificación completa).
- [ ] 7.3 En cada unidad entregada, crear un commit local firmado; no hacer push ni crear PR.

**Semánticas/exclusiones:** uso local ≠ observaciones upstream; conservar reset/vigencia y 429 separado, sin usar 429 para calcular utilización ni emitir alertas de cuota. Sin polling, Anthropic Admin API ni señales especulativas de Gemini/Ollama; sin agregaciones incompatibles.
