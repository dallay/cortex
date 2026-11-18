---
sessionId: session-261118-190203-nlku
---

# Requirements

### Overview
Remediar los 3 leads de seguridad encontrados en el audit previo (NEEDS-VAL). Cada lead tiene un nivel de riesgo diferente y requiere un tipo de remediación distinto.

### Lead 1: CSRF Token Reuse (NEEDS-VAL/CSRF-Token-Reuse-Login)
**Riesgo:** Bajo. El token CSRF se genera en login y se reutiliza en GET /login si ya existe un token válido. El patrón double-submit con SameSite=Strict ya protege contra ataques cross-origin.

**Remediación:** Rotar el token CSRF en cada request que cambia estado (POST, PUT, DELETE) en rutas de management, no solo en login.

### Lead 2: Provider SSRF (NEEDS-VAL/Provider-SSRF)
**Riesgo:** Mitigado. El código ya implementa:
- `redirect::Policy::none()` en el cliente HTTP (providers-openai)
- `EndpointPolicy` con `validate_url` y `validate_ip` que bloquea IPs privadas
- `GuardedResolver` que valida DNS y bloquea hosts inesperados
- Tests de regresión que verifican que los redirects no se siguen

**Remediación:** Agregar validación de URL en `config_to_domain` para rechazar URLs con esquemas no HTTPS y hosts loopback. Actualizar findings.json con evidencia.

### Lead 3: Shell Guard Relative Paths (NEEDS-VAL/Shell-Guard-Relative-Paths)
**Riesgo:** Mitigado. El código usa `rsplit('/')` que extrae correctamente el basename de rutas relativas como `./bash` → `bash`, que está en INDIRECT_EXECUTABLES.

**Remediación:** Agregar tests de regresión explícitos para rutas relativas (`./bash`, `../bin/sh`) y symlinks. Actualizar findings.json con evidencia.

### Scope
- **In Scope:** Cambios de código para CSRF rotation, validación URL de provider, tests de shell guard, actualización de findings.json.
- **Out of Scope:** Cambios de deployment, configuración de TLS, o modificaciones a la arquitectura de auth.

# Technical Design

### Current Implementation

**CSRF (auth.rs + csrf_guard.rs):**
- `login_handler` genera un token CSRF fresco en POST /login (línea 52)
- `get_login_handler` reutiliza el token existente si es válido (línea 246-248)
- `csrf_guard_middleware` valida cookie vs header pero no rota el token
- El token se setea como cookie con SameSite=Strict, sin HttpOnly (necesario para double-submit)

**Provider SSRF (provider_dto.rs + endpoint_resolver.rs):**
- `ConnectionConfigDto.base_url` acepta cualquier String sin validación
- `config_to_domain` pasa `base_url` directamente a `ConnectionConfig`
- `endpoint_resolver.rs` tiene `GuardedResolver` y `EndpointPolicy.validate_url/validate_ip`
- `providers-openai` usa `redirect::Policy::none()`

**Shell Guard (shell.rs + shell_policy.rs):**
- `reject_command` extrae basename con `rsplit('/').next()` y compara con blocklists
- Tests en `shell_policy.rs` cubren casos como `/bin/RM`, `GiT`, etc.
- No hay tests explícitos para `./bash`, `../bin/sh`

### Proposed Changes

#### 1. CSRF Token Rotation
- Modificar `csrf_guard_middleware` para generar un token CSRF nuevo después de cada request exitoso que cambia estado
- Setear el nuevo token en Set-Cookie y en un header `X-CSRF-Token` en la respuesta para que el cliente lo actualice
- Modificar `get_login_handler` para siempre generar un token fresco en lugar de reutilizar

**Archivos afectados:**
- `crates/infrastructure/transport-axum/src/middleware/csrf_guard.rs`: Agregar rotación en middleware
- `crates/infrastructure/transport-axum/src/handlers/auth.rs`: Simplificar `get_login_handler` para siempre generar fresco

#### 2. Provider URL Validation
- Agregar validación en `config_to_domain` para rechazar URLs con:
  - Esquemas no HTTPS (http://, file://, etc.)
  - Hosts loopback (127.0.0.1, localhost, ::1)
  - Hosts de red privada (10.x.x.x, 192.168.x.x, 172.16-31.x.x)
- Esto es una defensa en profundidad: EndpointPolicy ya valida en tiempo de ejecución

**Archivos afectados:**
- `crates/infrastructure/transport-axum/src/provider_dto.rs`: Agregar `validate_base_url` en `config_to_domain`

#### 3. Shell Guard Tests
- Agregar tests de regresión para rutas relativas y symlinks
- Verificar que `./bash`, `../bin/sh`, `./sudo` son bloqueados

**Archivos afectados:**
- `crates/huginn/guards/tests/shell_policy.rs`: Agregar tests de rutas relativas

### Key Decisions
- **CSRF rotation en middleware:** Es más seguro que rotar solo en handlers específicos porque cubre todas las rutas de management automáticamente
- **Validación URL en DTO:** Es una defensa en profundidad; EndpointPolicy ya valida en tiempo de ejecución, pero validar en la entrada previene configuraciones inválidas antes de persistirlas
- **Tests de shell guard:** No se modifica el guard (ya funciona), solo se agregan tests para documentar el comportamiento y prevenir regresiones

# Delivery Steps

### ✓ Step 1: Implement CSRF token rotation
Rotar el token CSRF en cada request exitoso que cambia estado y en GET /login.

- Modificar `csrf_guard_middleware` en `csrf_guard.rs` para generar un token nuevo después de cada request POST/PUT/DELETE exitoso en rutas /api/*
- Setear el nuevo token en Set-Cookie y en un header `X-CSRF-Token` en la respuesta
- Modificar `get_login_handler` en `auth.rs` para siempre generar un token fresco en lugar de reutilizar el existente
- Ejecutar `cargo test -p transport-axum` para validar que los tests existentes pasan

### ✓ Step 2: Add provider base_url validation
Agregar validación de URL en el DTO de provider para rechegar URLs sospechosas.

- Crear `validate_base_url` function en `provider_dto.rs` que rechace:
  - Esquemas no HTTPS (http://, file://, gopher://)
  - Hosts loopback (127.0.0.1, localhost, ::1, 0.0.0.0)
  - Hosts de red privada (10.x.x.x, 192.168.x.x, 172.16-31.x.x)
- Llamar `validate_base_url` en `config_to_domain` antes de crear ConnectionConfig
- Ejecutar `cargo test -p transport-axum` para validar que los tests existentes pasan

### ✓ Step 3: Add shell guard regression tests and update findings
Agregar tests de regresión para rutas relativas y actualizar findings.json con evidencia de mitigación.

- Agregar tests en `shell_policy.rs` para verificar que `./bash`, `../bin/sh`, `./sudo`, `./sh` son bloqueados por INDIRECT_EXECUTABLES
- Actualizar `findings.json` en ~/security-audit-skill/cortex-ai-sec/run-1/:
  - Marcar NEEDS-VAL/CSRF-Token-Reuse-Login como remediated con evidencia
  - Marcar NEEDS-VAL/Provider-SSRF como mitigated con evidencia de EndpointPolicy y validate_base_url
  - Marcar NEEDS-VAL/Shell-Guard-Relative-Paths como mitigated con evidencia de tests
- Ejecutar `cargo test -p huginn-guards` para validar los nuevos tests

---

## PR #716 Code Review Fixes

Los comentarios de code review de PR #716 identifican 6 issues adicionales que requieren corrección.

### ✓ Step 4: Fix CSRF Secure flag inconsistency
El middleware CSRF usa `ROOK_ENV == "production"` para el flag Secure, mientras que auth.rs usa `cfg!(not(debug_assertions)) || ROOK_ENV == "production"`. Unificar la lógica.

- Crear `is_secure()` helper compartido o usar la misma expresión en csrf_guard.rs
- Aplicar la misma lógica en `rotate_csrf_token_in_response` que en `build_csrf_token_cookie`
- Agregar test que verifica Secure flag en configuración release sin ROOK_ENV

**Archivos:** `csrf_guard.rs`, `auth.rs`, tests

### ✓ Step 5: Fix CSRF token generation error handling
`generate_csrf_token` en auth.rs ignora el error de RNG (`let _ = rng.fill`). El fallback de token cero en csrf_guard.rs es inseguro.

- `generate_csrf_token` en auth.rs retorna `Result<String, CsrfError>` y propaga el error
- En csrf_guard.rs, propagar el error de `generate_token` como 500 en lugar de usar un token predecible
- En auth.rs, manejar el fallo de generación sin reemplazar la cookie CSRF entrante

**Archivos:** `csrf_guard.rs`, `auth.rs`

### ✓ Step 6: Fix POST /login double CSRF cookie
El middleware CSRF rota el token después de POST /login exitoso, emitiendo un segundo csrf_token cookie que conflictúa con el del handler.

- Saltar la rotación de token en el middleware para POST /login (el handler ya emite un token fresco)
- Asegurar que LoginResponse.csrf_token, csrf_token cookie, y X-CSRF-Token header coincidan

**Archivos:** `csrf_guard.rs`

### ✓ Step 7: Fix provider base_url validation for Ollama
`validate_base_url` rechaza incondicionalmente `http://localhost:11434`, que es el default de Ollama con `endpoint_validation: EndpointValidation::None`.

- Mover la validación de base_url al dominio (EndpointPolicy) o usar el descriptor del provider para decidir la política
- Para providers con `endpoint_validation: None` (Ollama, OpenAI, Anthropic, etc.), permitir cualquier URL
- Para providers con `endpoint_validation: OpenAiEndpointPolicy` (openai-compatible), aplicar las restricciones de loopback y private hosts
- Aplicar la misma regla en oauth_routes.rs::config_to_domain

**Archivos:** `provider_dto.rs`, `oauth_routes.rs`

### ✓ Step 8: Fix URL parsing and IPv6 in validate_base_url
El parsing manual de URLs no maneja correctamente userinfo, brackets IPv6, y puertos. `is_private_host` no cubre IPv6.

- Usar `url::Url` (ya en Cargo.toml) para parsear la URL en lugar de string slicing
- Extraer el host correctamente sin brackets IPv6 ni userinfo
- En `is_private_host`, clasificar IPv6 unique-local (fc00::/7) y otras rangos privados

**Archivos:** `provider_dto.rs`

### ✓ Step 9: Update dashboard client to read X-CSRF-Token from responses
El dashboard client no actualiza `cachedCsrfToken` desde respuestas exitosas de state-changing requests.

- En `api.ts`, leer `X-CSRF-Token` header de respuestas exitosas y actualizar `cachedCsrfToken`
- Si el cliente es cross-origin, asegurar que la API expone el header (Access-Control-Expose-Headers)

**Archivos:** `apps/rook/dashboard/src/lib/api.ts`

### ✓ Step 10: Fix PR description and run tests
Corregir la descripción de la PR para describir la rotación como per-response refresh, no single-use consumption. Ejecutar todos los tests.

- Actualizar PR body
- `cargo test -p transport-axum`
- `cargo test -p huginn-guards`
- `cargo clippy -p transport-axum -p huginn-guards -- -D warnings`
- Dashboard tests si se modifican