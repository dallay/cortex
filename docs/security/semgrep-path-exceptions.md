# Excepciones puntuales de rutas en Semgrep

## Alcance y autorización

El usuario autorizó exactamente siete excepciones nuevas para la regla
`rust.actix.path-traversal.tainted-path.tainted-path`, por expresión y no por
archivo. Lens y Mirror aceptaron el triaje y la formulación del riesgo residual:
`ses_e828d89f0ffeUAnQyVDAzESwf7` y `ses_e828d899bffegiooi6Zl3B671x`.
Lens y Mirror dieron `VERDICT CLEAN` al árbol completo respaldado en `ee44ab8`.
Esta unidad B1 reconstruye solo el scanner sobre `origin/main` (`5945002`), sin
hooks ni guards F1/F2. Su evidencia se verifica por separado, no se hereda.

No se cambian I/O, validadores, reglas, severidad, exclusiones ni baseline.
Una anotación solo afecta a la expresión inmediatamente siguiente. Cambiar la
expresión, función, razón, enlace o inventario requiere una nueva revisión.
Estas excepciones no autorizan rutas arbitrarias suministradas por el modelo.

En los cuatro casos del runtime, la validación canónica bloquea traversal por
argumentos en un árbol estable. La sustitución concurrente de archivos o enlaces
entre validación e I/O sigue siendo un riesgo residual **no resuelto**. No hay
confinamiento atómico. Las comprobaciones de metadata y contenido
obsoleto no eliminan esa carrera. Las excepciones no afirman lo contrario.

## huginn-config

- Archivo: `apps/huginn/src/config.rs`; función `Config::load`.
- Expresión: `match std::fs::read_to_string(&resolved)`.
- Origen: opción del operador `--config` o rutas fijas de configuración del host.
  Leer fuera del workspace es intencional. No es una ruta de una tool/modelo.
- `PathBuf` representa una ruta; no la sanea ni confiere autorización por sí mismo.

## huginn-ui-settings

- Archivo: `apps/huginn/src/ui_settings.rs`; función privada `load_or_init`.
- Expresión: `fs::read_to_string(path)`.
- `persist_seeded` recibe del host la ruta de guardado de configuración o la
  configuración anterior como semilla de migración. La UI cambia valores `[ui]`,
  no suministra un filepath. No se concede acceso a rutas del modelo.

## rook-usage-db

- Archivo: `apps/rook/src/di.rs`; función `RookContainer::build`.
- Expresión: `SqliteUsageRepository::new(Path::new(&config.database.db_path))`.
- `db_path` es configuración SQLite del operador en la composición del servidor,
  no un argumento HTTP/modelo. Puede estar fuera de un workspace de Huginn.
- `Path::new` solo convierte la representación; no valida, sanea ni hace I/O.
  El constructor abre la base SQLite autorizada por esa configuración.

## runtime-prepare

- Archivo: `crates/huginn/runtime/src/tools.rs`; método `NativeTool::prepare`.
- Expresión: `Some(tokio::fs::read_to_string(&resolved).await?)`.
- `resolve_path` canonicaliza la raíz y el destino existente, o el padre para
  nuevas escrituras; exige pertenencia a la raíz y rechaza enlaces colgantes.
  La lectura original para la previsualización también comprueba tipo y tamaño.
- Garantía limitada al árbol estable; sustitución concurrente no resuelta.

## runtime-mutation

- Archivo: `crates/huginn/runtime/src/tools.rs`; función privada `execute_mutation`.
- Expresión: `let current = match tokio::fs::read_to_string(path).await`.
- El caller `NativeTool::execute` vuelve a resolver y comprobar la ruta
  contra la preparación antes de llamar a este helper. El helper comprueba tipo,
  tamaño y contenido original para rechazar una aprobación obsoleta.
- Garantía limitada al árbol estable; no hay confinamiento atómico ni solución
  de la sustitución concurrente entre comprobación y lectura/escritura.

## runtime-read

- Archivo: `crates/huginn/runtime/src/tools.rs`; función privada `execute_read`.
- Expresión: `let text = tokio::fs::read_to_string(path).await?`.
- El mismo caller privado de ejecución revalida la ruta del workspace antes de
  despachar la lectura. El helper comprueba archivo regular y límite de tamaño.
- Garantía limitada al árbol estable; sustitución concurrente no resuelta.

## runtime-search

- Archivo: `crates/huginn/runtime/src/tools.rs`; función `execute_search`.
- Expresión: `let Ok(text) = tokio::fs::read_to_string(&file).await`.
- La raíz procede del caller validado. `workspace_files` enumera solo
  entradas regulares/directorios, omite enlaces y directorios de dependencias,
  sin seguir enlaces. Antes de leer, `execute_search` comprueba el límite de
  tamaño. El archivo raíz directo también procede de la resolución canónica.
- Garantía limitada al árbol estable; enumerar sin enlaces no impide una
  sustitución concurrente posterior. No hay confinamiento atómico.

## Inventario histórico separado

Siete anotaciones anteriores de la misma regla permanecen intactas. No se
aprueban ni se reinterpretan en esta resolución. Líneas anteriores al cambio:

- `apps/rook/src/di.rs:495`: `is_in_memory_sqlite_target` (anotación de función).
- `apps/rook/src/di.rs:529`: `resolve_api_key_secret`, conversión con `replacen`.
- `apps/rook/src/di.rs:531`: `resolve_api_key_secret`, conversión directa.
- `apps/rook/src/di.rs:582`: `resolve_api_key_secret`, lectura del secreto.
- `apps/rook/src/config.rs:453`: `RookConfig::load`, lectura de configuración.
- `apps/rook/src/main.rs:328`: `load_config`, carga de configuración.
- `apps/rook/src/main.rs:343`: `load_config_with_path`, carga de configuración.

El verificador compara identidades por archivo/función/expresión, no números de
línea ni un total global de siete. Exige **7 nuevas y 7 históricas** por separado.

## Controles reproducibles

Ejecutar con el Semgrep instalado por CI:

```sh
python3 .github/scripts/check-semgrep-path-exceptions.py
```

Para reproducir el motor de CI sin cambiar la instalación global, instala Semgrep
1.126.0 con Python 3.12 y setuptools 80.9.0 en un entorno aislado. Luego ejecuta:

```sh
python3 .github/scripts/check-semgrep-path-exceptions.py \
  --output-dir "$EVIDENCE_DIR"
```

El script invoca el ejecutable fijo `semgrep` desde `PATH`; no acepta un comando o
argumentos alternativos. El directorio de evidencia debe estar fuera de fuentes.
Sin `--output-dir`, conserva evidencia en un directorio temporal.

El script genera Rust deliberadamente inseguro solo en un temporal externo.
Con `p/rust` y severidad ERROR comprueba JSON, archivos escaneados y códigos:

1. Lectura sin validar desde un argumento: un finding, salida 1.
2. Lectura equivalente con la anotación exacta: cero findings activos, salida 0.
3. La misma lectura con `--disable-nosem`: reaparece un resultado en JSON.
   En 1.126.0 conserva `is_ignored=true` y salida 0 incluso con `--error`;
   por eso no basta comprobar el código de salida.
4. `Path::new` pasado a un validador sin lectura: cero findings en 1.126.0.
   Esta observación del scanner no convierte `Path::new` en un saneador.

Conserva JSON completo con versión/metadatos de regla y stderr por caso.
Los rulesets remotos son mutables: los controles positivos deben fallar si deja
de cargarse la regla. Semgrep local 1.182.0 añade tres avisos sobre conversiones
`Path::new` que no reproduce CI 1.126.0; no tienen autorización de supresión.
La regresión del scanner no sustituye los tests de límites nativos del runtime.

## Evidencia de aplicación

- RED, antes de anotar: escaneo completo con los tres rulesets de CI,
  `--severity ERROR --error --json`: salida 1, siete findings originales.
- RED del inventario, después de aprobar los cuatro controles del scanner:
  salida 1, `authorized inventory mismatch: expected seven NEW exceptions`,
  inventario nuevo vacío. Las siete anotaciones históricas ya coincidían.
- GREEN: los cuatro controles dieron respectivamente 1, 0, 1 y 0 resultados;
  sus códigos fueron 1, 0, 0 y 0. Inventario: 7 nuevas, 7 históricas sin cambios.
- GREEN B1: Semgrep 1.126.0, tres rulesets de CI y
  `--severity ERROR --error --json`: salida 0, cero findings, 226 targets
  (177 Rust, 47 YAML, 2 Dockerfiles). RED: salida 1, siete findings sobre los
  mismos targets. Ninguno pertenece a `target` o `node_modules`.
- Persisten cuatro avisos `PartialParsing` en `release.yml:159,181`: dos reglas
  intentan analizar fragmentos PowerShell como Bash. Coinciden exactamente con
  RED. No se ocultan ni se presentan como parsing completo. Se conserva la
  selección de archivos de CI, incluidas sus exclusiones preexistentes.
- Metadatos de la regla positiva: `r_id=66114`, `rule_id=zdUJ10`,
  `rv_id=1415500`, `version_id=44TbL2Y`, severidad ERROR.
- Nueve regresiones Python del inventario pasan: identidad exacta, desaparición,
  cambio de expresión/función/razón/enlace, anotación histórica eliminada,
  anotación adicional y anotación sin regla explícita.
- La décima prueba Python comprueba la invocación de unittest en CI. El mismo
  paso ejecuta unittest y los controles reales antes del scan y su count gate.
- B1 no modifica comportamiento Rust: solo siete bloques de comentarios.
  No incorpora ni atribuye tests de guards F2. La validación local incluye fmt,
  actionlint, markdownlint y cargo check del workspace mediante el hook obligatorio.

Los JSON y logs B1 están fuera de fuentes, en `huginn-b1-evidence/` del temporal
aprobado: `baseline-red.json`, `controls/`, `complete-green.json` y logs del runner.
Para repetir las pruebas del inventario:

```sh
python3 .github/scripts/test-semgrep-path-exceptions.py
```
