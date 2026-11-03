# Entorno de referencia de Muninn

Primera unidad de G0: inventario estático y captura inicial del oráculo fijado.
No implementa un servidor Muninn ni demuestra compatibilidad TCK.
Consulta [la referencia y sus brechas](../../docs/muninn/reference-lock.md).

## Requisitos

- Git, Python 3.11 o posterior y Docker con Compose.
- Usuario local sin privilegios root y acceso a Docker.
- Red para obtener fuentes, imágenes y dependencias durante la preparación.

Se validó el contexto `dory` sin remapeo rootless de UID/GID. Docker rootless o
`userns-remap` no están validados: el UID del host puede no coincidir con el
propietario del bind mount dentro del contenedor. En esos contextos, prepara
permisos o ACL para el UID realmente mapeado antes de ejecutar la captura.
No cambies el capturador a root ni hagas el directorio escribible por todos.

No uses una base existente, cuentas externas ni credenciales de producción.
El entorno no publica puertos ni monta datos persistentes de otros productos.

## Preparación

Desde la raíz del repositorio:

```sh
test "$(id -u)" -ne 0
mkdir -p dev/muninn/.cache/sources dev/muninn/artifacts
chmod 700 dev/muninn/.cache
export MUNINN_REFERENCE_ROOT="$PWD/dev/muninn/.cache/sources"
export MUNINN_ORACLE_UID="$(id -u)"
export MUNINN_ORACLE_GID="$(id -g)"

git clone --filter=blob:none --no-checkout \
  https://github.com/neo4j-labs/agent-memory.git \
  "$MUNINN_REFERENCE_ROOT/agent-memory"
git -C "$MUNINN_REFERENCE_ROOT/agent-memory" checkout --detach \
  9d45af5529ecb5f2fe352013468a11087f3163d4
git clone --filter=blob:none --no-checkout \
  https://github.com/neo4j-labs/agent-memory-tck.git \
  "$MUNINN_REFERENCE_ROOT/agent-memory-tck"
git -C "$MUNINN_REFERENCE_ROOT/agent-memory-tck" checkout --detach \
  e60b410f1046dc21db50f9aba48e171dc8a9b5a1
```

Si ya existen checkouts, no los sobrescribas ni ejecutes `reset` o `clean`.
Verifica el inventario con el comando siguiente. Un commit incorrecto o archivos
versionados modificados provocan un error. El manifiesto de integridad también
rechaza módulos Python adicionales dentro de los paquetes importables.

```sh
python3 dev/muninn/reference.py --sources "$MUNINN_REFERENCE_ROOT" \
  --output docs/muninn/specs/parity-matrix.csv \
  --integrity-output dev/muninn/source-integrity.json --check
python3 -m unittest discover -s dev/muninn/tests -v
```

Para regenerar los artefactos después de una revisión explícita de referencia,
omite `--check`. No cambies los commits para hacer que un test pase.
El inventario contiene materiales Apache-2.0; conserva las licencias de
`docs/muninn/licenses/` al distribuirlo.

## Secreto local de prueba

Este comando crea el secreto **solo si no existe**. No lo imprime ni lo añade a
Git. El directorio `.cache` restringe el acceso en el host; el archivo es legible
por el usuario no privilegiado del contenedor mediante el montaje del secreto.

```sh
python3 - <<'PY'
from pathlib import Path
import secrets
path = Path("dev/muninn/.cache/neo4j.auth")
with path.open("x", encoding="utf-8") as output:
    output.write("neo4j/" + secrets.token_hex(24))
path.chmod(0o444)
PY
```

## Construcción y captura

```sh
docker compose -f dev/muninn/compose.yaml --profile capture config --quiet
docker compose -f dev/muninn/compose.yaml --profile capture build oracle
docker compose -f dev/muninn/compose.yaml --profile capture run --rm oracle
```

Las imágenes se fijan por digest. El Dockerfile instala únicamente las
dependencias base exportadas del `uv.lock` de upstream con versiones y hashes;
no instala el paquete desde PyPI ni los extras de modelos. El capturador copia
únicamente bytes autorizados a un árbol temporal limpio e importa esa copia,
nunca el `src` original. Rechaza bytecode y archivos adicionales dentro del
paquete; los módulos hermanos de `src` no se copian. También verifica los recursos
de datos del paquete. El contenedor final no
contiene herramientas de construcción y funciona sin usuario root.

El artefacto completo se escribe en `dev/muninn/artifacts/oracle-gm01.json`.
Contiene únicamente mensajes sintéticos. Los códigos de salida son:

- **0:** captura terminada y comprobación local de eliminación aprobada.
- **1:** fallo de preparación, integridad, conexión o precondición; no hay una
  captura nueva. Una captura anterior puede seguir existiendo sin cambios.
- **2:** captura guardada con discrepancia normativa. Es el resultado observado
  para `delete_message` en esta referencia. No equivale a un TCK aprobado.

La captura comprueba ingestión/orden/metadatos, identidad estable y separación de
sesiones. Registra esquema y relaciones reales. No mide extracción, embeddings,
ranking, autorización multiusuario ni discovery MCP. No captura razonamiento.

Un segundo intento sobre la misma base no vacía falla sin borrar datos.
Para detener exclusivamente **este entorno sintético y efímero**, ejecuta:

```sh
docker compose -f dev/muninn/compose.yaml down
```

Los datos de Neo4j están en `tmpfs`; al retirar ese contenedor desaparecen.
Los artefactos locales se conservan. No uses comandos de borrado general ni
recetas de reset de bases persistentes para preparar esta prueba.

## Captura comparativa GM-01 aislada

Esta ejecución está separada de la captura del oráculo anterior. El runner
verifica y materializa el checkout fijado antes del build. Construye el binario
Muninn actual en Linux arm64 desde un contexto privado acotado y ejecuta el
capturador Python dentro de Compose. No copia el ejecutable del host.

Requiere autorización para iniciar los tres servicios y ejecutar el build. No
ejecutes este comando si solo se autorizó validar la configuración:

La imagen Neo4j fijada debe existir localmente antes de ejecutar el runner. El
runner inspecciona el ID de la imagen antes del build y falla sin iniciar los
servicios si la imagen falta o el ID no tiene formato válido. No descarga Neo4j.
Si hace falta prepararla, ejecuta este pull manual por separado:

```sh
docker pull neo4j:5.26-community@sha256:414adfe41c6ff32abb752be37f265c77607c166cab298f65a917fc4f05776218
```

El runner registra la referencia y el ID local validado en `neo4j_image` del
manifiesto final o bloqueado. El paso explícito de preparación
`docker compose build --pull capture` puede descargar las imágenes base Rust y
Python fijadas por digest; el runtime sigue sin descargar.

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 dev/muninn/gm01_runner.py \
  --reference dev/muninn/.cache/sources/agent-memory \
  --evidence .agents/rpi/plan/tasks/muninn-h1-delivery/evidence/gm01-isolated-nueva
```

El proyecto contiene `reference-db`, `rust-db` y `capture`, en una red `internal`
sin puertos publicados. El bridge se inicia como hijo de `capture` y enlaza solo
su loopback. Python usa los nombres DNS de las dos bases. El capturador no es root,
usa un filesystem de solo lectura y solo escribe en el directorio nuevo de
evidencia. No hay socket Docker dentro de ningún contenedor. Compose elimina solo
el proyecto de esta ejecución.

La preparación ejecuta `docker compose build --pull capture`; puede descargar las
imágenes base Rust y Python fijadas por digest y las dependencias de compilación.
Antes del build, el runner ejecuta `docker image inspect` para la imagen Neo4j
fijada y valida que el ID local sea `sha256:<64 hex>`. Si la imagen no está
disponible o el ID no es válido, el runner guarda el bloqueo y no ejecuta el build
ni `up`. No hace pull automático de Neo4j. Después del build, el host inspecciona
el tag único de `capture` y registra su ID local en `capture_image.id`; también
registra la referencia e ID local de Neo4j en `neo4j_image.reference` y
`neo4j_image.id`. La ejecución usa `docker compose up --pull never --no-build`;
no descarga ni reconstruye imágenes. Neo4j y las imágenes base están fijadas por
digest. La contraseña llega a Neo4j mediante `NEO4J_AUTH_FILE`; no aparece en argv
ni en el manifiesto. El manifiesto registra hashes locales, pero no es una
atestación C1. `h1_acceptance` permanece `not_established`, incluso si
`comparison` es `pass`.

Las pruebas unitarias no inician build ni servicios:

```sh
PYTHONPATH=dev/muninn python3 -m unittest discover \
  -s dev/muninn/tests -p 'test_gm01*.py' -v
```

## Reproducir la exportación de dependencias

Con `uv` 0.9.5, desde el checkout fijado de `agent-memory`:

```sh
uv export --frozen --no-dev --no-default-groups --no-emit-project \
  --no-header --no-annotate --output-file /ruta/al/worktree/dev/muninn/requirements-oracle.txt
```

La exportación no sustituye el entorno separado del TCK ni fija los
modelos de extracción. No se debe interpretar esta captura como G0 terminado.

## Esquema no vectorial contra fixtures aislados

El contrato de esquema no vectorial se valida contra fixtures Neo4j 5.26
efímeras, no contra una base existente. Cada escenario recibe su propio
contenedor Neo4j nuevo con almacenamiento `tmpfs`, de modo que ningún escenario
observa el esquema ni los datos de otro.

El runner no implementa el contrato: lo valida. El sujeto bajo prueba es la
prueba de integración Rust `crates/muninn/neo4j/tests/schema_non_vector.rs`, que
se compila dentro de Dory y se ejecuta contra el fixture.

```sh
docker run --rm -v "$PWD:/src" -v dallay630-linux-target:/target \
  -e CARGO_HOME=/tmp/cargo -e CARGO_TARGET_DIR=/target -w /src \
  rust:1.99.1-slim@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247 \
  cargo +1.99.1 test --locked --no-run -p muninn-neo4j --test schema_non_vector

python3 dev/muninn/schema_runner.py \
  --evidence .agents/rpi/plan/tasks/muninn-h1-delivery/evidence/schema-isolated-nueva
```

El runner crea una red `internal` sin puertos publicados, monta el secreto de
autenticación en solo lectura, aplica límites de memoria y capabilities al
contenedor, y elimina contenedores, red y secretos al terminar. Un escenario
cuyo fixture no arranca se marca `BLOCKED` y se reintenta una vez con un
fixture nuevo; ese reintento no sustituye un resultado de esquema.

Cada directorio de evidencia debe ser nuevo. El runner escribe un log por
escenario y un `summary.json` con el estado, el contenedor, el código de salida
y la limpieza. Código **1** significa fallo o bloqueo, no un contrato aprobado.

## TCK contra el oráculo

El [informe de línea base](../../docs/muninn/reference-tck.md) distingue el
adaptador oficial de la eliminación Python nativa. No mide conformidad de
Muninn. Se preservan los tests oficiales y se sustituye solo su fixture de
adaptación; no se parchea upstream.

Con las variables y el secreto preparados en los pasos anteriores:

```sh
docker compose -f dev/muninn/compose.tck.yaml config --quiet
docker compose -f dev/muninn/compose.tck.yaml build tck
docker compose -f dev/muninn/compose.tck.yaml run --rm tck
docker compose -f dev/muninn/compose.tck.yaml down

docker compose -f dev/muninn/compose.tck.yaml run --rm tck \
  --sources /references --profile native-delete \
  --output /artifacts/tck-native-delete
docker compose -f dev/muninn/compose.tck.yaml down
```

Ambas ejecuciones de la línea base devuelven **2** por los fallos observados.
Ejecuta `down` también cuando el runner falle. Detiene solo el proyecto
`muninn-tck-reference` y retira su base sintética `tmpfs`; no borra artefactos ni
bases persistentes. No ejecutes ambos perfiles simultáneamente sobre esa base.
La propiedad atómica hace que un segundo propietario falle sin limpiar datos
del primero. La base también debe estar vacía al comenzar una nueva ejecución.

Cada directorio de salida debe ser nuevo: el runner no sobrescribe evidencia.
Si ya existe, elige otro nombre explícito con `--output`; no borres informes
para ocultar resultados. Código **1** significa bloqueo, no un TCK aprobado.

La suite adaptada recopila 189 casos. El perfil `native-delete` selecciona 13
tests de eliminación de mensajes, excluye `delete_conversation` y deselecciona
los otros 176. No es una ejecución nativa completa de todos los contratos.
Los informes conservan razones de omisión, cláusulas, fuentes, versiones,
hashes y resultados. Una selección inesperada bloquea el resultado.

La imagen instala `requirements-tck.txt` con hashes. Se obtuvo con `uv` 0.9.5:

```sh
uv pip compile --generate-hashes --python-version 3.13 --no-header --no-annotate \
  --output-file dev/muninn/requirements-tck.txt dev/muninn/requirements-tck.in
```

`requirements-tck.in` incluye las dependencias obligatorias del TCK fijado y el
lock base del oráculo. No instala testcontainers: la infraestructura ya es
explícita en Compose. Ningún contenedor accede al socket Docker.

### Regresión concurrente de propiedad

Esta prueba es opt-in y exige el mismo entorno efímero vacío. En la ejecución
unitaria del host queda omitida de forma explícita; se verificó por separado
dentro del contenedor:

```sh
docker compose -f dev/muninn/compose.tck.yaml run --rm \
  -e MUNINN_TCK_SAFETY_INTEGRATION=1 --entrypoint /opt/venv/bin/python tck \
  -m unittest discover -s /harness/tests -p test_tck_safety_integration.py -v
docker compose -f dev/muninn/compose.tck.yaml down
```

No se debe activar contra servicios persistentes ni eliminar la comprobación
de base vacía. Una marca ajena o ambigua bloquea la limpieza.

## Discovery MCP aislado

La captura usa solo las fuentes verificadas: no requiere Neo4j, el secreto de
prueba anterior ni una cuenta NAMS. No monta el harness del host, el directorio
completo `.cache` ni secretos; monta exclusivamente las fuentes y los artefactos.
Consulta [resultados y límites](../../docs/muninn/reference-mcp.md).

Con `MUNINN_REFERENCE_ROOT`, `MUNINN_ORACLE_UID` y `MUNINN_ORACLE_GID` definidos
en la preparación:

```sh
docker compose -f dev/muninn/compose.mcp.yaml config --quiet
docker compose -f dev/muninn/compose.mcp.yaml build discovery
docker compose -f dev/muninn/compose.mcp.yaml run --rm discovery
docker compose -f dev/muninn/compose.mcp.yaml down
```

El contenedor usa `network_mode: none`. El HTTP probado vive únicamente en su
loopback; no publica puertos. Se ejecutan cuatro registros y ocho sesiones:
`core`/`extended`, `settings=None`/NAMS offline, por stdio y Streamable HTTP.
El lifespan NAMS es el original con `validate_on_connect=False`, endpoint de
prueba y una clave sintética efímera. Eso no demuestra autenticación ni
operaciones del servicio.

La política de los hijos prohíbe conexiones y DNS; HTTP solo puede hacer bind
en `127.0.0.1:8765`. El control negativo habilita el probe y debe observar una
denegación atribuida a `127.0.0.1:9`. Una excepción ajena, cobertura incompleta,
fuga del canary o cierre por SIGKILL impiden publicar el snapshot final.
El hook Python no es una auditoría exhaustiva de llamadas nativas.

El resultado correcto de **esta captura** es salida **0** y
`dev/muninn/artifacts/mcp-discovery/discovery.json`. No es un certificado de
paridad de Muninn. La salida **1** indica bloqueo; ejecuta `down` también en
ese caso. Los logs parciales de un intento fallido se conservan para diagnóstico.

La salida debe ser un directorio nuevo. Para un rerun sin sobrescribir evidencia:

```sh
docker compose -f dev/muninn/compose.mcp.yaml run --rm discovery \
  --sources /references --output /artifacts/mcp-discovery-otra
```

`--serve`, perfil/backend, transporte y ruta de auditoría son opciones internas
del helper hijo. La entrada normal captura la matriz completa.
La imagen contiene las herramientas y manifiestos por copia explícita y
fija sus dependencias desde el `uv.lock` upstream con los extras `mcp` y `nams`:

```sh
# Desde el checkout fijado de agent-memory; uv 0.9.5.
uv export --frozen --no-dev --no-default-groups --extra mcp --extra nams \
  --no-emit-project --no-header --no-annotate \
  --output-file /ruta/al/worktree/dev/muninn/requirements-mcp.txt
```

No omitas hashes ni cambies la referencia para hacer que una captura apruebe.

## API consumible por backend

El inventario canónico es adicional al CSV1335 y no importa código upstream:

```sh
python3 dev/muninn/api_inventory.py --sources "$MUNINN_REFERENCE_ROOT" \
  --output docs/muninn/specs/api-profile.csv \
  --metadata .agents/rpi/plan/tasks/muninn-h1-delivery/evidence/api-profile.json --check
```

Incluye fachada, fábrica, contratos y accesores concretos; separa exports,
sentinelas, rechazos, stubs y estado local. Los bindings y notas son entradas
curadas sujetas a revisión. No equivale a pruebas runtime de 322 capacidades.
Para regenerar después de una modificación revisada, omite `--check`; vuelve a
ejecutarlo tras estabilizar el generador, cuyo hash forma parte del metadato.

## Recibos de modelos y APOC

Consulta [selección, hashes y límites](../../docs/muninn/reference-api-artifacts.md).
El validador no descarga ni instala modelos:

```sh
python3 dev/muninn/artifact_lock.py \
  --input dev/muninn/fixtures/artifact-lock.json \
  --sources "$MUNINN_REFERENCE_ROOT"
```

Esto comprueba estructura, perfil obligatorio, origen Git/lock y versiones
declaradas. No revalida el servicio del publicador ni las URLs por red.
En este workspace, los 16 archivos pequeños leídos durante la captura pueden
volver a comprobarse con `--cache dev/muninn/.cache/model-evidence`.
En un clon limpio esa caché no existe: no se cuentan los archivos ausentes como
verificados. Los dos pesos safetensors no están materializados ni se verifican
por ejecutar este comando sin caché.

El recibo contiene revisiones, URLs y sumas para materialización posterior.
Cambiar referencias exige un cambio de compatibilidad revisado; no se sigue
`main` ni se adapta un hash para conseguir un resultado aprobado. No hay un
loader de modelos implementado por estas herramientas. Un rango spaCy declarado
compatible no prueba carga, extracción, resolución ni calidad de recuperación.

La inspección del APOC empaquetado es independiente y no inicia una base:

```sh
docker run --rm --pull=never --network none --read-only --cap-drop ALL \
  --security-opt no-new-privileges --user 7474:7474 --entrypoint sh \
  neo4j:5.26-community@sha256:414adfe41c6ff32abb752be37f265c77607c166cab298f65a917fc4f05776218 \
  -c 'sha256sum /var/lib/neo4j/labs/apoc-5.26.32-core.jar; stat -c "%s %n" /var/lib/neo4j/labs/apoc-5.26.32-core.jar'
```

La imagen debe estar ya disponible por el digest fijado. No se usan descargas
flotantes de plugins, bases persistentes ni secretos. Esa lectura no activa el
plugin ni acredita que sus funciones sean compatibles.

## Smoke funcional APOC posterior

El recibo original del JAR continúa describiendo solo bytes. La prueba posterior
`.agents/rpi/plan/tasks/muninn-h1-delivery/evidence/apoc-smoke.json` verifica únicamente `apoc.version()` real.
No convierte el recibo de metadatos en aceptación de todas las funciones.

```sh
docker compose -f dev/muninn/compose.apoc.yaml build
docker compose -f dev/muninn/compose.apoc.yaml run --rm probe
docker compose -f dev/muninn/compose.apoc.yaml down
```

La imagen derivada copia el JAR empaquetado solo tras verificar su SHA-256, sin
red ni descargas de plugins. Se usa el allowlist mínimo `apoc.version`, base
sintética `tmpfs`, secretos de fixture y ningún puerto publicado. No se activa
import/export de archivos ni `apoc.*` sin restricciones. La salida debe ser
nueva; la prueba falla sin el JAR y aprueba con el plugin cargado. No prueba
extracción, carga de modelos o compatibilidad completa.

El goal continuo se sigue en `.agents/rpi/plan/tasks/muninn-functional-parity-goal.md`.
El [spike Rust](bridge/README.md) está separado de la arquitectura permanente.
