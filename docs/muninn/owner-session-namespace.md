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

La búsqueda actual calcula similitud coseno dentro del propietario antes de
aplicar el límite. No toma candidatos de un top-k ANN global para filtrarlos
después: ese orden puede ocultar resultados propios por actividad ajena.
Esta implementación EXACT **no demuestra aceptación ANN nativa**, rendimiento
acotado, aceptación MiniLM, procedencia C1 ni cierre del hito H1.

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
