# Archive — huginn-overlay-system

## Resultado y destino

- Fecha: 2026-10-20. Persistencia: OpenSpec en `.agents/sdd/` por ruta explícita.
- Destino: `.agents/sdd/changes/archive/2026-10-20-huginn-overlay-system/`.
- Cierre autorizado con advertencias; no acredita despliegue, aceptación humana,
  disponibilidad remota ni integración operativa externa.
- Sin colisión de destino ni de los tres dominios nuevos. El spec existente
  `ui-settings` se leyó y fusionó; no se sobrescribió como un dominio nuevo.
- Estado preparado antes del movimiento: `current_phase: archived`, `archive`
  añadido a `completed`, `next: none` y `archived_to` con el destino anterior.
- Ejecución fallback: no se afirma enforcement mediante runner/FSM determinista.
  Skill resolution: `none`; no existe registro local de skills.

## Gate comprobado antes de sincronizar

Se leyeron completos proposal, design, tasks, cuatro specs, base `ui-settings`,
state, configuración, verify-report y qa-report. No hay reglas locales de Archive
ni política P2/P3 más restrictiva. Apply tiene 21/21 tareas y 3/3 subpasos de 2.2.

- Verify: **PASS WITH WARNINGS**. V-01/V-02 resueltos; la reproducción independiente
  original pasa después del fix de SET-05. El FAIL inicial se conserva como historia.
- QA propio: **PASS WITH WARNINGS**, gate **AUTORIZADO CON ADVERTENCIAS**.
  No hay CRITICAL/P0/P1 pendientes ni fallos funcionales abiertos identificados.
- QA ejecutó 263 casos Cargo, incluidos 39 PTY, y 10 comprobaciones exploratorias
  adicionales; no se presentan como ejecución de Archive ni como cobertura exhaustiva.
- CI final corregida de Apply: 9/9, exit 0, 477 s, Vitest 210 y E2E 90.
  Se comprobó `huginn-fix-v01/ci-summary.txt` y se conserva su procedencia.
  Archive no repite CI amplia para estas operaciones exclusivamente documentales.

Los NOT TESTED operativos no son escenarios contractuales pendientes: OVL-05-A
exige un servicio controlado y OVL-05-B un fallo controlado de almacenamiento,
ambos ejecutados por QA. No se usa una excepción de documentación/configuración
para aceptar el producto. Las frases históricas de tasks/Verify que entregaban a
la fase siguiente se conservan; el informe QA posterior autoriza este gate.

## Specs sincronizados

- `.agents/sdd/specs/huginn-overlays/spec.md`: creado, 5 requisitos añadidos.
- `.agents/sdd/specs/huginn-ui-events/spec.md`: creado, 4 requisitos añadidos.
- `.agents/sdd/specs/huginn-status-bar/spec.md`: creado, 3 requisitos añadidos.
- `.agents/sdd/specs/ui-settings/spec.md`: REQ-07, REQ-08 y REQ-12 sustituidos
  exactamente por sus bloques delta; 0 añadidos y 0 eliminados.

Se verificó igualdad de bytes de las tres copias nuevas. Se compararon los 16
bloques de `ui-settings`: los 13 no mencionados y todo el contenido exterior a
requisitos permanecen idénticos a la base leída, contrastada con HEAD. No se
normalizan cabeceras históricas, checklists ni referencias de targets anteriores.
La referencia heredada a `--test settings` inexistente sigue documentada en Verify;
la evidencia vigente usa los targets reales lib/state/PTY, no ese comando.

## Contenido y evidencia preservados

La carpeta completa conserva proposal, exploration, design, tasks, specs,
apply-progress, apply-validation, verify-report, qa-report y state, además de este
informe. Los informes de aceptación y los deltas no se editan durante Archive.
No se modifica ningún archivo después de moverlo al archivo histórico.

Los logs y las capturas externos conservan sus rutas originales, no se borran ni
se trasladan. Prefijo de evidencia:
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/`.

- `huginn-fix-v01/`: CI final, regresiones y repetición focal de Apply.
- `huginn-verify/`: Verify inicial, prueba independiente y evidencia RED histórica.
- `huginn-reverify-v01/`: prueba original recompilada, presentation y estáticos.
- `huginn-qa/`: logs propios, reproducción, snapshots, PNG y asciicast; también
  envelopes fallidos/interrumpidos y `recording.json` con error de WebM.
- Las demás rutas históricas de Apply quedan referenciadas sin alteración en sus
  informes. El histórico no se presenta como evidencia del candidato corregido.
- `huginn-archive/before-move.json`: hashes SHA-256 de los 13 archivos anteriores
  a este informe, para comprobar integridad tras el movimiento.

La evidencia externa sigue en almacenamiento temporal: conservar esas rutas o
respaldarlas antes de limpiar temporales. Este archivo no incorpora sus binarios.

## Validación documental e integridad

- Comprobación estructural: PASS, 21/21 tareas, tres sustituciones exactas,
  trece requisitos no modificados y tres copias nuevas exactas.
- Markdownlint de los cuatro deltas y ambos informes Verify/QA: PASS, 6 archivos.
- Markdownlint inicial de los cuatro specs principales: exit 1 por MD038 heredado
  en `ui-settings`. Se corrigió la representación del literal con comillas y una
  aclaración: la entrada conserva exactamente los tres espacios y no incluye comillas.
  No cambia el requisito conductual.
- Markdownlint final de los cuatro specs principales y todos los Markdown del
  cambio archivado: PASS, 17 archivos, cero incidencias. `git diff --check`: PASS.
- Staging SHA-256 esperado:
  `180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`.
  Se comprueba antes y después. Sin add/stage/unstage/reset/checkout/commit/push,
  merge/PR, reset de datos ni cambios productivos.

## Advertencias obligatorias

- Proveedor externo, almacén seguro real del SO y dos opt-in heredados: NOT TESTED;
  QA-03 P2 operativo. No se convierten en PASS externo ni producción validada.
- QA-01 P3: exportación WebM FAIL del instrumento; PNG/asciicast válidos no son vídeo.
- QA-02 P3: arranque agent-tty requiere la respuesta DSR explícita documentada.
- Causa del fallo intermedio del helper PTY no establecida; el caso íntegro pasó
  después en los gates finales, reverify y los 39 PTY propios de QA.
- Sin cobertura instrumental, lector de pantalla ni locales completos validados.
- La evidencia externa permanece en almacenamiento temporal y requiere respaldo
  antes de limpiar esos directorios. El MD038 heredado quedó resuelto.

## Siguiente paso

Ninguna fase SDD pendiente. El ciclo queda cerrado con advertencias. Cualquier
extensión a integración externa requiere credenciales de prueba y autorización
explícita; Archive no autoriza publicación, merge ni despliegue.
