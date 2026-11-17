## Exploración: protocolo de aceptación H5

### Estado actual

H5 tiene evidencia descriptiva local, pero no un contrato de aceptación aprobado. El cambio SDD debe proponer ese contrato para decisión de producto; no debe afirmar aceptación ni tratar la corrida ya realizada como preinscrita. La re-medición, si el usuario aprueba el protocolo y sus valores, es una tarea posterior.

**Acordado para la evaluación descriptiva existente:** corpus piloto de seis consultas y doce ocurrencias query-candidato; grados humanos 0–3, con grado ≥2 relevante para Recall/MRR; Recall, MRR y nDCG en K={1,3,5}; resultados por consulta y macro; reporte descriptivo sin umbral PASS; una corrida local. El corpus y rúbrica definen un piloto exploratorio; hay un solo revisor y no hay adjudicación. La implementación también deja nDCG indefinido/excluido del macro cuando el DCG ideal es cero. Evidencia: [RPI H5, líneas 17–22](../../../rpi/plan/tasks/muninn-h5-retrieval-context.md#L17-L22), [rúbrica, líneas 5–22, 38–40, 55–59](../../../../docs/muninn/retrieval-judgment-pilot-v1.md#L5-L22), [métricas, líneas 287–320 y 380–423](../../../../dev/muninn/retrieval_quality.py#L287-L320).

**Evidencia observada, no aceptación:** el informe del 8 de noviembre registra órdenes ANN/exhaustive iguales en 6/6 consultas, diferencia máxima absoluta de score 0.003067493438720703 y métricas descriptivas Recall@1/3/5=0/1/1, MRR=0.5 y nDCG@1/3/5≈0.02381/0.64408/0.64408. Registra `measured_not_accepted`, juicios no adjudicados y ninguna tolerancia aprobada. La identidad del bridge/build fue suministrada por el caller, no verificada por el runner. No reinterpretar esta corrida retrospectivamente como preinscrita. Evidencia: [resultados H5, líneas 134–162](../../../../docs/muninn/retrieval-h5-local-results.md#L134-L162), [guía del runner, líneas 63–69](../../../../docs/muninn/retrieval-h5-runner.md#L63-L69).

La especificación de paridad externa `tmp/plans/muninn-feature-parity-spec.md` permanece **Proposed**. Su §10.3 exige corpus con etiquetas versionadas y aprobación previa de tolerancias estadísticas y número de corridas; §13 G5 pide GM-07 verde con tolerancias de ranking documentadas. No constituye por sí sola una decisión aprobada ni fija umbrales H5. Evidencia: [especificación propuesta, líneas 534–541 y 695–705](../../../../../../cortex/tmp/plans/muninn-feature-parity-spec.md#L534-L541); la ruta está fuera de este repositorio/worktree y puede no resolverse como enlace relativo.

La tolerancia H4 `1e-9` **no es un umbral H5**. H4 la declara para comparaciones numéricas concretas de su differential GM-07; su orden de ranking se compara exactamente. H5 usa otra corrida, un piloto de relevancia y comparadores ANN frente a exhaustive; su máximo score observado (~0.00307) no cambia ni invalida aquella regla, ni autoriza a copiarla. Evidencia: [H4, líneas 216–226](../../../../docs/muninn/h4-measurement.md#L216-L226), [contrato H4, líneas 37–40 y 58–63](../../../../dev/muninn/gm_h4_contract.py#L37-L63), [resultado H5, líneas 139–141](../../../../docs/muninn/retrieval-h5-local-results.md#L139-L141).

La implementación compara dos preguntas distintas que el contrato debe mantener separadas:

1. **Equivalencia ANN–exact:** ¿el ranking ANN coincide con la búsqueda exhaustiva para los mismos datos, consulta, scope, encoder y configuración? En producción, ANN pide top-K global y después filtra owner/sesión; exhaustive filtra scope antes de puntuar. Ante underfill, el fallback exact reemplaza la lista ANN; no combina ambas listas. El diagnóstico H5 llama ambos caminos directamente con la misma consulta/configuración, por lo que su comparación no cubre todos los efectos del fallback normal. Evidencia: [queries ANN y exact, líneas 134–174 y 213–265](../../../../crates/muninn/neo4j/src/queries/short_term.rs#L134-L174), [fallback, líneas 756–768](../../../../crates/muninn/neo4j/src/short_term.rs#L756-L768), [ruta diagnóstica](../../../../docs/muninn/retrieval-h5-runner.md#L9-L18).
2. **Calidad absoluta de recuperación:** ¿el ranking recupera candidatos relevantes según juicios humanos? El piloto mide esa calidad descriptivamente, pero no acredita calidad de producción ni paridad estadística; una corrida de seis consultas y un revisor no representa por sí sola evidencia inferencial.

### Áreas afectadas

- `docs/muninn/retrieval-h5-local-results.md` — evidencia existente; conservar sus límites y estado, sin reescribir la corrida como preinscrita.
- `docs/muninn/retrieval-judgment-pilot-v1.md` y `dev/muninn/fixtures/retrieval-pilot-v1-corpus.json` — rúbrica y corpus piloto; falta decidir si son suficientes para el gate formal y cómo versionar/adjudicar futuros juicios.
- `dev/muninn/retrieval_quality.py` y `dev/muninn/retrieval_h5_runner.py` — reportan métricas y procedencia; actualmente producen salida descriptiva y `measured_not_accepted`, no un veredicto contra un umbral aprobado.
- `crates/muninn/neo4j/src/queries/short_term.rs` y `crates/muninn/neo4j/src/short_term.rs` — definen ANN, búsqueda exhaustive y fallback real; importa fijar la unidad bajo aceptación y sus invariantes.
- `.agents/rpi/plan/tasks/muninn-h5-retrieval-context.md` — registro de coordinación y límites ya aprobados; no sustituye el artefacto contractual SDD.
- `tmp/plans/muninn-feature-parity-spec.md` — referencia externa Proposed para §10.3 y G5; no es una especificación aprobada de este cambio.

### Decisiones abiertas para el protocolo

1. **Objetivo de producto:** aceptar calidad absoluta frente a etiquetas, equivalencia ANN–exact, o dos gates separados. La recomendación es separarlos: el éxito ANN–exact no prueba relevancia y una métrica relevante no prueba equivalencia de los algoritmos.
2. **Métrica principal y K:** ya existe un conjunto descriptivo Recall/MRR/nDCG en K={1,3,5}; no está decidido cuál, si alguna, gobierna el veredicto ni si se conserva ese K para aceptación.
3. **Regla de decisión:** tolerancia de scores, condición de igualdad de ranking, comparación entre métricas, margen admisible o regla alternativa. No hay valor aprobado; no inferirlo de `1e-9`, de la diferencia observada ni de los resultados.
4. **Corridas y agregación:** el piloto tiene una corrida y macro por consulta. No se ha aprobado cantidad de repeticiones, tratamiento de variabilidad, intervalos/contrastes ni agregación entre ejecuciones.
5. **Corpus y adjudicación:** si seis consultas/doce ocurrencias son solo piloto o el corpus de aceptación; tamaño/representatividad, revisores, adjudicación, tratamiento de consultas sin relevantes y proceso de cambio de etiquetas siguen por decidir.
6. **Identidad/procedencia requerida:** qué encoder y revisión deben fijarse y cómo verificar identidad de modelo, código/build/bridge, dependencias, configuración de búsqueda y máquina. La evidencia actual no autentica independientemente la identidad del build.
7. **Alcance de ANN–exact:** comparar salidas directas o el comportamiento público completo incluyendo underfill/fallback, filtros, límite, threshold, empates y scope. No alterar ranking/filtros para hacer pasar un gate.

### Enfoques

1. **Contrato de dos gates con aprobación antes de medir (recomendado)** — especificar por separado equivalencia ANN–exact y calidad con juicios; registrar decisiones aún abiertas y presentar valores candidatos al usuario sin asignar defaults. Aprobado el protocolo y sus identidades, otra tarea prepara corpus/adjudicación si hace falta y vuelve a medir.
   - Ventajas: evita confundir exactitud del índice con calidad semántica; mantiene la evidencia existente como diagnóstico; habilita gates con criterios distintos.
   - Costos: requiere decisiones de producto explícitas y posiblemente más anotación o instrumentación.
   - Esfuerzo: medio.
2. **Un único gate agregado de calidad/equivalencia** — combinar métricas de relevancia, igualdad de ranking y tolerancia numérica en un solo veredicto.
   - Ventajas: un resultado de aceptación simple.
   - Costos: mezcla propiedades independientes, dificulta interpretar fallos y puede ocultar calidad baja si ANN replica exactamente a exhaustive.
   - Esfuerzo: medio; no recomendado sin una justificación de producto.
3. **Adoptar los resultados ya medidos como aceptación** — no recomendado y no compatible con el requisito de preinscripción. La corrida precede a una regla aprobada; juicios/procedencia tienen límites conocidos.
   - Ventajas: ninguna más allá de evitar trabajo posterior.
   - Costos: aceptación retrospectiva, criterios no aprobados y riesgo de sobreafirmar evidencia de seis consultas.
   - Esfuerzo: bajo, pero inválido como protocolo verificable.

### Recomendación

Crear en la siguiente fase una **propuesta de protocolo para aprobación del usuario**, no una especificación con valores supuestos. La propuesta debe conservar como hechos acordados el corpus/juicios piloto y las métricas descriptivas existentes, marcar las preguntas abiertas de arriba, y distinguir formalmente el gate ANN–exact del gate de calidad absoluta. Debe declarar que la ejecución reportada es exploratoria y no preinscrita. No iniciar re-medición, no fabricar criterio PASS y no trasladar la epsilon H4. Tras el visto bueno explícito de objetivo, métricas/K, regla/tolerancias, corridas, corpus/adjudicación y procedencia, se podrá especificar la automatización y planificar una re-medición posterior.

### Riesgos

- Convertir datos piloto pequeños o juicios de un solo revisor en una afirmación de calidad general.
- Usar igualdad ANN–exact como sustituto de la relevancia del resultado o viceversa.
- Tratar la tolerancia H4, el score observado o el orden 6/6 como umbral de aceptación sin autorización.
- Aprobar retrospectivamente una corrida ya observada y perder la distinción entre diagnóstico y medición preinscrita.
- Medir sin fijar/validar provenance de encoder, build, corpus, anotaciones y configuración.
- Cambiar ranking, scope, threshold o fallback para adaptar la implementación a un umbral aún no acordado.

### Listo para propuesta

**Sí, con aprobación de producto requerida antes de cerrar el protocolo o volver a medir.** La próxima fase puede redactar una propuesta que plantee las decisiones pendientes y recomiende gates separados. No debe completar valores por defecto ni presentar la especificación externa Proposed como aprobada. Re-medición: tarea posterior al sign-off.
