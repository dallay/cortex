# Namespace de sesiones por propietario

## Contrato del prototipo

La identidad de una conversación es `(owner_scope, session_id)`. El mismo
`session_id` puede existir para dos propietarios y para el namespace sin
propietario. Omitir `user_identifier` selecciona únicamente este último; no
selecciona todos los propietarios.

`OwnerScope` conserva el identificador exacto y rechaza identificadores vacíos
o compuestos solo por espacios. La codificación distingue `unowned` de
`owned:<identificador>`, incluso si el identificador contiene esos prefijos.
El scope proporcionado por el caller **no es autenticación**.

El puerto exige scope en resolución, ingesta, lectura, listado, búsqueda y
borrado. Las mutaciones finales filtran también por scope. Un UUID explícito
no permite adoptar una conversación ajena ni cambiar su sesión. Una lectura
por sesión más UUID exige ambas identidades. La lectura solo por UUID sigue
siendo una operación distinta, siempre scoped.

El esquema conserva unicidad de UUID y añade unicidad compuesta para
`(owner_scope, session_id)`. No rellena propiedades legacy al arrancar.
Las operaciones por identidad relevante rechazan conversaciones legacy sin
namespace. Los listados y búsquedas no incluyen esas conversaciones.
Una migración de datos requiere una operación explícita separada.

## Búsqueda y límites de aceptación

La búsqueda intenta primero ANN global con `db.index.vector.queryNodes`, con
`k = limit`. Cypher aplica los filtros de propietario, sesión opcional, identidad
y dimensiones del embedding, metadata y umbral antes de devolver resultados.
Si ese resultado filtrado contiene menos de `limit` filas, ejecuta una búsqueda
exacta scoped y devuelve únicamente el resultado completo de esa consulta,
limitado a `limit`. No mezcla ambas listas ni hace consultas por mensaje.

El fallback calcula coseno sobre los mensajes que cumplen los filtros scoped.
Evita que candidatos ajenos oculten resultados propios cuando ANN queda
incompleto. Si ANN completa el límite, el ranking sigue siendo aproximado: no
se garantiza equivalencia con la búsqueda exacta ni recall completo.

Los errores ANN de índice, dimensiones o almacenamiento se propagan sin fallback.
Los predicados de identidad y dimensiones posteriores a ANN no protegen al
procedimiento frente a dimensiones distintas entre la consulta y el índice.
No hay oversampling, paginación ni configuración adicional.

El fallback puede recorrer todos los mensajes del scope y añade una segunda
consulta cuando ANN queda incompleto. No demuestra rendimiento acotado,
aceptación MiniLM, procedencia C1 ni cierre global del hito H1.

La operación administrativa global `clear_all_data` conserva su control
explícito en el bridge y no equivale a una operación cotidiana sin propietario.

## Idempotencia opcional de mensajes

`add_message` acepta `operation_key` opcional. La clave se limita a la
conversación resuelta por `(owner_scope, session_id)`. La misma clave con el
mismo payload devuelve el mensaje almacenado; la misma clave con otro payload
produce un conflicto sin añadir un mensaje. Sin clave, los mensajes repetidos
siguen siendo escrituras distintas. La clave se puede volver a usar después de
borrar el mensaje al que estaba asociada.

El payload incluye role, content, metadata y `generate_embedding`; no incluye
el UUID generado ni el resultado del proveedor de embeddings. La aplicación
comprueba primero si ya existe la operación, para que un replay no dependa de
otra llamada al proveedor. La escritura mantiene además la comprobación atómica
del adaptador Neo4j para proteger las carreras entre solicitudes concurrentes.
Esta clave no sustituye autenticación ni autorización.

Límite aprobado: la garantía de no llamar al proveedor cubre replays de mensajes
ya persistidos. Dos solicitudes concurrentes que consultan antes de que exista
el mensaje pueden llamar ambas al embedder; la escritura atómica evita el
duplicado y resuelve replay/conflicto. El usuario acepta este límite para evitar
el coste operativo de coordinación/reserva cross-process y recuperación de
operaciones pendientes.

## Validación focalizada

- Pruebas de paquetes: `cargo test -p muninn-core -p muninn-application -p muninn-neo4j -p muninn-bridge --locked`.
- Prueba real opt-in: `cargo test -p muninn-neo4j --test owner_namespace --locked -- --ignored`.
- La prueba opt-in requiere `MUNINN_FIXTURE_URI` y `MUNINN_FIXTURE_PASSWORD`,
  exclusivamente contra una Neo4j sintética nueva. Crea datos de prueba; no
  debe apuntar a una base activa.

La prueba real cubre sesiones homónimas, namespace unowned, UUID ajeno o
incoherente, escritura individual y por lote, lectura, listado, búsqueda con
candidatos ajenos, borrados scoped, resolución concurrente y rechazo legacy.
No sustituye el TCK ni la matriz completa de cadenas y fallos concurrentes.
