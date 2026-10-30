# Skill: rust-anti-patterns

Collection of Rust anti-patterns adapted for the Cortex monorepo.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — filtered and adapted.

## Purpose

Help developers avoid common mistakes that conflict with hexagonal architecture
or degrade code quality in the context of Rook.

## Included Rules

### 1. anti-clone-excessive

> Do not clone when borrowing works.

In Cortex, this is especially important in providers and the `RouteRequest` hot path.
The code has frequently cloned `Arc<ProviderPort>` unnecessarily.

 ```rust
 // BAD — Clone inside a loop
 for provider in providers.clone() {
     check(provider);
 }

 // GOOD — Iterate by reference
 for provider in &providers {
     check(provider);
 }
 ```

Useful for:

- `providers.read().await.iter()` in `router_impl`
- Iteration over `Vec<Arc<dyn ProviderPort>>`

### 2. anti-over-abstraction

> Do not over-abstract with excessive generics.

Our architecture already defines traits in rook-core (`ProviderPort`, `RouterPort`, etc.).
Do not add more abstraction unless needed.

 ```rust
 // BAD — Unnecessary trait explosion
 trait Readable {}
 trait Writable {}
 trait AsyncReadable {}
 trait AsyncWritable {}

 // GOOD — Concrete architecture traits
 trait ProviderPort: Send + Sync + 'static { ... }
 ```

Exception: Domain traits in rook-core ARE the correct abstraction.

### 3. anti-stringly-typed

> Do not use strings when enums or newtypes would provide type safety.

In Cortex, we already use this pattern correctly: `ProviderId`, `ModelId`, and `RequestId`.
Keep following it.

 ```rust
 // BAD
 fn route(provider: &str, model: &str) { ... }

 // GOOD — Existing newtypes
 fn route(provider: &ProviderId, model: &ModelId) { ... }
 ```

### 4. anti-unwrap-abuse

> Do not use `.unwrap()` in production code.

This is especially critical in providers and use cases. Transport code (`transport-axum`)
is where `.unwrap()` is most likely to appear.

 ```rust
 // BAD
 let response = provider.complete(req).unwrap();

 // GOOD
let response = provider.complete(req)
      .map_err(|e| CortexError::provider(e.to_string()))?;
 ```

Valid exceptions:

- Tests
- Const/static initialization (compile-time guaranteed)
- After a check that guarantees success

## Clippy Configuration

 ```toml
 [lints.clippy]
 clone_on_copy = "warn"
 clone_on_ref_ptr = "warn"
 redundant_clone = "warn"
 unwrap_used = "warn"
 ```

## Related Files

- `crates/domain/rook-core/src/ports.rs` — Domain traits
- `crates/domain/shared-kernel/src/id.rs` — Newtypes
- `crates/application/rook-usecases/src/router_impl.rs` — Hot path

## See Also

- rust-api-design — For correct API patterns
- rust-async-patterns — For async patterns
