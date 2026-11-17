# Exportador privado C0

Interfaz: `export_minilm_c0 ROOT BUNDLE INPUT.json`. `ROOT` debe ser una ruta
absoluta. `BUNDLE` debe ser el identificador SHA-256 hexadecimal minúsculo de
64 caracteres de un bundle existente. El ejemplo abre y verifica ese bundle;
no prepara, descarga ni recupera artefactos.

Entrada JSON, sin campos adicionales:

```json
{"corpus_id":"c0_example","batches":[{"id":"batch_1","texts":["", "Texto de ejemplo"]}]}
```

`corpus_id` debe comenzar por `c0`. Los identificadores admiten entre 1 y 64
bytes ASCII alfanuméricos, guion y guion bajo. Los identificadores de lote deben
ser únicos. Se admiten entre 1 y 32 lotes y entre 1 y 32 textos por lote. Cada
texto admite hasta 4096 bytes UTF-8. El archivo admite hasta 256 KiB, incluidos
espacios. La lectura usa `take(límite + 1)` y rechaza el exceso antes de abrir
la caché. El hash SHA-256 corresponde a los bytes exactos del archivo.

La salida contiene un único JSON con `schema_version: 1`, `corpus_id`,
`input_sha256`, `artifact_bundle_id`, `execution: "candidate_cpu_f32"`,
`token_source` y `batches`. Cada lote conserva su identificador y contiene
`input_ids`, `attention_mask`, `token_type_ids` y `vectors`: un vector de 384
valores f32 finitos por texto, en el mismo orden. Los tokens se **recalculan**
con un `MiniLmTokenizer` independiente desde la misma caché verificada; no son
una captura del tokenizer interno del encoder. El encoder ejecuta su propia
tokenización y cálculo sin modificar sus operaciones.

El exportador serializa todo después de procesar todos los lotes. Los errores
se notifican en stderr mediante categorías fijas, sin rutas ni payloads. Un
fallo de escritura del sistema operativo puede interrumpir stdout: el consumidor
debe exigir salida JSON completa y código de salida cero. No se incluyen textos
en la salida, pero los tokens pueden revelar contenido: tratar el JSON como
dato sensible.

Este ejemplo no materializa un corpus aprobado, no establece tolerancias y no
emite un veredicto numérico ni aceptación. Sus pruebas usan datos sintéticos
solo para validar el contrato de entrada/salida y no cargan pesos ni cachés.
