# Skill: rust-anti-patterns

Colección de anti-patterns de Rust adaptada para el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — filtrada y adaptada.

## Propósito

Guiar a los desarrolladores a evitar errores comunes que contradicen la arquitectura
hexagonal o degradan la calidad del código en el contexto de Rook.

## Reglas Incluidas

### 1. anti-clone-excessive

> No clones cuando borrow funciona.

En Cortex esto es especialmente importante en los providers y en el hot path
de RouteRequest. El código frequently cloneaba Arc<ProviderPort> innecesariamente.

 ```rust
 // BAD — Clone en loop
 for provider in providers.clone() {
     check(provider);
 }

 // GOOD — Iterar por referencia
 for provider in &providers {
     check(provider);
 }
 ```

Útil para:

- `providers.read().await.iter()` en router_impl
- Iteración sobre `Vec<Arc<dyn ProviderPort>>`

### 2. anti-over-abstraction

> No sobre-abstract con generics excesivos.

Nuestra arquitectura ya define traits en rook-core (ProviderPort, RouterPort, etc.).
No agregar más abstracción a menos que sea necesario.

 ```rust
 // BAD — Trait explosion innecesario
 trait Readable {}
 trait Writable {}
 trait AsyncReadable {}
 trait AsyncWritable {}

 // GOOD — Traits concretos de la arquitectura
 trait ProviderPort: Send + Sync + 'static { ... }
 ```

Exception: Los traits de dominio en rook-core SON la abstracción correcta.

### 3. anti-stringly-typed

> No uses Strings donde enums/newtypes darían type safety.

En Cortex ya usamos esto bien: ProviderId, ModelId, RequestId.
Mantener este padrão.

 ```rust
 // BAD
 fn route(provider: &str, model: &str) { ... }

 // GOOD — Newtypes ya definidos
 fn route(provider: &ProviderId, model: &ModelId) { ... }
 ```

### 4. anti-unwrap-abuse

> No uses .unwrap() en código de producción.

Especialmente crítico en providers y use cases. El código de transporte
(transport-axum) es donde más вероятно encontrar unwrap.

 ```rust
 // BAD
 let response = provider.complete(req).unwrap();

 // GOOD
let response = provider.complete(req)
      .map_err(|e| NuxaError::provider(e.to_string()))?;
 ```

Excepciones válidas:

- Tests
- Const/static initialization (compile-time guaranteed)
- After a check that guarantees success

## Configuración Clippy

 ```toml
 [lints.clippy]
 clone_on_copy = "warn"
 clone_on_ref_ptr = "warn"
 redundant_clone = "warn"
 unwrap_used = "warn"
 ```

## Archivos Relacionados

- `crates/domain/rook-core/src/ports.rs` — Traits de dominio
- `crates/domain/shared-kernel/src/id.rs` — Newtypes
- `crates/application/rook-usecases/src/router_impl.rs` — Hot path

## See Also

- rust-api-design — Para patrones de API correctos
- rust-async-patterns — Para patrones async
