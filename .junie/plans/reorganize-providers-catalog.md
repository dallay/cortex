---
sessionId: session-261114-224511-5sxx
---

# Requirements

### Overview & Goals

Reorganizar el catálogo de Providers de Rook para alinearlo con la estructura funcional de OmniRoute: separar integraciones por tipo de conexión (compatible, OAuth, IDE, API Key, free, local, otros), colocar OpenAI Compatible y Anthropic Compatible primero, y distinguir OpenAI API de OpenAI Codex como integraciones independientes.

### User Stories

- Como usuario, quiero ver OpenAI Compatible y Anthropic Compatible en la primera sección para configurar endpoints personalizados rápidamente.
- Como usuario, quiero que buscar "OpenAI" muestre las integraciones relevantes (OpenAI API, OpenAI Compatible, OpenAI Codex) en sus secciones respectivas.
- Como usuario, quiero configurar OpenAI API sin que afecte mis conexiones OpenAI Codex.
- Como administrador, quiero que cada conexión se pueda probar, habilitar, deshabilitar y administrar independientemente.

### Functional Requirements

1. **7 categorías de proveedores**: Compatible, OAuth, IDE, API Key, Free/No Auth, Local, Otros.
2. **Orden de categorías**: Compatible primero, luego OAuth, IDE, API Key, Free, Local, Otros.
3. **OpenAI Codex** como descriptor separado con OAuth placeholder (sin implementación completa aún).
4. **Anthropic Compatible** como descriptor separado en la sección Compatible.
5. **Sincronización backend-frontend**: las categorías se definen en el backend (`ProviderDescriptor`) y el frontend las consume.
6. **Todos los 16 proveedores existentes** del backend deben aparecer en el frontend con su categoría correcta.
7. **Filtros y búsqueda** actualizados para las 7 categorías.

### Out of Scope

- Implementación completa de OAuth para OpenAI Codex (placeholder solo).
- Gestión de múltiples conexiones por integración con estado independiente (P1).
- Importación y onboarding (P2).
- Cambios al motor de routing de Rook.

# Delivery Steps

### ✓ Step 1: Add category field to backend ProviderDescriptor
Agregar campo `category` a `ProviderDescriptor` en Rust y asignar las 7 categorías a los 16 proveedores existentes.

- Agregar enum `ProviderCategory` en `crates/domain/rook-core/src/provider_descriptor.rs` con valores: `Compatible`, `OAuth`, `IDE`, `ApiKey`, `Free`, `Local`, `Other`.
- Agregar campo `category: ProviderCategory` a `ProviderDescriptor`.
- Asignar categorías a los 16 proveedores existentes: openai→ApiKey, anthropic→ApiKey, gemini→ApiKey, azure-openai→OAuth, groq→ApiKey, ollama→Local, ollama-cloud→ApiKey, xai→ApiKey, mistral→ApiKey, deepseek→ApiKey, together→Other, fireworks→Other, cerebras→Other, openrouter→Other, nan→Other, openai-compatible→Compatible.
- Agregar `category` al DTO `ProviderDescriptorResponse` en `provider_routes.rs`.
- Actualizar tests en `crates/domain/rook-core/tests/descriptor_table.rs`.

### ✓ Step 2: Add openai-codex and anthropic-compatible descriptors
Agregar `openai-codex` y `anthropic-compatible` como nuevos `ProviderDescriptor` con OAuth placeholder.

- Agregar `openai-codex()` en `provider_descriptor.rs`: `auth: OAuth`, `oauth: None` (placeholder), `protocol: OpenAiChat`, `requires_base_url: false`, `category: OAuth`, `accepted_auth_kinds: &[AuthKind::OAuth]`.
- Agregar `anthropic-compatible()` en `provider_descriptor.rs`: `auth: ApiKey`, `protocol: AnthropicMessages`, `requires_base_url: true`, `category: Compatible`, `adapter: OpenAiCompatible` (o nuevo `AnthropicCompatible` si se prefiere).
- Agregar `ProviderAdapter::OpenAiCodex` y `ProviderAdapter::AnthropicCompatible` si se requiere, o reusar adaptadores existentes.
- Actualizar `PROVIDER_DESCRIPTORS` array con los nuevos entries.
- Actualizar tests en `descriptor_table.rs` y `provider_kind.rs`.

### ✓ Step 3: Sync frontend catalog with backend categories
Sincronizar `providerCatalog.ts` para consumir categorías desde la API y eliminar el catálogo estático duplicado.

- Actualizar `ProviderDescriptorResponse` en `lib/api.ts` para incluir `category`.
- Modificar `useProviderDescriptors.ts` para pasar el `category` del backend al catálogo.
- Actualizar `CategoryKind` en `providerCatalog.ts` a: `compatible`, `oauth`, `ide`, `api-key`, `local`, `free`, `other`.
- Reemplazar `CATEGORIES` estático por una derivación desde los descriptors o mantenerlo como metadata de presentación (i18n keys) pero con las 7 categorías.
- Eliminar la dependencia de `PROVIDER_KINDS` estático para la lista de proveedores; usar solo los descriptors del backend.
- Mantener `PROVIDER_KINDS` como fallback para metadata de presentación (iconFile, brandUrl, defaultModels) que no viene del backend.

### ✓ Step 4: Update dashboard UI for 7 categories
Actualizar `ProvidersView.vue`, `ProviderCategorySection.vue`, `ProviderCatalogFilter.vue` y `ProviderCatalogCard.vue` para las 7 categorías.

- Actualizar `ProviderCatalogFilter.vue` para mostrar chips de las 7 categorías (compatible, oauth, ide, api-key, local, free, other).
- Actualizar `ProvidersView.vue` para ordenar las secciones por el orden de categoría (compatible primero, luego oauth, ide, api-key, local, free, other).
- Actualizar `ProviderCategorySection.vue` para manejar las nuevas categorías.
- Actualizar `ProviderCatalogCard.vue` para mostrar badges de categoría o autenticación si es necesario.
- Agregar i18n keys en `locales/en.json` y `locales/es.json` para las 7 categorías (nombres y descripciones).
- Agregar i18n keys para los nuevos proveedores (openai-codex, anthropic-compatible).
- Actualizar `AddProviderDialog.vue` para que los nuevos descriptors aparezcan en el selector de kind.

### ✓ Step 5: Run tests and update fixtures
Ejecutar `cargo test -p rook-core` y `pnpm --filter @dallay/rook-dashboard exec vitest run` para validar los cambios.

- Validar que `cargo test -p rook-core` pase con los nuevos descriptors y categorías.
- Validar que los tests del dashboard pasen, incluyendo `ProvidersView.spec.ts`, `useProviderDescriptors.spec.ts`, `useProviderCatalog.ts`, y `AddProviderDialog.spec.ts`.
- Actualizar mocks y fixtures en los tests para incluir los nuevos providers y categorías.
- Validar que `cargo clippy` no tenga warnings nuevos.
- Validar que el dashboard compile sin errores TypeScript.