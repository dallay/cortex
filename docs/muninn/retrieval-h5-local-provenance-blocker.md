# H5: bloqueo de procedencia en la continuación local

## Resultado

Estado: **BLOCKED — missing_build_provenance**. El 7 de noviembre de 2026 se
compiló el exporter Rust autorizado, pero no se ejecutó inferencia. No se
produjeron exports ni una comparación numérica real. No se declara paridad.

El runner C1 actual exige una atestación para cualquier ejecutable distinto del
intérprete Python del proceso controlador. Un hash calculado tras una compilación
local identifica bytes, pero no satisface ese contrato. No se modificó el runner,
no se simuló una atestación y no se usó un intérprete como envoltorio para eludir
la verificación del binario Rust.

## Evidencia local

- HEAD Cortex: `34ef2d4afafeb9b28df11596f1f5bc77aeb1eeed`.
- Referencia local comprobada: `/Users/acosta/Dev/dallay/cortex/tmp/project_examples/memory/agent-memory`.
- HEAD de referencia: `9d45af5529ecb5f2fe352013468a11087f3163d4`.
- Compilación autorizada: `cargo build --locked --offline -p muninn-c1-exporter --features cpu-f32 --bin export_minilm_c1`.
- Resultado: código 0, perfil `dev`, 12,55 segundos; cuatro advertencias de
  constantes obsoletas en `third_party/candle-core/src/cpu/erf.rs`. No se
  suprimieron advertencias ni se cambiaron fuentes.
- Binario generado: `target/debug/export_minilm_c1`.
- SHA-256 del binario: `491def1c28b39fa4821dc0c95c14d3cb909769f152892fc04d66dd016386fcff`.
- Toolchain: `rustc 1.99.1 (58cf8f9c3 2026-10-13)` y
  `cargo 1.99.1 (5f94df478 2026-08-27)`.
- SHA-256 de `Cargo.lock`: `be4b861280f44ed09788d13d4e23e0a4594a92db0cf834f782bf7b854ed64198`.

La comprobación directa de `c1_runner._verify_build` con el binario real y su
hash real devolvió `RunnerError.category == 'missing_build_provenance'`.
La comprobación ocurre antes de leer entradas o ejecutar un proceso externo.
El método es interno; esta prueba documenta el rechazo actual, no añade una API.

El bloqueo se explica en estos controles existentes:

- `dev/muninn/c1_runner.py`, líneas 242–253: exige procedencia para binarios nativos.
- `dev/muninn/c1_attempt_run.py`, líneas 115–135: rechaza un plan nativo sin procedencia.
- `dev/muninn/c1_provenance.py`: exige verificación de ambos sujetos con identidad
  del workflow protegido de `main`, commit fijado y registro de materiales.
- `dev/muninn/c1_attempt_plan.json`: contiene valores requeridos sin completar
  para commit/toolchain y rutas `artifacts/muninn/c1/build-record.json` y
  `artifacts/muninn/c1/attestation.jsonl`. Esos archivos no estaban disponibles.

No se invocó `gh attestation verify`, porque no hay sujetos locales verificables
y la autorización prohíbe red. Una atestación de otro binario no autenticaría el
binario recién compilado.

## Entradas preservadas

- `tmp/muninn/h5-singletons.json`: 1693 bytes;
  SHA-256 `be3c08be15be420b20f5a09144eb149ec09859bd2535d22234d78db38abadbc8`.
- `tmp/muninn/h5-singletons-manifest.json`: 3316 bytes;
  SHA-256 `ef7847856ac9b1d0e9990606055d401f9f8dc6e4f4d6f5ed17053691d2df09ea`.
- `tmp/muninn/retrieval-h5-20261107b.json`: 5173 bytes;
  SHA-256 `674ff3518d7865d684e568419ec22f4f6cf7494bf5ae994f25f32d5bedac78da`.

Ninguno de estos archivos se sobrescribió. No se modificaron modelo, encoder,
ranking, umbrales, corpus ni juicios. La existencia del entorno Python offline
no resuelve la procedencia Rust. No se ejecutó el exporter Python de forma
parcial, porque no permitiría completar la comparación requerida.

## Verificación focalizada

Con `PYTHONPATH=dev/muninn`, se ejecutó `python3 -m unittest discover` con
`-s dev/muninn/tests` y cada patrón siguiente:

- `test_retrieval*.py`: 42 pruebas, OK.
- `test_c1_runner.py`: 24 pruebas, OK.
- `test_c1_attempt_run.py`: 5 pruebas, OK.
- `test_c1_provenance.py`: 5 pruebas, OK.

Las pruebas de procedencia usan dobles sintéticos; no autentican este build.
No hubo cambios de código ejecutable, por lo que no corresponde un ciclo TDD
nuevo. No se ejecutó CI completo, builds del workspace, servicios, descargas,
llamadas remotas ni operaciones sobre bases de datos.

## Siguiente decisión necesaria

Para continuar con el contrato C1 actual se necesitan un binario y un registro de
build con sus atestaciones válidas y los materiales exactos. También debe
comprobarse si el verificador puede operar sin red con los recursos disponibles.
Alternativamente, el usuario debe aprobar y revisar un contrato **separado** para
diagnósticos locales no autenticados, con estado explícito y sin equivalencia a
aceptación C1. Esta unidad no autoriza ni implementa esa alternativa.

La captura de vectores del servicio histórico y la escala de sus scores siguen
sin verificar. Una comparación futura de exports no resolverá esas limitaciones
por sí sola. H6/H7 y la paridad integral permanecen fuera del resultado.
