# H5: diagnóstico local real Rust/Python

## Contrato autorizado

El usuario autorizó una vía local separada con hashes y trazabilidad, sin
aceptación C1 ni afirmación de paridad. `retrieval_h5_local.py` ejecuta los
exporters existentes y reutiliza el comparador H5. No modifica controles C1.
El bloqueo histórico de procedencia sigue documentado por separado.

Estado: `local_diagnostic_not_c1_acceptance`. Autenticación:
`none_local_hashes_only`. No existe umbral de aceptación nuevo.

## Reproducción

Desde la raíz, con los recursos locales existentes:

```sh
python3 dev/muninn/retrieval_h5_local.py \
  --output tmp/muninn/h5-local-20261107-02
```

El directorio de salida debe ser nuevo. Para repetir, use otro nombre. El runner
rechaza directorios existentes, entradas derivadas incompatibles, hashes de
consumo incorrectos, materiales cambiados e identidades incompatibles. Un intento
fallido puede conservar logs, pero no recibe `local-diagnostic.json` de éxito.
Los archivos parciales nunca deben tratarse como un diagnóstico completo.

El runner es específico de macOS: requiere `/usr/bin/sandbox-exec` y falla si
el proceso protegido no puede ejecutarse. Aplica `(deny network*)` a cada exporter,
incluidos sus descendientes, y un entorno explícito sin credenciales heredadas.
Configura `HF_HUB_OFFLINE=1`, `TRANSFORMERS_OFFLINE=1` y desactiva telemetría.
La prueba negativa de red intenta un bind local y comprueba el rechazo del hijo.

## Evidencia

Intentos conservados: `tmp/muninn/h5-local-20261107-01` y
`tmp/muninn/h5-local-20261107-02`. El segundo incluye la comprobación explícita
del recibo del bundle y la verificación global de materiales entre etapas.
Los exports de ambos intentos son idénticos byte a byte.

El segundo intento contiene:

- `rust/stdout.json`: SHA-256 `b1b55dd95fca28ca3b591e84002060c156d7861896a2591a5caadc5478c36199`.
- `python/stdout.json`: SHA-256 `7e956597ffbfba910c88f8ef51160e47ac4a45bfeddc28140f74760283bfa9b6`.
- `comparison.json`: SHA-256 `104bceda3a737486b330d94bf903b0565a040d43d4a92be846c6aee414907a07`.
- `local-diagnostic.json`: SHA-256 `922d141633b31459cf93f36cda97a27cd3ca83acd065062291b4a18472f5a2e6`.
- `bundle-verification.jsonl`: verificación fresca de los 11 archivos fijados,
  91578415 bytes, antes de cargar el modelo. El recibo del bundle también debe
  tener SHA-256 igual a `ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1`.
- `rust/stage.json` y `python/stage.json`: comandos exactos, hashes de binario,
  fuentes, entradas, modelo, outputs y logs; entorno offline y duración real.

Binario Rust: `target/debug/export_minilm_c1`, SHA-256
`491def1c28b39fa4821dc0c95c14d3cb909769f152892fc04d66dd016386fcff`.
Build previo permitido: `cargo build --locked --offline -p muninn-c1-exporter --features cpu-f32 --bin export_minilm_c1`.
Toolchain: rustc 1.99.1 y cargo 1.99.1; detalles en el informe del bloqueo previo.
Los hashes de fuentes son trazabilidad local, no prueba autenticada de que esas
fuentes produjeron el ejecutable. Dependencias Rust: `Cargo.lock` preservado.

Python: 3.13.18; torch 2.14.1; sentence-transformers 6.2.0; transformers 5.20.2;
tokenizers 0.23.3; numpy 2.5.4; safetensors 0.8.0. CPU/F32, atención `sdpa`,
modo evaluación, longitud máxima 256. Plataforma: macOS 27.3 arm64.
El exporter Rust no proporciona recibo de runtime interno; el runner registra
el hash del ejecutable y la configuración externa, sin inventar ese recibo.
Duración segundo intento: Rust 11,514 s; Python 4,119 s. No son benchmarks.

## Resultado numérico descriptivo

Se compararon 18 vectores singleton, 384 componentes cada uno, correspondientes
a seis queries y doce ocurrencias de candidatos. Las matrices de tokens
recomputadas coinciden exactamente entre ambos exports.

- Error absoluto máximo de componente: `2.4697374129308003e-7`.
- Coseno mínimo entre vectores Rust/Python correspondientes: `0.9999999999991336`.
- Diferencia máxima entre normas: `1.8457344463929104e-7`.
- Los seis órdenes coinciden entre Rust, Python y el reporte histórico.
- No hay empates exactos. El margen mínimo Rust entre candidatos es
  `0.014000570722189992` en `query-04`.

Los órdenes, sin inferir relevancia, son:

- query-01: candidate-01, candidate-02.
- query-02: candidate-04, candidate-03.
- query-03: candidate-06, candidate-05.
- query-04: candidate-08, candidate-07.
- query-05: candidate-09, candidate-10.
- query-06: candidate-11, candidate-12.

El reporte histórico conserva SHA-256
`674ff3518d7865d684e568419ec22f4f6cf7494bf5ae994f25f32d5bedac78da`.
No se cambiaron modelo, encoder, corpus, juicios, ranking ni thresholds.

## Pruebas y límites

RED inicial: unittest falló por ausencia del módulo nuevo. GREEN: tres pruebas
pasaron después de implementar ejecución, rechazo de consumo incorrecto y
rechazo de deriva. Una prueba adicional del recibo del bundle falló antes de
implementar su comprobación; después pasaron las cinco pruebas locales.
Refactor: se añadió la comprobación global de materiales entre etapas y se
repitieron las pruebas y la ejecución real con directorio nuevo.

Verificación final: 47 pruebas `test_retrieval*.py`, 24 de `test_c1_runner.py`,
5 de `test_c1_attempt_run.py` y 5 de `test_c1_provenance.py`, todas OK.
La sesión padre completó la revisión independiente: confirmó las métricas y
halló dos problemas MEDIA, corregidos según el registro siguiente. No se ejecutó
CI completo.

Esta comparación no captura los vectores originales del servicio histórico.
La escala Neo4j permanece `blocked/unverified-scale`; no se restan sus scores
a cosenos ni se supone una transformación. Los resultados no aceptan C1, H5,
paridad integral ni compatibilidad universal. No hubo red, descargas, servicios
ni acceso a bases de datos. Los exports contienen vectores y tokens: mantener
los artefactos locales bajo las mismas restricciones de los inputs.

## Isolated ANN/exact pilot run (2026-11-08)

The locally built feature-enabled ARM64 bridge image was inspected by immutable
image ID `sha256:3d21785ca096ecebeb75b077268b092898f8e82d1899790dd62c532bb2f00111`.
Its bridge binary SHA-256 is
`a7716eb271093112907c8c608ba86526760b1f10f237955c4dfb165424161c05`. The image
was built offline/locked and no image pull was used.

The isolated project was `muninn-h5-pilot-v1`. Preflight found no existing
resources with that exact Compose project label. Compose resolved with zero
published ports, an internal network, Neo4j `/data` on tmpfs, the existing model
bundle staged read-only, and dedicated credential files staged read-only through
a project-scoped volume. No external database endpoint was configured. A first
runner attempt stopped at bridge-token authorization before adding pilot data;
inspection found that the ephemeral token did not meet the bridge's hex-token
contract. Only resources created by that attempt were removed. A correctly
formatted ephemeral token was generated without logging its value, and a clean
retry completed.

The sanitized report is
`tmp/muninn/retrieval-h5-20261108-123831.json` (SHA-256
`a9cc20a7b4b4c7d94aa2e207ed7b50af3695e9e00fc52ba64d85e754ce8504e0`). It records
six queries and alias-only ANN/exact rankings. No internal message IDs, owner
identifiers, corpus text, vector payloads, or credentials are included. ANN and
exhaustive exact order match in all six queries. The maximum observed absolute
score difference is `0.003067493438720703`. This is a measured difference only;
no score transformation, equivalence assumption, or tolerance was applied.

With a global search limit of 12, threshold 0, and query-specific owner/session
scope, macro metrics against the current judgments are Recall@1/3/5
`0.0/1.0/1.0`, MRR `0.5`, and nDCG@1/3/5
`0.0238095238/0.6440764182/0.6440764182`. The judgments identify at least one
relevant candidate in each query; five queries have no relevant result at rank
1, while all six have one by rank 2. One query's first result is partially
relevant. Annotation provenance remains single-reviewer and not adjudicated.

No rank discrepancy was observed, so no ranking or retrieval-scope change was
made. The report state is `measured_not_accepted`: no approved acceptance
threshold exists. These results do not accept H5, establish universal hardware
support, or establish ANN/exact score parity. The focused retrieval unittest
suite passed 50 tests. The attempt's Compose containers, network, and named
volumes were removed after copying and checking the report. A later read-only
inventory found zero containers, volumes, or networks with the exact
`com.docker.compose.project=muninn-h5-pilot-v1` label; the temporary credential
directory was absent. The image and model bundle were retained. Independent review
verified the report hash, recomputed the metrics, confirmed alias-only output and
the six matching orders, and found no acceptance threshold in the report. The
reviewer identified stale task notes, now corrected.

## Correcciones tras revisión independiente

La revisión identificó deriva no detectada de outputs/recibos entre etapas y
captura de stdout/stderr sin límite de disco. Se corrigieron sin cambiar C1:

- Se conservan hashes originales de stdout, stderr y `stage.json`. Se verifican
  antes de comparar y antes de publicar `local-diagnostic.json`. El comparador
  continúa rechazando exports incompletos o identidades incompatibles.
- Cada canal admite como máximo 8 MiB. El controlador lee pipes con `selectors`
  y comprueba el límite antes de escribir cada bloque. Al excederlo, aborta sin
  recibo de etapa/final y termina el grupo de procesos. También termina/recolecta
  procesos ante timeout o error y cierra pipes. Los logs parciales se conservan,
  acotados; no se sobrescriben intentos anteriores.
- `file_record` calcula SHA-256 por bloques de 64 KiB, sin `read_bytes` completo.

RED: ocho pruebas locales dieron un fallo y cuatro errores por ausencia de las
nuevas verificaciones y hashing todavía no acotado en memoria. GREEN: ocho
pruebas locales OK. Las regresiones cambian JSON válido añadiendo whitespace a
stdout/recibo de etapa y comprueban rechazo antes del marcador final; también
exceden stdout y stderr por separado y comprueban captura acotada sin recibo.
Refactor: la publicación final usa `publish_receipt`, probada directamente con
la misma evidencia alterada. Suite retrieval final: 50 OK; C1 runner 24 OK,
attempt 5 OK y provenance 5 OK.

**Límite de seguridad:** el sandbox bloquea red, no acceso al filesystem. Este
runner ejecuta exporters locales de confianza; no es un entorno para ejecutar
código hostil. Los hashes y comprobaciones son puntuales, no previenen toda
carrera de escritura concurrente. El límite de captura protege los dos canales,
no archivos arbitrarios que un proceso pudiera crear fuera de esos canales.

Nueva ejecución real con código corregido:
`python3 dev/muninn/retrieval_h5_local.py --output tmp/muninn/h5-local-20261107-03`.
Código 0; Rust 11,384 s y Python 3,889 s. Los exports y `comparison.json` son
idénticos byte a byte a los del intento 02; las métricas anteriores no cambian.
El nuevo recibo `tmp/muninn/h5-local-20261107-03/local-diagnostic.json` tiene
SHA-256 `2d729108804a08aed12291fe52e9224cd598a5ab394cf52e836043335479f740`.
Los intentos 01/02 y el informe histórico se conservan. La nueva corrección queda
lista para comprobación del reviewer; no convierte el diagnóstico en aceptación.
