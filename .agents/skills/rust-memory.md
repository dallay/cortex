# Skill: rust-memory

Patrones de gestión de memoria para Rust, adaptados para el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — memory rules.

## Propósito

Guía para optimizar uso de memoria en el codebase. Especialmente relevante
para providers que procesan respuestas grandes y el cache en memory.

## Reglas Incluidas

### 1. mem-with-capacity

> Usa `with_capacity()` cuando conoces el tamaño.

 ```rust
 // ✅ Correcto - pre-allocation
 let mut results = Vec::with_capacity(providers.len());
 for provider in providers {
     results.push(process(provider));
 }

 // ❌ Incorrecto - reallocations
 let mut results = Vec::new();
 for provider in providers {
     results.push(process(provider));  // Puede reallocate múltiples veces
 }
 ```

### 2. mem-avoid-format

> Evita `format!()` en hot paths.

 ```rust
 // ✅ Correcto - string literal
 fn get_error_message() -> &'static str {
     "An error occurred"
 }

 // ❌ Incorrecto - allocation en cada llamada
 fn get_error_message() -> String {
     format!("An error occurred")  // Allocation innecesaria
 }
 ```

Para logging en loops:

 ```rust
 // ✅ Correcto - format args directo
 for event in events {
     log::info!("Processing item: {}", event.name);
 }

 // ❌ Incorrecto
 for event in events {
     let msg = format!("Processing item: {}", event.name);
     log::info!("{}", msg);  // Double work
 }
 ```

### 3. mem-clone-from

> Usa `clone_from()` para reusar allocaciones en clones repetidos.

 ```rust
 let mut buffer = String::with_capacity(1024);
 for source in sources {
     buffer.clone_from(source);  // Reusa la capacity
     process(&buffer);
 }
 ```

### 4. mem-compact-string

> Considera `CompactString` para muchos strings pequeños.

Para IDs de providers y modelos que son cortos:

 ```rust
 // En lugar de String para Short IDs
 struct ModelId(CompactString);

 impl ModelId {
     pub fn new(id: &str) -> Option<Self> {
         if id.len() <= 24 {
             Some(ModelId(id.into()))
         } else {
             None
         }
     }
 }
 ```

**Aplicar solo si profiling muestra que es necesario.**

### 5. mem-zero-copy

> Usa referencias en lugar de clones cuando sea posible.

 ```rust
 // ✅ Zero-copy - referencias
 fn process_items(items: &[Item]) {
     for item in items {
         println!("{}", item.name);
     }
 }

 // ❌ Allocation innecesaria
 fn process_items(items: &Vec<Item>) {
     for item in items.clone() {  // Clona todo el Vec
         println!("{}", item.name);
     }
 }
 ```

### 6. mem-boxed-slice

> Usa `Box<[T]>` en lugar de `Vec<T>` para datos de tamaño fijo.

 ```rust
 // ✅ Mejor - solo pointer + length (16 bytes)
 struct Document {
     pages: Box<[Page]>,
 }

 // ❌ Mayor overhead (24 bytes: pointer + length + capacity)
 struct Document {
     pages: Vec<Page>,
 }
 ```

### 7. mem-smallvec

> Usa `SmallVec` para colecciones usualmente pequeñas.

Para headers HTTP o query params que típicamente son pocos:

 ```rust
 use smallvec::{smallvec, SmallVec};

 struct Request {
     headers: SmallVec<[(String, String); 4]>,  // 4 items inline
 }
 ```

### 8. mem-reuse-collections

> Clear y reusa collections en loops en lugar de crear nuevas.

 ```rust
 // ✅ Correcto
 let mut temp = Vec::new();
 for batch in batches {
     temp.clear();  // Reusa la allocation
     for item in &batch.items {
         temp.push(process(item));
     }
     results.push(finalize(&temp));
 }

 // ❌ Incorrecto - allocation por iteration
 for batch in batches {
     let mut temp = Vec::new();  // Nueva allocation cada vez
     // ...
 }
 ```

## Anti-Patterns de Memoria

### Evitar clone excesivo

Ya cubierto en `rust-anti-patterns.md`:

- No clones cuando borrow funciona
- Usa `Arc` para shared ownership
- Clone only en async move

### Evitar String para Strings Peques

 ```rust
 // ❌ String para short identifiers
 struct ModelId(String);  // 24 bytes + heap

 // ✅ CompactString para strings cortos
 struct ModelId(CompactString);  // 24 bytes, inline
 ```

## Aplicación en Cortex

### Cache de Respuestas

El cache en memoria usa `DashMap` con TTL. Consideraciones:

- Keys: `CacheKey` derivado de request hash (short)
- Values: `CompletionResponse` puede ser grande
- No hacer clone innecesario de responses completas

### Providers

Providers procesan respuestas HTTP:

- Usar `reqwest` con streaming para respuestas grandes
- Considerar `bytes::Bytes` para evitar copias
- No parsear JSON completo si solo necesitas extract fields

### Routes/Axum

 ```rust
 // ✅ Streaming response
 async fn stream_completion(
     body: axum::body::Body,
 ) -> impl Response {
     let bytes = axum::body::to_bytes(body, 10_000_000).await?;
     // Process sin allocation excesiva
 }

 // ❌ Buffer entire body
 async fn stream_completion(
     body: Body,
 ) -> Result<String> {
     let body = body.collect().await?.to_bytes();
     let text = String::from_utf8(body.to_vec())?;  // Multiple allocations
 }
 ```

## Medición

Antes de optimizar memoria, mide:

 ```bash
 # Memory profiling
 valgrind --tool=massif target/release/rook
 cargo flamegraph

 # O con instruments en macOS
 Instruments > Allocations
 ```

## Archivos Relacionados

- rust-anti-patterns — Anti-patterns de clones
- rust-async-patterns — Async memory patterns
- rust-linting — Clippy memory lints

## See Also

- [Rust Memory Management](https://doc.rust-lang.org/nomicon/mem.html)
- [bytes crate](https://docs.rs/bytes) — Para zero-copy byte handling
