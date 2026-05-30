# Skill: rust-api-design

Patrones de diseño de API para Rust, adaptados para el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — filtrada y adaptada.

## Propósito

Estandarizar cómo diseñamos APIs públicas en los crates de Cortex,
respetando la arquitectura hexagonal y los patrones establecidos.

## Reglas Incluidas

### 1. api-sealed-trait

> Usa traits sellados para controlar implementación externa.

En Cortex ya lo hacemos con ProviderPort. Es la forma correcta.

 ```rust
 mod private {
     pub trait Sealed {}
 }

 pub trait ProviderPort: private::Sealed {
     async fn complete(&self, req: &CompletionRequest) -> CtxResult<CompletionResponse>;
 }

 // Solo providers internos pueden implementar
 impl private::Sealed for OpenAiProvider {}
 impl ProviderPort for OpenAiProvider { ... }
 ```

**Aplicar a:** Nuevos traits públicos que no deban ser implementables externamente.

### 2. api-newtype-safety

> Usa newtypes para prevenir mixing de valores semánticamente diferentes.

Ya lo hacemos bien en shared-kernel:

- `ProviderId`, `ModelId`, `RequestId` — todos newtypes de u64/String

 ```rust
 // GOOD — Ya establecido
 #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
 pub struct ProviderId(pub u64);

 #[derive(Debug, Clone, PartialEq, Eq)]
 pub struct ModelId(pub String);
 ```

**Aplicar a:** Nuevos tipos que puedan confundirse entre sí.

### 3. api-must-use

> Marca tipos y funciones con #[must_use] cuando ignorar resultados es bug.

Crucial para builders yResult types.

 ```rust
 #[must_use = "builders do nothing unless consumed"]
 pub struct RequestBuilder { ... }

 impl RequestBuilder {
     #[must_use = "builder methods return modified builder"]
     pub fn with_timeout(mut self, t: Duration) -> Self { ... }
 }
 ```

En Cortex: Apply a builders en transport-axum y any Result que no deba ignorarse.

### 4. api-common-traits

> Implementa traits estándar (Debug, Clone, PartialEq, etc.) para tipos públicos.

Guía de derivación:

| Trait       | Cuándo derivar                     |
 |-------------|------------------------------------|
| `Debug`     | Siempre para tipos públicos        |
| `Clone`     | Tipo puede duplicarse              |
| `Copy`      | Tipos pequeños, simples            |
| `PartialEq` | Comparación tiene sentido          |
| `Eq`        | Total equality, sin floating-point |
| `Hash`      | Para HashMap keys                  |

 ```rust
 // Mínimo para tipos de dominio
 #[derive(Debug, Clone, PartialEq)]
 pub struct CompletionRequest { ... }
 ```

### 5. api-default-impl

> Implementa Default para tipos con valores sensibles por defecto.

Útil para configuración y estados.

 ```rust
 impl Default for HealthStatus {
     fn default() -> Self {
         Self {
             available: false,
             latency_ms: None,
             last_check: None,
         }
     }
 }
 ```

### 6. api-non-exhaustive

> Usa `#[non_exhaustive]` en enums/structs públicos para forward compatibility.

 ```rust
 #[non_exhaustive]
 pub enum ProviderKind {
     OpenAi,
     Anthropic,
     Ollama,
     // Future providers pueden agregarse sin breaking change
 }

 // Consumidores DEBEN usar wildcard
 match provider.kind() {
     ProviderKind::OpenAi => { ... },
     ProviderKind::Anthropic => { ... },
     _ => { ... }, // Required
 }
 ```

**Aplicar a:** Enums públicos que puedan evolucionar (ProviderKind, etc.)

### 7. api-parse-dont-validate

> Parsea en tipos validados en los boundaries.

El patrón correcto: validar una vez al entry point, usar tipos internamente.

 ```rust
 // BAD — Validación scattering
 fn handle_request(email: &str) {
     if !is_valid_email(email) { return Err(...); }
     store_email(email); // ¿Validó alguien más?
 }

 // GOOD — Parse en boundary
 fn handle_request(raw: &str) -> Result<()> {
     let email = Email::parse(raw)?; // Validado UNA vez
     store_email(&email); // Tipo seguro aquí
 }
 ```

En Cortex: applies a parsing de requests en transport-axum adapters.

## Archivos Relacionados

- `crates/domain/shared-kernel/src/id.rs` — Newtypes existentes
- `crates/domain/rook-core/src/ports.rs` — Traits de dominio
- `crates/infrastructure/transport-axum/src/` — Adapters

## See Also

- rust-anti-patterns — Anti-patterns a evitar
- rust-async-patterns — Patrones async
