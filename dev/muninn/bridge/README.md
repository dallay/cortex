# Spike experimental de puente Rust

Workspace Cargo independiente y excluido del workspace raíz, `publish=false`.
No es la arquitectura permanente ni una afirmación de paridad. Expone un corte
HTTP autenticado y persiste conversaciones/mensajes en Neo4j real. Su manifest
activa el patch local `third_party/candle-core` para la compatibilidad del spike.

El Bronze oficial `test_add_message_creates_conversation_on_first_call` pasó
después de observar su RED HTTP 501. Los otros 188 casos quedaron
deseleccionados, no aprobados. Hay inspección del grafo, recuperación tras
reiniciar solo Rust, denegaciones 401/400/409 y error 503 con Neo4j detenido.
Consulta `docs/muninn/rust-spike.md` desde la raíz.

Dominio no depende de Axum/Neo4j. Presentación y adaptador usan tipos concretos,
sin traits anticipados. Scope y credenciales vienen del host, no del payload.
Esto no constituye seguridad integral o autorización multiusuario.

Desde la raíz del repositorio, ejecuta los comandos con el manifest del spike:

```sh
cargo test --manifest-path dev/muninn/bridge/Cargo.toml --lib --locked
cargo fmt --manifest-path dev/muninn/bridge/Cargo.toml --check
cargo clippy --manifest-path dev/muninn/bridge/Cargo.toml --all-targets --locked -- -D warnings
```

`neo4rs` 0.8.0 se fija como dependencia experimental MIT. La auditoría actual no
reporta vulnerabilidades conocidas, pero añade advertencias de mantenimiento
en su árbol (`backoff`, `instant`, `paste`, `rustls-pemfile`). No se ocultan ni
se acepta esa selección automáticamente para la distribución final.

El test de revocación de guard está ignorado en host porque necesita la fixture
Docker. Se ejecutó aparte Red→Green con barreras deterministas. El hook solo
existe bajo `cfg(test)`; no hay endpoint debug en producción.

Límites del corte: body de 1 MiB, identificadores de sesión de 512 bytes y hasta
1000 mensajes recuperados. No son un contrato permanente de paridad. Las rutas
no implementadas devuelven 501. No hay extracción, embeddings ni integración
Huginn. El perfil `messages` aprobó 28 Bronze distintos. `sessions` los amplió a
50 con listado, limpieza de sesión y eliminación por UUID con reparación de
cadena. Las tres rutas aplican propiedad/scope y devuelven 503 si Neo4j no está
disponible. Hay regresiones de append tras borrar y protección de scope ajeno.
El perfil `chains` amplió la evidencia a 60 Bronze, incluyendo cadenas e
idempotencia. Se recorrió directamente una cadena Neo4j de 100 mensajes.
No se demostró concurrencia ni cascada completa de entidades/hechos.
Siguiente corte: restantes Bronze y GM-01.
G1 exige 93/93 y GM-01.

## Integridad de buffers MiniLM: T1.1

`artifacts::load_verified` comprueba tamaño y SHA-256 antes de devolver un
buffer privado. Lee como máximo tamaño esperado más un byte, con límite de
128 MiB y especificación validada antes de I/O. El loader posterior puede
consumir exactamente ese buffer, sin reabrir la ruta.

Los metadatos esperados deben proceder del manifiesto confiable: este módulo
no autentica el manifiesto ni abre rutas, descarga pesos o impone deadlines.
Esos controles pertenecen a T1.2. No implica inferencia MiniLM ni T1 completo.

```sh
cargo test -p muninn-bridge-spike --lib --locked --offline artifacts::
```

## Manifiesto aprobado: T1.2a

`model_manifest::PinnedMiniLmManifest` comprueba la entrada contra el snapshot
canónico del manifiesto incluido al compilar, antes de deserializar. Selecciona
solo MiniLM, con revisión inmutable, once rutas cerradas, hashes/tamaños válidos
y URLs HTTPS exactas de esa revisión. Cambiar el pin requiere revisión y un
nuevo build; no se siguen ramas ni se acepta otro hash desde HTTP.

`verify_artifact` verifica el reader de un archivo declarado y devuelve el
buffer T1.1. Tener sus metadatos no significa tener pesos comprobados ni modelo
preparado. No hay descarga, apertura de rutas, caché ni inferencia en esta unidad.

```sh
cargo test -p muninn-bridge-spike --lib --locked --offline model_manifest::
```

## Apertura por descriptor: T1.2b1

En Linux/macOS, `artifact_fs::ArtifactDir` recorre cada componente de una raíz
absoluta sin seguir symlinks. Los hijos se abren por descriptor con nombres
simples validados. Se comprueban tipo regular, owner y permisos POSIX; el buffer
posterior debe pasar igualmente por verificación de tamaño/hash.

**Esto no verifica ACL extendidas ni habilita una caché completa.** El host debe
comprobar que no existen concesiones adicionales antes de activar el preparador.
No se implementaron escritura, promoción, durabilidad ni descarga. No se ofrece
esta API filesystem en otros targets ni se cambió la compilación restante.

```sh
cargo test -p muninn-bridge-spike --lib --locked --offline artifact_fs::
```

El test de FIFO se ejecuta solo en Linux. Los tests crean directorios con modo
0700 explícito: el default de tempfile no garantiza ese modo.

[ACL.md](ACL.md)
