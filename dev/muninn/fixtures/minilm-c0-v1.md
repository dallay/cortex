# Corpus C0 v1: preparación estática

## Alcance y congelación

Corpus de calibración preparado antes de medir, según §7 del diseño
`tmp/plans/2026-10-28-muninn-minilm-design.md`. No se ejecutaron modelos ni
el tokenizer. No se consultaron cachés ni red. Esta entrega no establece
resultados numéricos, equivalencia de implementaciones ni calidad en español.

Los juicios del companion son hipótesis de relevancia semántica preparadas por el agente; no son anotaciones humanas independientes.
«Positivo» significa respuesta pertinente según la intención del texto;
«negativo» significa distractor deliberado. No significa que un modelo deba
producir una puntuación específica ni que vaya a distinguir esas entradas.
Las paráfrasis buscan poca o ninguna coincidencia de términos de contenido;
no se ha medido su coincidencia léxica.

Conservar los bytes de `c0-v1` después de observar salidas. No ajustar textos,
etiquetas, orden ni composición para mejorar resultados. Una corrección exige
otra versión, nuevos hashes y motivo registrado; conservar esta versión y
mantener separados sus resultados. Los hashes siguientes quedan fijados antes
de cualquier medición de este corpus.

## Archivos y SHA-256 de bytes exactos

- `minilm-c0-v1.json`:
  `3a9d4a2fbc1d197d319ea538d51466b3837185e80c8d54e8fafd3081dcbe8d11`
- `minilm-c0-v1-labels.json`:
  `8fa061bb6f4084ad975467e3e82d356a8ae2c3c5b4ac1238a1a59dea06569af1`
- `minilm-c0-v1-runtime.json`:
  `a1b1da55558a0e1df0aa0bd48f57002b010ca65d076dd5a4ee2ca9a5d449ba4e`

JSON UTF-8, con salto de línea final. El corpus usa exclusivamente `corpus_id`
y `batches`, con `id` y `texts` por lote, conforme al exportador privado.
Las etiquetas no se pasan como entrada al exportador. Los IDs lógicos son
`batch_id/índice`, con índice desde cero.

## Referencias y cobertura

- `semantic_es/0..8`: durabilidad, separación de organizaciones y revocación.
  Tres grupos consulta/positivo/negativo; los distractores conservan vocabulario
  pero invierten la decisión.
- `semantic_en/0..8`: caché, recuperación de despliegues y memoria acotada.
  Tres grupos con paráfrasis y distractores léxicos.
- `technical/0..11`: cuatro grupos sobre `Result` frente a `unwrap` en Rust,
  filtro de tenant en Cypher, escritor único SQLite y puertos/adaptadores.
  Los fragmentos son datos, no código ejecutado.
- `unicode/0..4`: acentos NFC/NFD, paráfrasis, negación, caracteres no ASCII
  y emoji. `unicode/0` y `unicode/1` difieren en bytes, pero coinciden en NFC.
  No se presupone que todos los scripts tengan cobertura semántica útil.
- `empty/0..2`: cadena vacía, espacio y saltos/tabulación.
- `lengths/0..5`: respectivamente 125, 126, 127, 253, 254 y 255 repeticiones
  de `hello`, separadas por un espacio. Hipótesis a verificar: un token por
  palabra más dos especiales produce longitudes codificadas **antes del
  truncamiento** 127, 128, 129, 255, 256 y 257. No son conteos observados.
  La configuración prevista limita la secuencia a 256; 257 no es una longitud
  de salida afirmada.
- `prefix_tail/0..1`: prefijo idéntico de 255 `hello`, seguido por decisiones
  opuestas. La señal al final puede perderse por truncamiento.
- `prefix_tail/2..3`: decisiones opuestas al inicio, seguidas del mismo relleno.
  `prefix_tail/4..5` contienen únicamente las decisiones, para relacionarlas
  con sus versiones largas. Las etiquetas expresan intención del texto completo,
  no una afirmación de que el encoder conserve su cola.
- `single_es/0`: la consulta `semantic_es/0` aislada.
- `mixed/0..3`: esa consulta junto a `lengths/5`, `empty/0` y `unicode/1`.
- `mixed_reverse/0..3`: exactamente los mismos textos en orden inverso.
- `repeat/0..4`: consulta, texto largo, consulta repetida, vacío y texto largo
  repetido. `equal_text_groups` identifica coincidencias exactas entre lotes.
  Estos casos preparan el estudio de composición, orden y repetición, pero
  no demuestran invariancia.

## Configuración prevista, no recibo de ejecución

`minilm-c0-v1-runtime.json` registra identidad y revisión fijadas del modelo,
los once archivos del modelo y sus hashes, e ID de bundle calculado mediante
el recibo canónico de `artifact_model_stage.rs::receipt_bytes`. El cálculo usa
solo `artifact-lock.json`; no prueba que ese bundle exista en una caché.

Registra CPU/F32 como configuración prevista, candidato Candle y referencias
Python SentenceTransformers/PyTorch y Transformers. Las versiones se derivan
de `rust-toolchain.toml`, `bridge/Cargo.toml` y
`requirements-minilm-cpu.txt`. Incluye hashes de esos archivos y fragmentos
fuente verificables. No acredita versiones instaladas ni el runtime usado.
`actual_runtime` permanece `null`. No es un manifiesto consumido por el encoder
ni evidencia de ejecución; un recibo futuro debe registrar el runtime real.

## Validación local sin inferencia

Desde la raíz:

```sh
python3 -m unittest discover -s dev/muninn -p test_minilm_c0_fixture.py
```

La prueba usa solo biblioteca estándar. Comprueba esquema, referencias,
cobertura declarada, composiciones de lote, caracteres Unicode, número textual
de repeticiones, límites UTF-8, hashes congelados, derivación estática del bundle
y procedencia de versiones. No importa librerías de ML ni carga artefactos.

Inventario estático: 11 lotes, 64 posiciones de texto, máximo 1559 bytes UTF-8
por texto y 23149 bytes de corpus. Todos los lotes quedan dentro de 32 textos;
el corpus queda dentro de 32 lotes y 256 KiB. No se ejecutó el exportador Rust,
porque esa ejecución cargaría el modelo. La compatibilidad comprobada aquí es
la del esquema y los límites de entrada, no una ejecución de inferencia.
