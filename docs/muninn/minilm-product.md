# MiniLM local en Muninn

## Estado y límites

El proveedor local es un candidato CPU/F32. Su compilación y sus pruebas de
configuración no equivalen a aceptación numérica C1 ni a cierre H1.
No se ejecuta inferencia C1 sin procedencia verificada del exportador.

## Configuración explícita

Compilar desde el workspace raíz:

```sh
cargo build --locked -p muninn --features minilm-cpu
```

Seleccionar `--embedding-provider minilm` y proporcionar:

- `--embedding-bundle-path`: raíz local del cache de artefactos existente.
- `--embedding-bundle-id`: digest del bundle fijado por `artifact-lock.json`.

También se admiten `NAM_EMBEDDING__BUNDLE_PATH` y
`NAM_EMBEDDING__BUNDLE_ID`. El proveedor no descarga artefactos. Comprueba el
manifiesto, el recibo y los bytes del bundle antes de cargar el encoder.
Una configuración inválida impide abrir la conexión Neo4j y servir solicitudes.
Las dimensiones del proveedor local son 384; el esquema usa ese valor.

La inferencia síncrona usa `spawn_blocking` y un permiso de ejecución por
instancia. El permiso se conserva dentro del trabajo bloqueante aunque se
cancele el futuro que espera el resultado. Los errores no exponen textos ni
rutas del bundle.

## Identidad del espacio vectorial

Cada mensaje con vector conserva identidad y dimensiones. La búsqueda exige la
misma identidad, además del propietario y los filtros solicitados, antes de
ordenar y limitar. Los vectores legados sin identidad no se mezclan con datos
nuevos. No se migran automáticamente ni se eliminan.

MiniLM fija bundle y contrato `minilm-cpu-f32-v1`. Hash usa un espacio distinto
para el TCK y no ofrece semántica. Los proveedores OpenAI compatibles requieren
`--embedding-identity` o `NAM_EMBEDDING__IDENTITY`, con una identidad inmutable
que describa revisión y preprocesamiento. Una etiqueta mutable del modelo no
prueba compatibilidad del espacio; corresponde al operador fijar esa identidad.

## Comprobaciones sin inferencia

```sh
cargo test --locked -p muninn --features minilm-cpu --bin muninn
cargo test --locked -p muninn-providers --features minilm-cpu
cargo clippy --locked -p muninn -p muninn-providers \
  --all-targets --features minilm-cpu -- -D warnings
```

Estas comprobaciones no cargan pesos ni acreditan recuperación semántica real.

## Búsqueda en Neo4j 5.26

La búsqueda intenta primero ANN global con `k = limit`. Cypher aplica después
los filtros de propietario, sesión opcional, identidad y dimensiones del
embedding, metadata opcional y umbral antes de devolver resultados. Si quedan
menos de `limit` filas, ejecuta una consulta exacta scoped y devuelve únicamente
su resultado completo, limitado a `limit`, sin mezclar ambas listas.

Este híbrido no es ANN nativo scoped. Si ANN completa `k`, el ranking sigue
siendo aproximado y no garantiza equivalencia exacta ni recall completo.
Los errores ANN de índice, dimensiones o almacenamiento se propagan sin
fallback. Los predicados posteriores a ANN no protegen al procedimiento frente
a dimensiones distintas entre consulta e índice. El fallback añade una segunda
consulta y puede recorrer todos los mensajes del scope; no acredita rendimiento
acotado, aceptación C1/MiniLM ni cierre global H1.
