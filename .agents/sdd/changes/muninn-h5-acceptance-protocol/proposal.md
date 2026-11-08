# Propuesta: protocolo de aceptación H5

## Intención

Definir un contrato de aceptación de producto para H5 que permita decidir por separado si la búsqueda ANN reproduce la búsqueda exhaustiva y si la recuperación alcanza la calidad absoluta requerida según juicios humanos. El usuario aprobó la intención de mantener dos gates separados. La evidencia H5 disponible es descriptiva y exploratoria; todavía no existe un protocolo concreto de corrida con todos sus valores aprobado.

La corrida existente no se considerará preinscrita ni aceptada retroactivamente. La re-medición se hará únicamente después de que el usuario apruebe el protocolo, sus valores y la procedencia exigida.

## Alcance

### Incluido

- Proponer dos gates independientes: equivalencia ANN–exact y calidad absoluta de recuperación etiquetada.
- Dejar abiertos todos los valores estadísticos y específicos de cada corrida: métricas y K, reglas de aceptación y tolerancias, número y tratamiento de corridas, agregación, corpus y adjudicación, procedencia y unidad de comparación ANN–exact.
- Mantener la corrida disponible como evidencia descriptiva, con sus límites de corpus, anotación y procedencia; no asignarle un veredicto PASS.
- Permitir que la arquitectura y la especificación definan los campos obligatorios y el protocolo de aprobación, sin elegir valores para esos campos.
- Bloquear cada protocolo concreto de corrida y toda medición confirmatoria hasta que se completen y aprueben explícitamente todos los campos aplicables.

### Excluido

- Elegir por cuenta propia métricas gobernantes, K, umbrales, tolerancias, cantidad de corridas, corpus formal o proceso de adjudicación.
- Tratar la tolerancia H4 `1e-9` como criterio H5, o inferir un umbral de la diferencia de score observada o del orden coincidente en la corrida disponible.
- Presentar la especificación de paridad externa marcada **Proposed** como aprobación del contrato H5.
- Reetiquetar la corrida existente como preinscrita, aceptada o prueba de calidad general.
- Ejecutar una re-medición, cambiar el ranking, los filtros, el scope, el threshold o el fallback para ajustarse a un criterio no aprobado.
- Cambiar código, informes de resultados o el estado del cambio durante esta fase.

## Enfoque

La propuesta conserva los dos gates aprobados como preguntas independientes, con resultados y criterios de aceptación separados:

1. **Equivalencia ANN–exact:** determinar si la salida ANN concuerda con la búsqueda exhaustiva bajo datos, consulta, scope, encoder y configuración fijados. El contrato deberá decidir si compara las salidas directas o el comportamiento público completo, incluidos filtros y fallback por underfill.
2. **Calidad absoluta de recuperación:** determinar si los candidatos ordenados satisfacen el objetivo de relevancia establecido con juicios humanos. La coincidencia ANN–exact no demuestra relevancia; una métrica de relevancia tampoco demuestra equivalencia entre algoritmos.

No se combinan ambos gates en un único resultado agregado. La arquitectura y la especificación pueden formalizar qué campos debe contener cada protocolo concreto de corrida y cómo se bloquea una ejecución incompleta, pero no eligen los valores de esos campos ni constituyen su aprobación. Los valores estadísticos y operativos permanecen abiertos. Antes de iniciar cada corrida confirmatoria, el usuario o responsable autorizado debe aprobar explícitamente todos los campos aplicables y la versión completa de ese protocolo. Si falta cualquier campo o aprobación, esa corrida y cualquier medición confirmatoria quedan bloqueadas. La evidencia actual seguirá identificada como medición descriptiva, no aceptada y no preinscrita; no se permite preinscribirla retroactivamente. La referencia externa de paridad continúa siendo **Proposed** y no sustituye la aprobación del protocolo de corrida.

## Campos abiertos que requieren aprobación para cada protocolo de corrida

1. **Métrica y K:** definir qué métrica o métricas gobiernan cada gate y los valores de K aplicables. Recall, MRR, nDCG y los K del piloto son evidencia descriptiva existente, no valores de aceptación aprobados.
2. **Reglas de aceptación y tolerancias:** definir la condición de aceptación de cada gate, incluidas igualdad de ranking, comparación numérica de scores, márgenes u otra regla. No se asignan valores predeterminados.
3. **Corridas y agregación:** definir el número de ejecuciones, el tratamiento de variabilidad y datos indefinidos, y la agregación entre consultas y ejecuciones.
4. **Corpus y adjudicación:** definir si el piloto es suficiente para la corrida prevista; establecer requisitos de tamaño y representatividad, número de revisores, adjudicación, consultas sin candidatos relevantes y versionado/cambio de etiquetas.
5. **Procedencia:** definir qué identidad y versiones se deben fijar y verificar para encoder/modelo, código/build/bridge, dependencias, corpus/etiquetas, configuración de búsqueda y entorno de ejecución.
6. **Alcance de ANN–exact:** definir si se comparan salidas directas o el comportamiento completo de producción, incluidos underfill/fallback, filtros, límites, threshold, empates y scope.
7. **Campos específicos de cada corrida:** completar y aprobar todas las decisiones aplicables para la versión concreta del protocolo, incluidos objetivo, datos y corpus identificados, ruta bajo prueba y evidencia de procedencia. Ningún campo estadístico u operativo queda aprobado por el mero hecho de figurar como requisito en la arquitectura o especificación.

La intención de dos gates ya está aprobada y no se vuelve a abrir como decisión de alcance. La arquitectura y la especificación pueden describir estos campos sin elegir sus valores. La aprobación debe resolver todos los campos aplicables antes de iniciar cada protocolo concreto de corrida; mientras falte un campo o su aprobación explícita, no se iniciará esa corrida ni ninguna medición confirmatoria.

## Áreas afectadas previstas

| Área | Impacto previsto | Descripción |
|---|---|---|
| `.agents/sdd/changes/muninn-h5-acceptance-protocol/` | Nuevo artefacto | Esta propuesta documenta la intención, el alcance y las decisiones pendientes. |
| `docs/muninn/retrieval-h5-local-results.md` | Referencia; sin cambio en esta fase | Conservar el estado descriptivo y las limitaciones de la corrida existente. |
| `docs/muninn/retrieval-judgment-pilot-v1.md` y `dev/muninn/fixtures/retrieval-pilot-v1-corpus.json` | Revisión posterior condicionada | Evaluar adecuación del corpus y la rúbrica cuando se decida el protocolo. |
| `dev/muninn/retrieval_quality.py` y `dev/muninn/retrieval_h5_runner.py` | Posible cambio posterior | Solo tras aprobar las reglas y requisitos de procedencia. |
| `crates/muninn/neo4j/src/queries/short_term.rs` y `crates/muninn/neo4j/src/short_term.rs` | Referencia técnica; cambio no autorizado aquí | La especificación posterior precisará qué rutas y comportamientos cubre el gate ANN–exact. |
| `tmp/plans/muninn-feature-parity-spec.md` | Referencia externa no normativa | Su estado **Proposed** no representa aprobación de producto para H5. |

## Riesgos

| Riesgo | Mitigación propuesta |
|---|---|
| Confundir equivalencia algorítmica con calidad semántica. | Mantener gates, criterios y resultados separados. |
| Generalizar a partir del piloto pequeño o de juicios de un solo revisor sin adjudicación. | Solicitar aprobación explícita del corpus y del proceso de anotación antes de aceptarlos como evidencia formal. |
| Convertir resultados ya observados en aceptación retrospectiva. | Preservar el estado descriptivo no preinscrito y exigir aprobación previa a cualquier re-medición. |
| Importar un umbral no autorizado, incluida la tolerancia H4 `1e-9`. | No fijar valores en esta propuesta; decidir cada regla H5 de forma explícita. |
| Aceptar resultados con identidad de build, corpus o configuración no verificada. | Definir los requisitos de procedencia en la especificación y aprobar/verificar los valores correspondientes antes de cada corrida confirmatoria. |
| Alterar el comportamiento de búsqueda para satisfacer un gate. | Mantener ranking, filtros, scope, threshold y fallback fuera de cambios motivados por criterios aún no aprobados. |

## Reversión

Esta fase solo crea una propuesta documental. Si el usuario rechaza o cambia la propuesta, actualizar o retirar esta propuesta mediante una decisión explícita antes de iniciar la especificación. No hay cambios de código, datos, informes de medición ni estado que revertir en esta fase.

## Criterios de éxito

- [ ] La propuesta distingue formalmente el gate de equivalencia ANN–exact del gate de calidad absoluta etiquetada.
- [ ] La intención de dos gates permanece como alcance aprobado; arquitectura y especificación pueden formalizar campos requeridos sin escoger sus valores.
- [ ] Todos los valores estadísticos y específicos de corrida permanecen abiertos hasta la aprobación explícita de cada protocolo concreto.
- [ ] Cada corrida confirmatoria queda bloqueada hasta que se hayan completado y aprobado todos los campos aplicables.
- [ ] La corrida actual se conserva como descriptiva, no aceptada y no preinscrita; la propuesta no le atribuye un veredicto de aceptación.
- [ ] No se deriva ningún umbral H5 de H4, de resultados observados ni de la especificación externa **Proposed**.
- [ ] No se preinscribe ni acepta retroactivamente la corrida exploratoria existente.
