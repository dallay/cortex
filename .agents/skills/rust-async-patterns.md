# Skill: rust-async-patterns

Patrones de código async para Tokio, adaptados para el monorepo Cortex.

> Based on [rust-skills](https://github.com/leonardomso/rust-skills) — filtrada y adaptada.

## Propósito

Estandarizar patrones async en todo el codebase, especialmente críticos
para evitar deadlocks y garantizar graceful shutdown.

## Reglas Incluidas

### 1. async-no-lock-await ⚠️ CRÍTICO

> Nunca holder Mutex/RwLock across `.await`.

Este es EL problema crítico encontrado durante el análisis. El codebase
ACTUALMENTE tiene errores de este tipo en router_impl.rs.

 ```rust
 // BAD — Lock across await causa deadlocks
 async fn bad(state: &Mutex<State>) {
     let mut guard = state.lock().await;
     let data = fetch_from_network().await; // Lockheld!
     guard.value = data;
 }

 // GOOD — Lock solo para update rápido
 async fn good(state: &Mutex<State>) {
     let data = fetch_from_network().await; // Sin lock
     let mut guard = state.lock().await;
     guard.value = data; // Quick update
 }
 ```

**¿Por qué importa?** Tokio usa pocos threads para muchas tasks. Si una task
hold un lock durante I/O, todas las demás esperan — potencialmente indefinidamente.

### 2. async-clone-before-await

> Clone Arc/Rc data antes de await points.

Necesario para que los futures sean Send.

 ```rust
 // BAD — Borrow across await, future not Send
 async fn process(data: Arc<Data>) {
     let slice = &data.items[..]; // Borrow
     expensive_async_op().await;    // Await with active borrow
     use_slice(slice);
 }

 // GOOD — Clone owned data before await
 async fn process(data: Arc<Data>) {
     let items = data.items.clone(); // Owned
     expensive_async_op().await;
     use_items(&items);
 }
 ```

En Cortex: importante para spawn_blocking y para pasar datos a tareas.

### 3. async-join-parallel

> Usa `join!` o `try_join!` para futures independientes concurrentes.

 ```rust
 // BAD — Sequential, 300ms total
 async fn fetch_data() -> (User, Posts, Comments) {
     let user = fetch_user().await;      // 100ms
     let posts = fetch_posts().await;    // 100ms
     let comments = fetch_comments().await; // 100ms
     (user, posts, comments)
 }

 // GOOD — Concurrent, ~100ms
 async fn fetch_data() -> (User, Posts, Comments) {
     let (user, posts, comments) = tokio::join!(
         fetch_user(),
         fetch_posts(),
         fetch_comments(),
     );
     (user, posts, comments)
 }
 ```

**Aplicar a:** Fallback multiple providers, health checks paralelos.

### 4. async-spawn-blocking

> Usa `spawn_blocking` para trabajo CPU-intensive.

 ```rust
 // BAD — CPU work en async thread
 async fn hash_password(password: String) -> String {
     bcrypt::hash(&password, bcrypt::DEFAULT_COST).unwrap() // Blocks runtime!
 }

 // GOOD — Offload a blocking thread pool
 async fn hash_password(password: String) -> String {
     tokio::task::spawn_blocking(move || {
         bcrypt::hash(&password, bcrypt::DEFAULT_COST).unwrap()
     })
     .await
     .unwrap()
 }
 ```

Guideline:

- < 10µs: OK en async thread
- 10µs - 1ms: Consider spawn_blocking
- > 1ms: Definitivamente spawn_blocking

### 5. async-cancellation-token

> Usa CancellationToken para graceful shutdown.

 ```rust
 use tokio_util::sync::CancellationToken;

 async fn run_server(shutdown: CancellationToken) {
     loop {
         tokio::select! {
             _ = shutdown.cancelled() => {
                 println!("Shutting down gracefully");
                 cleanup().await;
                 break;
             }
             result = listener.accept() => {
                 let (socket, _) = result?;
                 let conn_token = shutdown.child_token();
                 tokio::spawn(handle_connection(socket, conn_token));
             }
         }
     }
 }
 ```

**Aplicar a:** Server shutdown en apps/rook, graceful request cancellation.

### 6. async-bounded-channel

> Usa bounded channels para aplicar backpressure.

 ```rust
 // BAD — Unbounded puede causar OOM
 let (tx, mut rx) = mpsc::unbounded_channel();

 // GOOD — Backpressure natural
 let (tx, mut rx) = mpsc::channel(100); // Cap at 100
 ```

Guideline para buffer size:

- Start con expected burst size
- Error on smaller side initially
- Measure en producción

## Errores Comunes en Cortex

 ```rust
 // ❌ ERROR ENCONTRADO en router_impl.rs
 let providers = self.providers.read().unwrap();
 // parking_lot::RwLock no tiene .read().await()
 // Debe usar tokio::sync::RwLock O no usar await con parking_lot

 // ✅ Fix correcto (opción 1: usar tokio::sync::RwLock)
 let providers = self.providers.read().await;

 // ✅ Fix alternativo (opción 2: no hacer async)
 let providers = self.providers.read(); // Sync read, no await
 ```

## Archivos Relacionados

- `crates/application/rook-usecases/src/router_impl.rs` —⚠️Tiene errores
- `crates/infrastructure/providers-*/src/` — Providers async
- `apps/rook/src/server.rs` — Server shutdown

## See Also

- rust-anti-patterns — Anti-patterns a evitar
- rust-api-design — Patrones de API
