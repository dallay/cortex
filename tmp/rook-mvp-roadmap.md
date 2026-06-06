# Rook MVP Roadmap — OmniRoute Replacement

Generado: 2026-06-03  
Actualizado: 2026-06-06  
Última revisión: 2026-06-06 (AUDITORÍA COMPLETA — múltiples features ya implementadas)

## Definición MVP
Proxy AI funcional con:
- Rate limiting por cliente
- Usage tracking + cost estimation
- Health checks + circuit breaker
- Multi-step fallback chains (combos)
- Response caching
- Request deduplication
- Model aliasing
- Basic telemetry

## Roadmap de Issues

### Wave 1 — Paralelo (sin dependencias entre sí)
| # | Issue | Estado | Notas |
|---|-------|--------|-------|
| 43 | Per-Client Rate Limiting | ✅ DONE | Merged in PR #95 |
| 41 | Usage Tracking + Cost | ✅ DONE | Merged in PR #102 |
| 42 | Health Check + Circuit Breaker | ✅ DONE | PR #103 (merged) + resilience endpoints |
| 50 | Read Cache | ✅ DONE | PRs #106 + #110 + #121 (dual-layer cache) |
| 47 | Model Aliasing | ✅ DONE | PRs #109 + #111 (merged) |
| **49** | **Request Deduplication** | **✅ DONE** | **PRs #119 + #123 (dual-layer signature cache)** |
| **65** | **Ollama Provider** | **✅ DONE** | **PR #128 + #115 dashboard UI** |
| 44 | OAuth Token Refresh | ⚠️ PARCIAL | Domain model existe, falta auto-refresh flow |
| 66 | Kiro AI Provider | ❌ PENDIENTE | BLOQUEANTE para paridad OmniRoute |

### Wave 2 — Después de #41 ✅ (COMPLETADO ✅)
| # | Issue | Depende de | Estado | Notas |
|---|-------|------------|--------|-------|
| **48** | **Request Telemetry** | **#41 ✅** | **✅ DONE** | **PRs #113 + #114 (p50/p95/p99 + API)** |

### Wave 3 — Después de #41 + #42 ✅ (COMPLETADO ✅)
| # | Issue | Depende de | Estado | Notas |
|---|-------|------------|--------|-------|
| 39 | Multi-step Fallback Chains | #41 ✅ + #42 ✅ | ✅ DONE | Completado 2026-06-04, archivado en openspec |

---

## Resumen de Waves

```
Wave 1:  9 issues en paralelo  →  8/9 completados ✅ (#43, #41, #42, #50, #47, #49, #65)
                                   1 bloqueante: #66 Kiro ❌
                                   1 parcial: #44 OAuth ⚠️
Wave 2:  1 issue               →  1/1 completado ✅ (#48)
Wave 3:  1 issue               →  1/1 completado ✅ (#39)

Total MVP: 11 issues
```

## Progreso Actual

```
[████████████████████████████████████████████░░] 91% (10/11 issues MVP completados)

Wave 1:  8/9 completados ✅ (#43, #41, #42, #50, #47, #49, #65)
         1/9 parcial ⚠️ (#44 OAuth — domain model listo)
         1/9 pendiente ❌ (#66 Kiro)
Wave 2:  1/1 completado ✅ (#48)
Wave 3:  1/1 completado ✅ (#39)

Próximo paso CRÍTICO:
  ❌ #66 Kiro AI Provider — BLOQUEANTE para paridad con OmniRoute
     36 conexiones en DB backup (segundo provider más usado)
     Necesario para combos "writer" y otros

Post-MVP (providers adicionales de OmniRoute):
  - codex (62 conexiones) — el más usado
  - antigravity (7) — usado en combo "writer"
  - kimi-coding (7), openrouter (4), otros
```

## Notas de Auditoría 2026-06-06

### ✅ Features Completadas (Verificadas en Código)
- **#43 Rate Limiting**: PR #95, código funcional
- **#41 Usage Tracking**: PR #102, endpoints `/api/usage/*`
- **#42 Health Check**: PR #103, circuit breaker + endpoints `/api/resilience`
- **#50 Read Cache**: PRs #106 + #110 + #121, dual-layer cache con signatures
- **#47 Model Aliasing**: PRs #109 + #111, domain model + HTTP API
- **#39 Combos**: PR #104, multi-step fallback chains
- **#49 Deduplication**: PRs #119 + #123, signature cache + endpoints `/api/cache/*`
- **#48 Telemetry**: PRs #113 + #114, p50/p95/p99 + endpoints `/api/telemetry/*`
- **#65 Ollama**: PR #128 (provider core) + #115 (dashboard UI)

### ⚠️ Parcialmente Completado
- **#44 OAuth**: Domain model implementado (`Credentials::OAuth`, `refresh_token` encrypted)
  - ✅ Existe: Almacenamiento de tokens, validación de expiración
  - ❌ Falta: Auto-refresh flow, device code flow, retry en 401

### ❌ Bloqueante Real
- **#66 Kiro Provider**: No existe en el código
  - 36 conexiones en OmniRoute (segundo más usado después de codex)
  - Necesario para replicar combos `writer` y otros de la DB backup

### 📊 Tests y CI
- ✅ `cargo check`: compila sin errores
- ✅ `cargo test --lib`: 116 tests pasan
- ✅ Últimos 8 PRs mergeados sin CI failures
- ✅ SonarQube Quality Gate: PASSING (PR #122)

### 🐛 Bug Fixes (2026-06-06)

#### Provider Test Flow Fixes
- **CONFLICT error on multiple tests**: Fixed by adding `POST /api/providers/test-credentials`
  endpoint that validates credentials WITHOUT persisting to database
- **Test creates temporary provider**: Replaced with ephemeral test that doesn't touch DB
- **Save allowed without test**: Save button now disabled until `testResult.ok === true`
- **baseUrl editable for Ollama Cloud**: Now readonly (always `https://api.ollama.com`)
- **isActive defaults**: Checkbox defaults to checked (already in form, verified)

These fixes address:
- Issue #49 (Request Deduplication) was marked done but had CONFLICT bug
- Dashboard UX issue: users could save without testing

### 🔍 Providers Faltantes (Post-MVP)
Según `tmp/rook-backup-2026-05-28T21-36-43-602Z.sqlite`:
- codex (62 conexiones) — el más crítico
- antigravity (7) — usado en combo "writer"
- kimi-coding (7)
- gemini-cli (7) — posible alias de gemini
- cerebras (4), jules (4), openrouter (4)
- deepseek (2), nvidia (2)
- github (1), minimax (1), tavily-search (1)