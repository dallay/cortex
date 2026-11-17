# C1 v1 — Borrador de aceptación, no congelado

Corpus: `minilm-c1-v1-draft.json`. Etiquetas: `minilm-c1-v1-draft-labels.json`.

Contiene 13 lotes, 45 posiciones y 6 triplets propuestos. Incluye español,
inglés, negativos léxicos, Unicode, vacío, Rust/Cypher, decisiones técnicas,
candidatos de fronteras y variaciones de batch/prefijo/cola.

El usuario autorizó el 2026-10-30 compartir exclusivamente `""` con C0 como
control de entrada vacía. La separación del resto se comprueba por igualdad
exacta y NFC. No se modifica C0.

Las etiquetas son hipótesis generadas por agente, no anotaciones humanas ni
aceptación de calidad. Las repeticiones de `planet` son candidatos: no se ha
verificado que produzcan 127/128/129/255/256/257 tokens. No se ha ejecutado
inferencia, tokenizer ni aceptación al preparar este borrador.

## Pendientes antes de congelar

- Revisar paráfrasis sin palabras compartidas y suficiencia de triplets.
- Verificar longitudes reales y truncamiento con el tokenizer fijado.
- Completar configuración runtime y vincular identidad de espacio/modelo.
- Completar validación integral del sidecar y casos negativos de sus campos.
- Definir política numérica justificada y obtener revisión independiente.
- Congelar hashes solo después de esas revisiones, antes de ejecutar C1.

## Comprobación offline

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover \
  -s dev/muninn/tests -p test_c1_corpus_draft.py -v
PYTHONDONTWRITEBYTECODE=1 python3 dev/muninn/c1_corpus_draft.py
```

Los hashes del informe identifican los bytes parseados desde los fixtures.
Para objetos suministrados en memoria se informa `null`, no identidad ajena.
`acceptance_ready` permanece `false` en todos los casos.
