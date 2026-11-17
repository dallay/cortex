# Especificación de aceptación de recuperación H5

## Propósito

Esta especificación define dos gates de aceptación independientes para H5: equivalencia ANN–exact y calidad absoluta de recuperación frente a juicios relevantes. Ambos gates están seleccionados para el futuro protocolo-run. Las métricas y fórmulas indicadas aquí están aprobadas como definiciones; esa aprobación no aprueba reglas de aceptación, umbrales, valores de K ni ningún otro valor de ejecución. No existe un protocolo-run aprobado y esta especificación no autoriza medición confirmatoria.

Los campos de ejecución que se indican como requeridos deben completarse y aprobarse para cada protocolo-run antes de medir. La corrida histórica permanece descriptiva, no preinscrita y no aceptada.

## Vigencia y precedencia de la enmienda ANN-only

La sección final «Enmienda: integración técnica offline ANN-only» modifica únicamente los requisitos que contradicen el slice autorizado. Prevalece sobre las afirmaciones anteriores que exigen ambos gates habilitados o dejan pendientes las reglas candidatas ya registradas. El resto del WIP y las definiciones de métricas se conservan. Las reglas candidatas no aprueban un corpus, un protocolo-run real ni una medición. La aceptación H5 continúa bloqueada.

## Requisitos

### Requisito: Mantener separados los gates H5

El protocolo-run futuro MUST incluir ambos gates. El sistema de evaluación MUST emitir un resultado independiente para equivalencia ANN–exact y calidad absoluta. Un resultado favorable en un gate MUST NOT implicar, sustituir ni mejorar el resultado del otro.

#### Escenario: Evaluación de ambos gates

- GIVEN un protocolo-run completado y aprobado que incluye ambos gates
- WHEN se evalúa una ejecución confirmatoria
- THEN el informe MUST presentar un resultado independiente para cada gate
- AND el informe MUST NOT derivar la aceptación de un gate del resultado del otro

#### Escenario: Se omite un gate

- GIVEN un protocolo-run propuesto que no incluye uno de los dos gates
- WHEN se valida el protocolo-run
- THEN el protocolo MUST quedar incompleto
- AND el informe MUST NOT declarar aceptación global de H5

### Requisito: Comparar las salidas directas ANN–exact

El gate ANN–exact MUST comparar las salidas directas de ANN y de la búsqueda exacta para la misma consulta y el mismo conjunto de candidatos, sin incluir filtros posteriores ni fallback de producción en esa comparación. El protocolo-run MUST identificar la consulta, el conjunto de candidatos, la configuración pertinente y los dos resultados comparados.

Para el valor simbólico K aprobado para el protocolo-run, `ExactTopKRecall@K` MUST calcularse como `|ANN_topK ∩ Exact_topK| / |Exact_topK|`. `OrderedTopKMatch@K` MUST ser 1 si las secuencias disponibles de IDs en top-K son idénticas, incluida su longitud, y 0 si difieren. Si una salida contiene menos de K elementos, el evaluador MUST usar los elementos disponibles; MUST NOT inventar ni completar candidatos faltantes. Estas métricas comparan IDs y ranking, no paridad de scores; el evaluador MUST NOT comparar scores ni aplicar una tolerancia de score.

Si `Exact_topK` está vacío, `ExactTopKRecall@K` y `OrderedTopKMatch@K` MUST ser indefinidas. Esta regla prevalece sobre la comparación de secuencias para `OrderedTopKMatch@K`. El tratamiento de métricas indefinidas para aceptación o agregación permanece pendiente de aprobación para cada protocolo-run; esta definición no establece una política predeterminada.

#### Escenario: Métricas ANN–exact sin paridad de scores

- GIVEN un protocolo-run aplica las definiciones aprobadas de métricas ANN–exact
- WHEN se evalúa la equivalencia
- THEN el evaluador MUST comparar IDs y orden según las definiciones aprobadas
- AND MUST NOT comparar scores ni exigir paridad de scores
- AND si una salida tiene menos de K resultados, el cálculo MUST usar solo los elementos disponibles, sin inventar candidatos faltantes

#### Escenario: Salida exacta vacía

- GIVEN `Exact_topK` está vacío
- WHEN se calculan `ExactTopKRecall@K` y `OrderedTopKMatch@K`
- THEN `ExactTopKRecall@K` MUST quedar indefinida por denominador vacío
- AND `OrderedTopKMatch@K` MUST quedar indefinida
- AND el evaluador MUST NOT sustituir esos valores ni inferir un resultado de aceptación o agregación

#### Escenario: Salida con menos de K elementos

- GIVEN ANN o exact devuelve menos de K elementos
- WHEN se calculan las métricas ANN–exact
- THEN el evaluador MUST usar únicamente los elementos disponibles en cada salida
- AND `OrderedTopKMatch@K` MUST comparar las secuencias disponibles y sus longitudes, de modo que longitudes distintas produzcan un valor de 0
- AND el evaluador MUST NOT inventar ni completar candidatos faltantes
- AND si `Exact_topK` está vacío, ambas métricas MUST quedar indefinidas
- AND el tratamiento de cualquier valor indefinido para aceptación o agregación MUST permanecer pendiente del protocolo-run

#### Escenario: Mismo conjunto, orden distinto

- GIVEN ANN y exact contienen el mismo conjunto de IDs en top-K, pero en distinto orden
- WHEN se calculan las métricas
- THEN `ExactTopKRecall@K` MUST reflejar coincidencia completa del conjunto
- AND `OrderedTopKMatch@K` MUST ser 0

#### Escenario: Scores distintos con IDs y orden coincidentes

- GIVEN las secuencias de IDs ANN y exact en top-K son idénticas, pero sus scores difieren
- WHEN se evalúa el gate ANN–exact
- THEN las métricas MUST depender solo de los IDs y el orden
- AND el evaluador MUST NOT comparar scores ni aplicar tolerancia de score

### Requisito: Calcular la calidad absoluta con métricas aprobadas

El gate de calidad absoluta MUST incluir las definiciones aprobadas de `Recall@K` y `nDCG@K`. Los valores de K para cada protocolo-run MUST registrarse como campos de ejecución y MUST recibir aprobación explícita antes de la medición; las definiciones de estas métricas no establecen reglas de aceptación ni valores de ejecución.

`Recall@K` MUST calcularse como `|relevantes recuperados top-K| / |relevantes juzgados|`, usando exclusivamente el conjunto relevante de la rúbrica aprobada para ese protocolo-run. MUST ser indefinida si no hay elementos relevantes juzgados.

`nDCG@K` MUST usar ganancia graduada `2^grade - 1` y descuento `log2(rank + 1)`. Su ranking ideal MUST basarse en los candidatos juzgados para la consulta. MUST ser indefinida si `IDCG=0`. La rúbrica, las etiquetas, sus grados y su adjudicación requieren aprobación futura; las etiquetas del piloto MUST NOT adoptarse automáticamente. El tratamiento agregado o de aceptación de métricas indefinidas permanece pendiente de aprobación para cada protocolo-run.

#### Escenario: Calidad con juicios graduados

- GIVEN una consulta con candidatos juzgados y grados definidos por la rúbrica aprobada para el protocolo-run
- WHEN se calcula `nDCG@K`
- THEN el evaluador MUST aplicar ganancia `2^grade - 1` y descuento `log2(rank + 1)`
- AND el ranking ideal MUST usar los candidatos juzgados

#### Escenario: No hay elementos relevantes juzgados

- GIVEN el conjunto de elementos relevantes juzgados para una consulta está vacío
- WHEN se calcula `Recall@K`
- THEN `Recall@K` MUST ser indefinida
- AND el evaluador MUST NOT inferir aceptación de la métrica

#### Escenario: IDCG igual a cero

- GIVEN el cálculo del ranking ideal produce `IDCG=0`
- WHEN se calcula `nDCG@K`
- THEN `nDCG@K` MUST ser indefinida
- AND el tratamiento de ese valor MUST permanecer sujeto a un campo de ejecución aprobado

#### Escenario: Etiquetas del piloto sin aprobación

- GIVEN existen etiquetas del piloto, pero no han sido aprobadas como rúbrica y etiquetas del protocolo-run
- WHEN se configura la evaluación de calidad absoluta
- THEN el sistema MUST NOT usar esas etiquetas automáticamente como referencia aprobada
- AND la procedencia, versión y rúbrica aprobadas MUST quedar identificadas como campos requeridos del protocolo-run

### Requisito: Mantener MRR como métrica descriptiva

`MRR` MUST permanecer descriptiva y MUST NOT ser una métrica gobernante de aceptación. Su presencia o definición en informes MUST NOT convertirla en criterio de aceptación ni aprobar la rúbrica del protocolo-run.

#### Escenario: Reporte descriptivo compatible

- GIVEN una consulta con resultados relevantes según la rúbrica usada para el reporte
- WHEN se genera el MRR descriptivo
- THEN MRR MUST identificarse únicamente como métrica descriptiva
- AND MRR MUST NOT gobernar el resultado del gate

### Requisito: Completar y aprobar los campos del protocolo-run

Antes de cualquier medición confirmatoria, el protocolo-run MUST registrar y obtener aprobación explícita para: objetivo; corpus y sus versiones; consultas y candidatos; valores de K para las métricas aprobadas de cada gate; regla de aceptación y tolerancias si aplican; número de corridas; agregación entre consultas y corridas; tratamiento de valores indefinidos; política de empates; alcance y configuración de cada ruta bajo prueba; y procedencia verificable de encoder/modelo, build/código/bridge, dependencias, datos, configuración y entorno. Las etiquetas, la rúbrica y la adjudicación MUST ser campos requeridos y aprobados cuando el gate de calidad absoluta esté habilitado; MUST NOT exigirse por el gate ANN–exact por sí solo. El alcance ANN–exact MUST ser requerido cuando ese gate esté habilitado. Los gates son independientes y ambos deben estar seleccionados.

Las definiciones de métricas aprobadas son `ExactTopKRecall@K`, `OrderedTopKMatch@K`, `Recall@K` y `nDCG@K`; `MRR` es descriptiva. Su aprobación MUST NOT interpretarse como aprobación de un criterio, umbral, tolerancia, valor de K, corpus, etiqueta, rúbrica, adjudicación, política de empates, número de corridas, agregación, tratamiento definitivo de indefinidos, aprobación, identidad de corrida o procedencia concreta. Todos los K MUST permanecer simbólicos hasta su aprobación como valores de ejecución. Ninguna regla de PASS se define aquí. Si falta un campo o su aprobación, el protocolo-run MUST permanecer bloqueado para medición confirmatoria y ningún gate MUST recibir PASS.

#### Escenario: Métricas aprobadas, protocolo-run aún incompleto

- GIVEN están aprobadas las definiciones de métricas de esta especificación
- AND falta cualquier valor, campo o aprobación requerida para el protocolo-run
- WHEN se solicita iniciar una medición confirmatoria
- THEN la medición MUST quedar bloqueada
- AND la aprobación de métricas MUST NOT tratarse como aprobación de criterios ni valores de ejecución

#### Escenario: Tratamiento de indefinidos y empates pendientes

- GIVEN el protocolo-run no especifica tratamiento agregado de valores indefinidos o política de empates
- WHEN se valida el protocolo-run
- THEN ambos asuntos MUST permanecer como campos pendientes del protocolo-run
- AND el sistema MUST NOT asumir una regla predeterminada de tratamiento o agregación

#### Escenario: Protocolo-run completo y aprobado

- GIVEN todos los campos requeridos están completos y aprobados antes de medir
- WHEN comienza una medición confirmatoria
- THEN el evaluador MUST vincular los resultados a la versión aprobada del protocolo-run
- AND MUST poder corroborar que el protocolo completo corresponde a su hash aprobado
- AND MUST evaluar ambos gates de forma independiente

### Requisito: Validar estructura, presencia y estado de cada campo del protocolo-run

La plantilla MUST describir una estructura y un tipo explícitos para cada campo del protocolo-run, con propiedades permitidas y aplicabilidad por gate. Como mínimo, objetivo MUST ser texto; corpus/versiones, configuración de cada ruta y procedencia MUST ser objetos con propiedades declaradas; consultas/candidatos y datos repetibles MUST ser listas de elementos con estructura declarada; y valores métricos, decisiones de aceptación, corridas, agregación, indefinidos y empates MUST tener estructuras declaradas que distingan `pending` de contenido presente. La especificación estructural MUST identificar cada campo por su ruta y MUST NOT fijar valores concretos ni reglas para esas decisiones.

La plantilla MUST declarar, como mínimo, el siguiente mapa de tipos y aplicabilidad; las propiedades internas de objetos y elementos MUST estar declaradas por el esquema, no aceptarse como JSON arbitrario:

| Campo                                        | Tipo estructural                                                                          | Aplicabilidad                     |
|----------------------------------------------|-------------------------------------------------------------------------------------------|-----------------------------------|
| Objetivo                                     | Texto no vacío                                                                            | Siempre                           |
| Corpus y versiones                           | Objeto no vacío                                                                           | Siempre                           |
| Consultas y candidatos                       | Lista no vacía de elementos estructurados                                                 | Siempre                           |
| Etiquetas, rúbrica y adjudicación            | Objeto(s) no vacío(s) con estructura declarada                                            | Solo con calidad absoluta         |
| K por métrica aprobada                       | Objeto no vacío con valores enteros cuando estén definidos y estado explícito de decisión | Para cada gate habilitado         |
| Regla de aceptación y tolerancias aplicables | Objeto no vacío con estado explícito de decisión                                          | Para cada gate habilitado         |
| Número de corridas                           | Objeto de decisión con valor entero cuando esté definido                                  | Siempre                           |
| Agregación entre consultas y corridas        | Objeto de decisión                                                                        | Siempre                           |
| Tratamiento de valores indefinidos           | Objeto de decisión                                                                        | Siempre                           |
| Política de empates                          | Objeto de decisión                                                                        | Siempre                           |
| Alcance y configuración de ruta              | Objeto no vacío con estructura declarada                                                  | Solo para la ruta/gate habilitado |
| Procedencia verificable                      | Objeto no vacío con componentes de procedencia estructurados                              | Siempre                           |

Cada lista MUST declarar el tipo y los campos permitidos de sus elementos. Cada objeto MUST declarar sus propiedades permitidas en todos los niveles. Los campos de decisión MUST distinguir estructuralmente `pending` de contenido presente, sin que contenido presente implique aprobación.

Los campos aplicables MUST estar presentes. En campos de decisión, un estado explícito `pending` indica que la decisión sigue sin resolver; contenido presente indica únicamente que hay contenido, no que esté aprobado. Ninguno de esos estados constituye aprobación. La plantilla MUST permanecer incompleta y MUST NOT ser confirmatory-ready. La validación MUST NOT añadir defaults, inferir valores, aceptar sustituciones del caller, decidir reglas de gate ni producir `PASS`. Solo la aprobación humana/producto y la corroboración de que el protocolo completo coincide con su versión/hash aprobado pueden satisfacer el gate existente de protocolo aprobado; la validación estructural no basta.

Etiquetas, rúbrica y adjudicación MUST aplicarse y ser obligatorias solo si está habilitado el gate de calidad absoluta. El alcance y la configuración ANN–exact MUST aplicarse y ser obligatorios solo si está habilitado el gate ANN–exact. La estructura MUST conservar esta condicionalidad y MUST NOT elevar campos condicionales a requisitos universales. La estructura MUST conservar ambos gates como decisiones independientes. `MRR` MUST seguir siendo descriptiva; los gates ANN–exact y de calidad absoluta MUST seguir siendo independientes, y `MRR` MUST NOT gobernarlos.

#### Escenario: Plantilla incompleta con decisiones pendientes

- GIVEN la plantilla contiene los campos estructurados requeridos y marca decisiones aún no resueltas como `pending`
- WHEN se valida la plantilla
- THEN la validación MUST identificar las rutas pendientes
- AND MUST bloquear `confirmatory_ready`
- AND MUST NOT declarar aprobación, aceptación ni `PASS`

#### Escenario: Campo requerido ausente o vacío

- GIVEN un campo aplicable falta, contiene un objeto vacío o contiene una lista vacía
- WHEN se valida el protocolo-run
- THEN el validador MUST informar la ruta precisa del campo
- AND MUST bloquear `confirmatory_ready`

#### Escenario: Tipo malformado, placeholder o propiedad desconocida

- GIVEN un campo tiene un tipo distinto al declarado, contiene una cadena placeholder o contiene una propiedad desconocida en cualquier nivel
- WHEN se valida el protocolo-run
- THEN el validador MUST informar la ruta precisa del campo o propiedad inválida
- AND MUST bloquear `confirmatory_ready`
- AND MUST NOT tratar el valor como presente ni inferir una sustitución

#### Escenario: Campo condicional no aplicable

- GIVEN el gate de calidad absoluta no está habilitado y sus etiquetas, rúbrica y adjudicación están ausentes
- AND el gate ANN–exact está habilitado con su alcance y configuración presentes
- WHEN se valida la estructura del protocolo-run
- THEN el validador MUST NOT requerir los campos exclusivos de calidad absoluta
- AND MUST validar los campos aplicables de ANN–exact

#### Escenario: Validación estructural no decide los gates

- GIVEN todos los campos tienen forma válida y contenido presente, pero no existe aprobación explícita del protocolo completo
- WHEN se valida la estructura o se solicita una decisión de gate
- THEN la validación MUST NOT declarar el protocolo aprobado ni confirmatory-ready
- AND MUST NOT producir `PASS` ni decidir reglas de aceptación
- AND el gate existente MUST permanecer bloqueado hasta corroborar la aprobación del protocolo completo y su hash

#### Escenario: Métricas y gates conservan su alcance

- GIVEN se validan los campos de ambos gates y se incluye MRR en un informe
- WHEN se valida el protocolo-run
- THEN ANN–exact y calidad absoluta MUST permanecer independientes
- AND MRR MUST permanecer descriptiva y MUST NOT gobernar aceptación
- AND la estructura MUST NOT definir umbrales, reglas `PASS`, parámetros de corridas/agregación, política de empates ni tratamiento final de indefinidos

### Requisito: Clasificar la procedencia de forma verificable

Cada gate MUST informar el estado de procedencia por separado. La procedencia MUST incluir evidencia verificable para la identidad y versión del encoder/modelo, build/código/bridge, corpus y etiquetas pertinentes, dependencias, configuración completa de recuperación y entorno de ejecución. Una declaración del caller sin evidencia independiente MUST NOT contar como verificación. La identidad devuelta por `/h5/diagnostic/search` es self-report del proceso: el validador MAY tratarla como evidencia observada del proceso únicamente si su forma es válida, corresponde a la respuesta de la misma consulta y coincide con la identidad esperada del protocolo-run y con la evidencia externa observada del build/container/run. Esa coincidencia permite correlacionar la declaración con la ejecución observada; MUST NOT presentarse como atestación criptográfica ni como prueba independiente del estado interno del proceso. La respuesta del bridge por sí sola MUST NOT satisfacer la procedencia requerida.

El estado MUST ser `verified` solo si toda la procedencia requerida está presente, corresponde a la ejecución y se valida independientemente. El estado MUST ser `partial` si existe evidencia verificable, pero falta o no se ha validado uno o más elementos requeridos. El estado MUST ser `insufficient` si no hay evidencia suficiente para identificar de forma confiable la ejecución o si la procedencia contradice la ejecución observada. Los estados `partial` e `insufficient` MUST impedir PASS. La procedencia `verified` no sustituye la aprobación previa del protocolo ni el cumplimiento del criterio.

#### Escenario: Procedencia completa

- GIVEN todas las identidades requeridas tienen evidencia independiente y corresponden a la ejecución
- WHEN se determina el estado de procedencia de un gate
- THEN el estado MUST ser `verified`
- AND la procedencia MUST NOT sustituir ningún otro requisito de aceptación

#### Escenario: Procedencia parcial o insuficiente

- GIVEN falta evidencia verificable, o la evidencia contradice la ejecución observada
- WHEN se determina el estado de procedencia de un gate
- THEN el estado MUST ser `partial` o `insufficient`, según la evidencia disponible
- AND el gate MUST NOT recibir PASS
- AND el informe MUST identificar los elementos faltantes, no verificados o contradictorios

### Requisito: Informar identidad allowlisted del encoder usado por la búsqueda H5

La ruta autenticada existente `/h5/diagnostic/search` MUST incluir en la misma respuesta que los resultados ANN/exact un único objeto de identidad allowlisted derivado del objeto encoder ya construido y realmente usado para esa consulta. El objeto MUST tener una forma cerrada con dos grupos separados: identidad declarada del bundle, manifiesto y space ID; e identidad loaded del perfil/configuración validado al cargar. MUST NOT admitir campos adicionales ni combinar esos grupos. MUST NOT representar un hash de archivos en disco como identidad de memoria. La ruta MUST conservar la feature gate `h5-diagnostic` y la autenticación actual. MUST NOT añadirse otra ruta ni modificarse el router normal, product retrieval o Store.

La respuesta MUST limitarse a campos allowlisted no sensibles. MUST NOT incluir secretos, el entorno completo, rutas locales, contenido de archivos, vectores ni payloads. Esta identidad es self-report dentro del trust boundary acordado de repo/operator/host/Docker, no atestación criptográfica y no resistente a un bridge, proceso, host o Docker comprometido.

#### Escenario: Identidad incluida en la respuesta autenticada de búsqueda

- GIVEN la feature `h5-diagnostic` está habilitada y una solicitud autenticada ejecuta `/h5/diagnostic/search`
- WHEN la ruta devuelve resultados ANN/exact para una consulta
- THEN la misma respuesta MUST incluir la identidad allowlisted del encoder usado para esa consulta
- AND la respuesta MUST conservar los resultados de búsqueda y la identidad juntos para correlación
- AND el endpoint MUST seguir sujeto a la autenticación actual

#### Escenario: Solicitud no autenticada

- GIVEN la feature `h5-diagnostic` está habilitada
- WHEN un caller sin autenticación válida solicita `/h5/diagnostic/search`
- THEN la ruta MUST rechazar la solicitud según el comportamiento de autenticación actual
- AND MUST NOT devolver resultados de búsqueda ni identidad del encoder

#### Escenario: Allowlist y ausencia de datos sensibles

- GIVEN una respuesta autenticada contiene identidad del encoder
- WHEN el validador inspecciona la respuesta completa
- THEN los campos de identidad MUST ajustarse estrictamente a la allowlist no sensible
- AND MUST distinguir bundle/manifiesto/space ID de configuración o perfil validado al cargar
- AND MUST rechazar campos adicionales o una forma que fusione ambos grupos de identidad
- AND MUST NOT exponer secretos, entorno completo, rutas locales, contenido de archivos, vectores ni payloads
- AND MUST NOT presentar un hash de archivos en disco como prueba de identidad en memoria

#### Escenario: Identidad procede del encoder realmente usado

- GIVEN una consulta produce resultados ANN/exact mediante un objeto encoder construido
- WHEN se forma la respuesta de `/h5/diagnostic/search`
- THEN la identidad reportada MUST derivarse de ese mismo objeto encoder usado por la consulta
- AND una identidad constante, de otro encoder o proporcionada por el caller MUST NOT sustituirla
- AND los resultados y la identidad MUST corresponder a la misma consulta y respuesta

#### Escenario: Self-report no constituye atestación

- GIVEN la respuesta contiene una identidad de encoder con forma válida
- WHEN se evalúa su fuerza como evidencia de procedencia
- THEN el informe MUST identificarla como self-report observado del proceso
- AND MUST NOT describirla como atestación criptográfica ni como resistente a compromiso del proceso, bridge, host o Docker
- AND la identidad MUST contar solo como evidencia correlacionada si coincide con la identidad esperada y con evidencia externa observada del build/container/run

### Requisito: Validar y correlacionar la identidad de proceso en el runner H5

El runner MUST validar estrictamente la forma de la identidad allowlisted y MUST correlacionarla con la consulta y respuesta que contienen los resultados ANN/exact, la inspección externa del Docker/container/run y la identidad esperada del protocolo-run. El runner MUST NOT aceptar claims del caller como sustituto de la identidad recibida en la respuesta autenticada ni de la evidencia externa. Una identidad ausente, inválida, contradictoria, no identificable o que no pueda correlacionarse MUST producir `partial` o `insufficient` y MUST bloquear cualquier `PASS`.

#### Escenario: Correlación completa de la identidad

- GIVEN la respuesta autenticada contiene identidad con forma estrictamente válida y ligada a los resultados de la consulta
- AND la identidad coincide con el protocolo-run esperado y la inspección externa observada del build/container/run
- WHEN el runner valida la procedencia
- THEN MUST registrar la identidad como self-report correlacionado con la ejecución observada
- AND MUST NOT tratar esa correlación como atestación criptográfica
- AND MUST continuar exigiendo el resto de evidencia y aprobación requeridas antes de cualquier `PASS`

#### Escenario: Identidad ausente, inválida o no correlacionable

- GIVEN falta la identidad, no cumple la forma estricta, no coincide con la identidad esperada o no coincide con la inspección externa del container/run
- WHEN el runner valida la procedencia de la consulta
- THEN el estado MUST ser `partial` o `insufficient`, según la evidencia disponible
- AND el runner MUST bloquear `PASS`
- AND el informe MUST identificar la falta, invalidez o discrepancia observada

#### Escenario: Caller intenta sustituir la identidad de respuesta

- GIVEN el caller proporciona una declaración de identidad distinta o afirma una identidad que no aparece en la respuesta autenticada correlacionada
- WHEN el runner valida la procedencia
- THEN el runner MUST NOT usar la declaración del caller como sustituto
- AND MUST clasificar la procedencia como `partial` o `insufficient`
- AND MUST bloquear `PASS`

### Requisito: Mantener H5 diagnostic feature-gated

La ruta `/h5/diagnostic/search` MUST existir únicamente cuando el build habilita `h5-diagnostic`. Un build normal sin esa feature MUST NOT registrar ni servir una ruta H5. Un build con la feature MUST emitir la ruta diagnóstica existente con la autenticación actual; MUST NOT crear otro endpoint ni alterar el router normal o product retrieval.

#### Escenario: Build normal sin feature H5

- GIVEN la aplicación se construye sin `h5-diagnostic`
- WHEN se inspeccionan las rutas registradas y se solicita `/h5/diagnostic/search`
- THEN el router MUST NOT registrar ni servir la ruta H5

#### Escenario: Build con feature H5

- GIVEN la aplicación se construye con `h5-diagnostic`
- WHEN se inspeccionan las rutas registradas
- THEN MUST estar disponible la ruta existente `/h5/diagnostic/search` con la autenticación actual
- AND MUST NOT aparecer un endpoint H5 adicional
- AND el router normal y product retrieval MUST permanecer sin cambios

### Requisito: Limitar veredictos y preservar la corrida histórica

Ningún gate MUST recibir `PASS` ni iniciarse una medición confirmatoria antes de que el protocolo-run completo haya sido aprobado explícitamente. Después de esa aprobación previa, un gate MUST recibir `PASS` únicamente si la procedencia está `verified` y se satisfacen los criterios aprobados para ese gate. Esta especificación no define esos criterios ni concede aprobación de corrida. Ante cualquier condición ausente o incumplida, el gate MUST permanecer no aceptado; el informe MUST distinguir entre resultado descriptivo, no evaluado, bloqueado por protocolo incompleto y fallo del criterio.

La corrida histórica MUST permanecer descriptiva, no preinscrita y no aceptada. MUST NOT reclasificarse retroactivamente ni usarse para cambiar su corpus o etiquetas, establecer criterios, aprobar un protocolo-run o autorizar una medición. Ninguna medición confirmatoria está autorizada por esta especificación.

#### Escenario: Corrida histórica

- GIVEN la corrida descriptiva existente, realizada antes de la aprobación de un protocolo-run
- WHEN se informa esa corrida bajo esta especificación
- THEN MUST conservarse como descriptiva, no preinscrita y no aceptada
- AND MUST NOT cambiarse su corpus o etiquetas
- AND MUST NOT recibir PASS ni justificar aprobación retrospectiva

#### Escenario: Solicitud de medición sin protocolo-run aprobado

- GIVEN no existe un protocolo-run completo y aprobado
- WHEN se solicita una medición confirmatoria
- THEN la solicitud MUST quedar bloqueada
- AND MUST NOT ejecutarse medición confirmatoria

## ADDED Requirements

### Requirement: Apply the approved ANN–exact protocol-run candidate

The protocol-run candidate MUST set `K=10` for the ANN–exact gate and MUST use one run. For every query, `ExactTopKRecall@10` MUST equal `1.0` AND `OrderedTopKMatch@10` MUST equal `1.0` for the gate to pass. The report MUST preserve both metrics per query and MAY include their macro-averages as descriptive summaries only. A macro-average MUST NOT override a failed per-query criterion. The evaluator MUST NOT break ties retrospectively. Any undefined required metric MUST block `PASS`.

This approval applies only to these candidate ANN–exact rules. It does not approve a protocol-run for measurement or establish readiness. The protocol-run MUST remain not-ready until the independent corpus scope and IDs, provenance, and required approvals are complete.

#### Scenario: Every query satisfies both ANN–exact criteria

- GIVEN a candidate protocol-run uses `K=10` and one run
- AND every query has defined `ExactTopKRecall@10` and `OrderedTopKMatch@10` values equal to `1.0`
- WHEN the ANN–exact gate is evaluated
- THEN the gate MAY report `PASS` only if the remaining applicable protocol and provenance gates are also satisfied
- AND the report MUST retain the two metrics for each query
- AND any macro-averages MUST be identified as descriptive

#### Scenario: A query fails despite a passing macro-average

- GIVEN at least one query has either required ANN–exact metric below `1.0`
- AND the descriptive macro-average is `1.0` or otherwise appears favorable
- WHEN the ANN–exact gate is evaluated
- THEN the gate MUST NOT report `PASS`
- AND the report MUST identify the failing query and metric

#### Scenario: A required metric is undefined

- GIVEN a required ANN–exact metric is undefined for any query
- WHEN the ANN–exact gate is evaluated
- THEN the gate MUST NOT report `PASS`
- AND the report MUST identify the query and undefined metric

#### Scenario: Ties are not resolved retrospectively

- GIVEN a tie affects the observed ANN or exact ranking
- WHEN the protocol-run candidate is evaluated
- THEN the evaluator MUST use the observed ranking and MUST NOT apply retrospective tie-breaking

#### Scenario: Corpus or approval inputs are incomplete

- GIVEN the candidate ANN–exact rules are recorded
- AND the corpus scope or IDs, provenance, or required approval inputs are incomplete
- WHEN protocol-run readiness is evaluated
- THEN the protocol-run MUST remain not-ready
- AND no measurement or gate `PASS` MUST be authorized

### Requirement: Keep absolute relevance disabled pending an approved utility threshold

The candidate protocol MUST keep the absolute-relevance gate disabled. `Recall@K` and `nDCG@K` definitions already recorded in this specification MUST NOT enable that gate. A numeric product utility threshold has not been approved; enabling the gate MUST require a separately approved threshold, rubric, labels, and adjudication.

#### Scenario: Validate candidate protocol with absolute relevance disabled

- GIVEN the candidate protocol includes the approved ANN–exact gate
- AND no numeric product utility threshold has been approved
- WHEN protocol-run applicability is validated
- THEN absolute relevance MUST remain disabled and pending
- AND the protocol MUST NOT require relevance labels to evaluate ANN–exact
- AND the protocol MUST NOT report an absolute-relevance `PASS`

#### Scenario: Request to enable absolute relevance without a threshold

- GIVEN no numeric product utility threshold has been approved
- WHEN a protocol-run attempts to enable absolute relevance
- THEN validation MUST block that gate
- AND the missing threshold, rubric, labels, and adjudication approvals MUST be identified

### Requirement: Prepare an independent unlabelled corpus candidate for review

The corpus candidate MUST be a new collection, independent of the existing H5 pilot and hypothetical C0/C1 cases. Its candidate artifact MUST contain stable query and candidate IDs, query text, candidate text, source-path provenance, and a stable hash of the corpus contents. It MUST NOT contain relevance grades, labels, retrieval outputs, or pilot material. Corpus preparation MUST use only eligible, versioned source paths in the approved baseline; it MUST exclude modified `docs/muninn/retrieval-h5-runner.md`, H5 pilot documents and results, the pilot corpus, and hypothetical C0/C1 labels.

The repository's corpus source/license root is MIT. No sensitivity classification has been established, and the owner MUST review the candidate's content and source paths before freezing it. The candidate MUST remain unapproved, unlabelled, and explicitly non-representative and non-confirmatory until that review and an explicit freeze decision. The available inventory of 29 versioned Markdown documents MUST NOT be treated as evidence of sufficient size or representativeness.

For `K=10`, every query MUST have at least 10 eligible candidates. If any query has fewer than 10 candidates, corpus preparation MUST stop or fail; the preparer MUST NOT pad the set with irrelevant candidates. Human labeling MUST occur only after corpus freeze and MUST remain separate from corpus preparation. The candidate labeling process MUST use two independent blinded reviewers and documented adjudication; owner acceptance of this process remains required.

#### Scenario: Candidate corpus contains only eligible unlabelled material

- GIVEN a corpus candidate is prepared from the approved baseline
- WHEN its contents and provenance are reviewed
- THEN each query and candidate MUST have stable IDs and source-path provenance
- AND the artifact MUST include query and candidate text and a stable corpus hash
- AND it MUST contain no grades, labels, retrieval outputs, pilot content, or hypothetical C0/C1 cases
- AND the modified runner guide and H5 pilot documents and results MUST be excluded

#### Scenario: A query has fewer than ten eligible candidates

- GIVEN at least one candidate query has fewer than 10 eligible candidates
- WHEN the corpus candidate is prepared for `K=10`
- THEN preparation MUST stop or fail
- AND the preparer MUST NOT add irrelevant candidates to reach 10
- AND the corpus MUST NOT be represented as ready for protocol use

#### Scenario: Owner reviews and freezes the corpus candidate

- GIVEN the candidate contains eligible source material and remains unlabelled
- WHEN the owner reviews its content and source paths
- THEN the candidate MUST remain a draft until the owner explicitly approves and freezes an identified version
- AND the frozen artifact MUST retain its source-path provenance and stable hash
- AND no claim of representativeness or confirmatory validity MUST be made by preparation alone

#### Scenario: Labels are proposed after corpus freeze

- GIVEN an exact corpus version has been explicitly frozen
- WHEN human relevance labeling is proposed
- THEN labeling MUST be a separate subsequent step
- AND the proposed process MUST use two independent blinded reviewers
- AND disagreements MUST have documented adjudication
- AND owner acceptance of the rubric and labeling process MUST be recorded before the labels can serve as protocol inputs

#### Scenario: Source sensitivity or ownership is unresolved

- GIVEN the source/license root is MIT but no sensitivity classification has been established
- WHEN corpus freeze is considered
- THEN the owner MUST review corpus content and source paths
- AND the candidate MUST remain unapproved until that review and explicit freeze decision are recorded

### Requirement: Do not authorize measurement or claim acceptance from this candidate

This specification and the corpus candidate MUST NOT authorize measurements. No protocol-run or corpus candidate MUST be described as confirmatory, ready, representative, or accepted solely because its candidate rules or draft artifact exist. Existing pilot results MUST remain descriptive and MUST NOT be reused as corpus content or retrospective acceptance evidence.

#### Scenario: Candidate artifacts exist but protocol inputs are incomplete

- GIVEN candidate ANN–exact rules or an unlabelled corpus draft exist
- AND required corpus scope, IDs, provenance, owner review, freeze, or other applicable protocol approvals are incomplete
- WHEN readiness or measurement authorization is requested
- THEN the protocol MUST remain not-ready
- AND the request MUST be blocked without running a measurement

#### Scenario: Existing pilot material is offered as acceptance evidence

- GIVEN an existing H5 pilot result, pilot corpus, pilot label, or hypothetical C0/C1 case is offered
- WHEN the candidate protocol or corpus is reviewed
- THEN that material MUST NOT be included in the independent candidate corpus
- AND it MUST NOT be treated as retrospective acceptance evidence
- AND the existing result MUST remain descriptive

## Enmienda: integración técnica offline ANN-only

Esta enmienda define comportamiento futuro comprobable con dobles offline. No implementa el runner ni autoriza utilizar corpus, modelos, Docker, bridge vivo o base de datos reales. `verify-report.md` y `qa-report.md` conservan su evidencia histórica; no verifican este slice. Los bloqueos de calidad existentes, incluido Clippy con `-D warnings`, permanecen abiertos. Las secciones MODIFIED siguientes sustituyen los requisitos indicados solo en el alcance de esta enmienda.

## MODIFIED Requirements

### Requirement: Mantener separados los gates H5

El protocolo MUST declarar ambos flags explícitamente. La nueva ruta protocol-aware MUST soportar exclusivamente `gates.ann_exact=true` y `gates.absolute_relevance=false`; MUST NOT exigir ambos gates habilitados. Cualquier otra combinación MUST bloquear esta ruta antes de interactuar, sin cambiar la validación estructural general de las cuatro combinaciones. Calidad absoluta MUST permanecer `not_evaluated`, sin métricas de relevancia ni `PASS`. ANN–exact MUST NOT implicar aceptación global H5. (Previously: el protocolo-run debía incluir ambos gates habilitados.)

#### Scenario: ANN-only no requiere annotations

- GIVEN un protocolo sintético completo y corroborado para ANN-only y un corpus sintético sin juicios
- WHEN se solicita ejecutar la nueva ruta con bridge falso
- THEN el runner MUST poder evaluar ANN–exact sin leer, exigir, validar ni fabricar annotations, CSV, hashes de etiquetas, rúbrica o adjudicación
- AND calidad absoluta MUST quedar `not_evaluated` con resultados vacíos
- AND MUST NOT calcular `Recall@K`, `nDCG@K` ni MRR para esa ruta

#### Scenario: Combinación de gates fuera del slice

- GIVEN la solicitud protocol-aware habilita calidad absoluta o deshabilita ANN–exact
- WHEN se realiza el preflight
- THEN la ruta MUST rechazar la combinación como no soportada sin llamadas al bridge ni escrituras de medición
- AND MUST NOT activar la ruta piloto como fallback

### Requirement: Completar y aprobar los campos del protocolo-run

El sistema MUST conservar los campos y aprobaciones aplicables del requisito original, pero MUST reconocer las reglas candidatas ya registradas: `K=10` para ambas métricas ANN–exact, igualdad de ambas a `1.0` por consulta, una corrida, medias solo descriptivas, ranking observado e indefinidos requeridos bloqueantes. Las decisiones exclusivas de relevancia MUST NOT ser requisitos ANN-only. La estructura válida, los wrappers `approved` y estas reglas MUST NOT constituir aprobación del protocolo completo, del corpus ni autorización operativa. El preflight MUST exigir corroboración independiente del hash canónico del documento completo, commit y referencia de revisión, además del freeze/review de la versión/hash exactos del corpus. Ningún resultado futuro MUST ser un requisito fabricado de autorización previa. (Previously: todo K y regla permanecían pendientes y la elegibilidad mezclaba requisitos de entrada con evidencia de ejecución completada.)

#### Scenario: Reglas candidatas presentes sin aprobación real

- GIVEN el candidato contiene las reglas registradas pero sus campos operativos, revisión o freeze siguen pendientes
- WHEN se solicita usarlo en la ruta protocol-aware
- THEN el preflight MUST bloquear sin medición, llamadas al bridge ni escrituras de artefactos de medición
- AND MUST NOT completar decisiones o aprobaciones a partir de las reglas candidatas

#### Scenario: Aprobación de otro documento

- GIVEN el protocolo es estructuralmente válido pero el registro del operador corresponde a otro hash, commit o referencia de revisión
- WHEN se corrobora la aprobación previa
- THEN el preflight MUST bloquear
- AND un cambio en cualquier campo del protocolo completo MUST invalidar la corroboración anterior

## ADDED Requirements

### Requirement: Reconocer un contrato ejecutable mínimo cerrado sin interpretar prosa

La ruta protocol-aware MUST reconocer únicamente la política `ann-exact-per-query-equality-v1`. Este identificador fija la conjunción de `ExactTopKRecall@K` y `OrderedTopKMatch@K`, igualdad numérica estricta de ambas al `threshold` por cada consulta y conjunción sobre todas las consultas aprobadas. No define un motor genérico de reglas ni tolerancia de scores. `criterion`, `rationale`, descripciones de procedencia y otra prosa MUST permanecer informativas; el runner MUST NOT interpretarlas, buscar palabras clave ni ejecutarlas. Si el usuario desea otra política, MUST requerirse una nueva especificación/autorización antes de implementarla.

Para evitar reinterpretar documentos v1 ya existentes, la ampliación mínima MUST identificarse como `schema_version: "h5-acceptance/v2"`. La validación estructural existente de v1 y la invocación piloto MUST conservarse; v1 MUST NOT ser ejecutable por esta nueva ruta, aunque su prosa describa la misma política. La nueva versión MUST reutilizar las envolturas `status/value/rationale`, sus estados y condicionalidad. Python y JSON Schema MUST expresar el mismo contrato cerrado; MUST NOT añadirse defaults ni migración automática de aprobación.

En v2, los valores ejecutables MUST ser exactamente los siguientes:

- `run_inputs.metrics_and_k.value.k_by_metric` MUST contener únicamente `ExactTopKRecall@K: 10` y `OrderedTopKMatch@K: 10`.
- `run_inputs.acceptance_rules_and_tolerances.value` MUST contener únicamente `ann_exact`; ese objeto MUST conservar `criterion` informativo y `threshold` numérico finito igual a `1.0`, y añadir únicamente `policy_id: "ann-exact-per-query-equality-v1"`. Un booleano MUST NOT contar como número.
- `run_inputs.run_count_and_aggregation.value` MUST conservar sus cuatro campos: `run_count` entero igual a `1`, `aggregation: "per-query-governing-macro-descriptive-v1"`, `tie_policy: "observed-ranking-no-retrospective-tiebreak-v1"` y `undefined_policy: "required-undefined-blocks-pass-v1"`. Estos strings MUST ser identificadores exactos, no frases interpretables ni aliases normalizados.
- La agregación MUST conservar cada consulta como unidad gobernante. Si se presentan macro-medias, MUST ser la media aritmética por métrica de todas las consultas aprobadas, únicamente descriptiva; si alguna está indefinida, su macro-media MUST quedar indefinida y MUST informar el conteo. MUST NOT omitirse la consulta indefinida del resumen para aparentar éxito.
- Todas las envolturas aplicables MUST estar completas, sin findings `missing`, `pending` o `malformed`, antes de ejecución. Un identificador desconocido, campo extra, política ausente, K distinto o distinto entre métricas, umbral distinto, número de corridas distinto o regla contradictoria MUST bloquear el preflight. La prosa no concede excepciones a esos valores; cualquier discrepancia declarada MUST resolverse revisando el documento, nunca interpretándola para elegir otra regla.

El nuevo contrato MUST mantener la plantilla genérica incompleta y el candidato real sin aprobación. La mera conversión explícita de un documento a v2 cambia su hash y MUST exigir nueva corroboración del documento completo; MUST NOT heredar una aprobación v1. Esta especificación fija representación técnica de reglas ya autorizadas, no una nueva decisión de producto.

#### Scenario: Política soportada identificada sin depender de criterion

- GIVEN un protocolo sintético v2 tiene todos los valores cerrados indicados y corroboración sintética coherente
- AND `criterion` contiene una descripción informativa no vacía que no se analiza como código
- WHEN el runner reconoce la política
- THEN MUST usar únicamente el contrato estructurado y sus identificadores exactos para determinar el criterio
- AND MUST NOT inferir comparadores ni reglas de la prosa

#### Scenario: Variante desconocida o documento heredado

- GIVEN el documento usa v1 con prosa equivalente, carece de `policy_id`, introduce un token desconocido o cambia cualquier valor ejecutable requerido
- WHEN se solicita la ruta protocol-aware
- THEN el preflight MUST informar el campo o política no soportados y MUST bloquear antes de interactuar o escribir mediciones
- AND MUST NOT seleccionar defaults, inferir equivalencia textual ni promover el documento a v2

### Requirement: Vincular corpus, scope y configuración a entradas aprobadas verificables

V2 MUST añadir únicamente `corpus_hash` al objeto existente `run_inputs.corpus_and_query_candidate_scope.value`, conservando `corpus_id`, `version` y `queries`. `corpus_hash` MUST tener la forma `sha256:` seguida de 64 hexadecimales minúsculos y MUST coincidir con `retrieval_quality.corpus_hash` del corpus validado. La versión y el freeze/review MUST corroborarse con evidencia independiente de ese mismo corpus/hash; el cuerpo del corpus MUST conservar su esquema actual sin metadatos añadidos. Los IDs de consultas y candidatos MUST coincidir exactamente con el scope aprobado, sin duplicados locales, consultas omitidas, IDs extra ni sustitución de textos. El hash MUST detectar textos alterados aun con IDs iguales. Cada consulta MUST contar con al menos diez candidatos elegibles; MUST NOT rellenarse artificialmente.

V2 MUST conservar los campos `retrieval_path` y `candidate_scope` de `ann_exact_comparison_scope.value`, reconocidos únicamente como `/h5/diagnostic/search` y `same-query-candidates-v1`, respectivamente. MUST añadir únicamente `search_config`, objeto cerrado con `limit` entero entre 10 y 100 y `threshold` numérico finito entre 0 y 1, sin booleanos. Esos números operativos MUST proceder del protocolo completo aprobado; esta especificación MUST NOT escogerlos para un corpus real. La configuración efectiva y la evidencia externa MUST corroborar esos valores. Un override del caller MUST rechazarse si no coincide; MUST NOT ajustarse automáticamente `limit` según el piloto o tamaño del corpus.

El scope MUST asegurar que ANN y exact comparan candidatos de la misma consulta y corrida, sin contaminación de otras sesiones, filtros posteriores ni fallback de producción. Si la configuración aprobada no permite realizar esa comparación, el preflight MUST bloquear como scope no soportado, no reinterpretarlo ni cambiar la configuración silenciosamente. Los aliases MUST vincularse al par consulta/candidato; reutilizar un candidate ID entre consultas MUST NOT mezclar sus inserciones o respuestas.

#### Scenario: Identidad de corpus coherente

- GIVEN un corpus sintético validado y una evidencia sintética de freeze/review de la misma versión/hash
- AND el scope aprobado contiene exactamente sus consultas y candidatos y una configuración compatible
- WHEN se realiza el preflight
- THEN MUST comprobarse hash, versión, IDs, configuración y compatibilidad del scope antes de insertar candidatos
- AND MUST conservarse esa misma vinculación en cada solicitud y resultado posterior

#### Scenario: Corpus o configuración discrepantes

- GIVEN el texto cambió con IDs iguales, falta una consulta, hay un ID desconocido, el hash/version no coincide o un override difiere de la configuración aprobada
- WHEN se realiza el preflight
- THEN MUST bloquear sin llamadas al bridge ni escrituras de medición
- AND MUST identificar la discrepancia sin sustituir entradas ni modificar el corpus

### Requirement: Separar autorización previa de evidencia posterior a la respuesta

Antes de cualquier interacción con el bridge, incluida inserción de candidatos, y antes de crear directorios o escribir artefactos de medición, el runner MUST completar un preflight local fail-closed. MUST verificar protocolo presente/válido/soportado, decisiones aplicables completas, hash/aprobación corroborados, corpus congelado y scope/configuración coherentes, identidad esperada allowlisted, identidad de run y evidencia externa disponible de build, dependencias, modelo, entorno y configuración. Claims del caller sin observación independiente MUST NOT satisfacer esos requisitos. Si falta o contradice cualquier requisito previo, MUST bloquear con cero llamadas al bridge y cero escrituras de medición; MAY devolver diagnóstico seguro en memoria o stderr.

El preflight MUST NOT exigir ni fabricar `gate_results`, `encoder_response_identity`, `encoder_response_binding`, `finished_at` ni evidencia de finalización como si existieran antes de responder. `encoder_loaded_identity` y cualquier evidencia de configuración efectiva que dependa de la respuesta MUST permanecer pendientes hasta observarse; MUST NOT derivarse de un hash de disco ni de la identidad esperada. La ausencia esperada de esa evidencia posterior MUST NOT impedir por sí sola iniciar el flujo sintético ya autorizado por entradas previas válidas. Superar preflight MUST NOT producir `PASS`, procedencia final `verified` ni aceptación; `confirmatory_ready` estructural MUST NOT sustituir autorización previa corroborada o verificación final.

Después de cada respuesta, el runner MUST validar resultados e identidad con las allowlists existentes y correlacionar cada respuesta con la consulta y corrida esperadas y con evidencia externa independiente de la misma ejecución. Tras completar la corrida, MUST registrar resultados y tiempos realmente observados y evaluar toda la procedencia aplicable con los helpers existentes. MUST NOT reducir los requisitos finales de procedencia para eludir la separación temporal. Cualquier respuesta faltante, inválida, extra, duplicada o no correlacionable, hash alterado, identidad contradictoria o procedencia final `partial`/`insufficient` MUST bloquear `PASS`. La identidad de respuesta MUST describirse como self-report correlacionado, nunca como atestación.

#### Scenario: Preflight válido sin evidencia futura

- GIVEN entradas previas sintéticas completas y corroboradas sin `finished_at`, resultados ni identidad de respuesta
- WHEN el preflight concluye
- THEN MUST permitir únicamente comenzar el flujo con el bridge falso
- AND MUST mantener la evaluación final pendiente sin inventar evidencia ni declarar `PASS`

#### Scenario: Preflight inválido antes de efectos

- GIVEN protocolo ausente, inválido, pendiente, no corroborado o cualquiera de las comprobaciones previas incumplidas
- WHEN se solicita ejecutar la ruta protocol-aware
- THEN MUST producir un bloqueo seguro con cero llamadas de búsqueda o inserción
- AND MUST NOT crear directorios ni crear, truncar o modificar archivos de medición, incluido un output ya existente
- AND MUST NOT invocar el piloto como fallback

#### Scenario: Evidencia posterior contradictoria

- GIVEN el preflight sintético permitió comenzar y el bridge falso devuelve resultados métricamente perfectos
- AND la identidad, el vínculo consulta/run, el hash o la evidencia externa no coincide o falta evidencia final requerida
- WHEN se evalúa la procedencia posterior
- THEN el gate MUST quedar bloqueado sin `PASS` y MUST identificar el faltante o conflicto
- AND MUST NOT usar evidencia de otra consulta, corrida o aprobación para completarlo

### Requirement: Evaluar únicamente ANN–exact y preservar el piloto descriptivo

La nueva ruta MUST reutilizar validación, hash canónico, corroboración, sanitización, correlación, clasificación de procedencia, fórmulas y serialización existentes; MUST NOT reimplementarlas ni alimentar helpers de relevancia con etiquetas ficticias. MUST calcular ambas métricas sobre las secuencias observadas directas por consulta, con `K=10`, sin ordenar nuevamente por score/ID ni desempatar retrospectivamente. MUST completar exactamente una corrida con cobertura exacta de todas las consultas aprobadas, sin reintentos selectivos para mejorar el gate. Los underfills MUST conservar las definiciones previas de elementos disponibles; salida exacta vacía MUST conservar ambas métricas indefinidas.

Con protocolo/aprobación coherentes y procedencia final `verified`, todas las consultas con ambas métricas definidas iguales a `1.0` MUST satisfacer el criterio ANN–exact; una métrica definida distinta MUST producir `FAIL`; una métrica requerida indefinida MUST producir `blocked`, no `PASS` ni cero imputado. Falta de autorización, cobertura o procedencia MUST producir `blocked` aunque el criterio numérico parezca cumplido. El informe MUST conservar K, valores por consulta, razones de fallo/bloqueo, hash y versión del protocolo, identidad del corpus/run y evidencia correlacionada no sensible. Ningún estado de gate MUST convertirse en aceptación global H5.

La invocación y los informes del piloto existente MUST conservar sus annotations requeridas, valores K históricos y clasificación descriptiva. La selección de ruta MUST ser explícita: una solicitud protocol-aware inválida MUST NOT degradarse a piloto. El slice MUST NOT modificar C0, product retrieval, corpus o resultados históricos ni resolver bloqueos QA. Un `PASS` sintético MUST identificarse exclusivamente como resultado de prueba del contrato técnico.

#### Scenario: Camino positivo sintético ANN-only

- GIVEN fixtures sintéticas independientes del piloto y del corpus candidato con al menos diez candidatos por consulta
- AND preflight válido, bridge falso con ambas secuencias iguales y evidencia final sintética completa/coherente
- WHEN se completa una corrida y se evalúa ANN–exact
- THEN ambas métricas por consulta MUST ser `1.0` y el gate sintético MUST ser `PASS`
- AND calidad absoluta MUST permanecer `not_evaluated` sin métricas ni annotations
- AND el resultado MUST NOT aprobar datos reales, autorizar mediciones ni aceptar H5

#### Scenario: Fallo, indefinido y resumen descriptivo

- GIVEN entradas sintéticas válidas y procedencia final `verified`
- WHEN una consulta tiene orden distinto con recall completo, o devuelve exact vacío
- THEN el primer caso MUST producir `FAIL` por coincidencia ordenada `0` y el segundo MUST producir `blocked` por ambas métricas indefinidas
- AND el informe MUST retener las consultas afectadas sin desempatar ni repetirlas
- AND ninguna macro-media, redondeo o consulta favorable MUST anular ese resultado

#### Scenario: No regresión descriptiva

- GIVEN la invocación piloto existente con sus annotations y un informe histórico preservado
- WHEN se añade la nueva ruta o se ejecutan pruebas de compatibilidad con dobles
- THEN el contrato de invocación y formato descriptivo del piloto MUST conservarse
- AND el informe histórico MUST permanecer sin mutación ni reclasificación

### Requirement: Verificar el slice exclusivamente mediante TDD offline

La fase apply MUST comenzar con pruebas RED de comportamiento y continuar GREEN y Refactor. MUST utilizar bridge falso, observaciones externas falsas explícitas y fixtures sintéticas independientes; MUST probar camino ANN-only sin annotations, cero efectos ante cada fallo previo, contratos desconocidos, hashes/IDs/configuración discrepantes, identidad/procedencia/binding posterior ausentes o contradictorios, fallos/indefinidos y compatibilidad piloto. Las fixtures MUST NOT completar ni corroborar el candidato real y MUST NOT presentarse como evidencia operativa independiente. La fase actual de spec MUST NOT ejecutar pruebas, builds, modelos, Docker, mediciones ni bridge vivo.

#### Scenario: Límite de la evidencia técnica

- GIVEN una suite futura usa exclusivamente dobles y datos sintéticos y demuestra los escenarios del slice
- WHEN se comunica su resultado
- THEN MUST describirse como verificación offline del contrato técnico, no readiness ni aceptación real
- AND freeze/review, autorización de protocolo-run real, blockers históricos y aceptación H5 MUST permanecer pendientes

## Enmienda U9: contrato cerrado de evidencia previa y salida sin efectos

Esta enmienda concreta exclusivamente el preflight técnico offline de U9/fase 14. Prevalece sobre cualquier formulación anterior que deje sin definir los registros indicados o que permita comenzar U10 al terminar la entrega intermedia U9. La ejecución sintética posterior y su evaluación final siguen perteneciendo a U10/U11; no se eliminan sus requisitos. El WIP, el historial y los contratos implementados y revisados de U7/U8 permanecen vigentes. U9 MUST permanecer bloqueada para apply hasta actualizar secuencialmente spec y design; esta actualización de spec no implementa U9 ni verifica código.

Los nombres, claves y tokens siguientes son formato técnico, no nuevas decisiones humanas de aprobación. Esta enmienda MUST NOT aprobar ni congelar corpus reales, completar candidatos, autorizar mediciones ni modificar C0, QA, verify o los bloqueos históricos. La evidencia sintética MUST permanecer explícitamente limitada a pruebas del contrato offline.

## ADDED Requirements

### Requirement: Especializar registros previos sin cambiar el contrato genérico U8

La nueva ruta protocol-aware MUST exigir observaciones locales del proveedor `ProtocolRunEvidence.before_run`, sin promover un diccionario de claims del caller a `LocalObservations`. Dentro de esas observaciones, `corpus_freeze_review`, `storage_scope`, `effective_config` y `operator_corroboration` MUST utilizar envolturas cerradas con exactamente cuatro propiedades obligatorias: `value`, `source`, `method` y `observed_at`; únicamente `conflict` MAY aparecer como propiedad opcional. `source`, `method` y `observed_at` MUST ser strings con contenido después de comprobar whitespace, sin modificar su valor. Si existe, `conflict` MUST ser booleano; `true` MUST bloquear. Cualquier propiedad adicional, campo obligatorio ausente o tipo inválido MUST bloquear.

Cada `value` MUST ser el objeto cerrado específico definido a continuación, sin propiedades adicionales en ningún nivel. Los identificadores y referencias de decisión MUST ser strings no vacíos ni placeholders; los hashes de corpus MUST contener exactamente `sha256:` seguido de 64 hexadecimales minúsculos. Las comparaciones MUST usar valores exactos, sin coerción de tipos, normalización de strings, aliases de tokens ni defaults. Los booleanos MUST NOT contar como números. Los registros MUST NOT contener textos del corpus, secretos ni rutas sensibles.

Esta especialización MUST aplicarse a la nueva ruta, sin cambiar la validación genérica polimórfica de valores de U8, sus baselines ni sus clasificaciones. MUST NOT añadir formato timestamp, orden temporal ni política de antigüedad a `observed_at`; la comprobación temporal existente de aprobación MUST conservarse separada. Los objetos cerrados de esta enmienda no convierten el conjunto completo de observaciones en una allowlist nueva ni eliminan otros registros obligatorios.

La forma válida, el tipo `LocalObservations`, `source`, `method` o cualquier token de estado MUST NOT interpretarse como autenticidad, autoridad humana, atestación o prueba suficiente de adquisición independiente. Las fuentes externas de build, dependencias, modelo, entorno y configuración requeridas anteriormente MUST seguir siendo necesarias. Los strings descriptivos de `provenance.value` MUST NOT sustituirlas.

#### Scenario: Registros especializados completos y límite U8 preservado

- GIVEN observaciones sintéticas locales con las cuatro envolturas y valores cerrados válidos
- WHEN la nueva ruta comprueba su forma y sus correspondencias
- THEN MUST poder continuar las restantes comprobaciones de preflight
- AND MUST NOT cambiar la aceptación genérica de valores polimórficos en U8
- AND MUST NOT afirmar aprobación real ni procedencia final `verified`

#### Scenario: Forma especializada inválida o claim promovido

- GIVEN una envoltura tiene una clave extra, un metadato no string, un `conflict` no booleano o un valor con tipo incorrecto
- OR las observaciones son un diccionario de claims sin adquisición independiente por el proveedor
- WHEN se solicita el preflight protocol-aware
- THEN MUST bloquear antes de cualquier efecto
- AND MUST NOT reparar, convertir ni completar el registro para aceptarlo

### Requirement: Corroborar freeze y revisión del corpus exacto

El registro previo `corpus_freeze_review.value` MUST contener exactamente `corpus_id`, `version`, `corpus_hash`, `status` y `review_reference`. `status` MUST ser el string literal `frozen`. Los otros strings MUST cumplir las restricciones comunes de esta enmienda.

`corpus_id`, `version` y `corpus_hash` MUST coincidir exactamente con `run_inputs.corpus_and_query_candidate_scope.value`. `corpus_id` MUST coincidir también con el corpus validado; `corpus_hash` MUST coincidir con `retrieval_quality.corpus_hash` de ese corpus. La versión MUST corroborarse externamente: MUST NOT añadirse al cuerpo cerrado del corpus.

`review_reference` MUST identificar la revisión explícita del contenido y de los source paths que sustenta el freeze de esa versión/hash exactos. MUST NOT exigirse que coincida con `repository_review.review_reference`, porque la revisión del corpus y la del protocolo pueden ser distintas. Este registro MUST NOT requerir `run_id`: el freeze identifica el corpus, no una ejecución. El vínculo con la ejecución MUST comprobarse mediante el scope aprobado y los registros de run de la misma llamada.

La ausencia del registro, estado draft/pending o desconocido, referencia pendiente, discrepancia de identidad o hash MUST bloquear. Un manifiesto `draft/not frozen`, licencia MIT, hash correcto o cantidad suficiente de candidatos MUST NOT convertirse en freeze/review. El token `frozen` MUST describir una decisión corroborada por el proveedor, no crearla. En pruebas, una evidencia falsa explícita MUST representar únicamente una condición sintética y MUST NOT congelar el candidato real.

#### Scenario: Freeze sintético del corpus identificado

- GIVEN corpus validado, scope aprobado y registro sintético de freeze con el mismo ID, versión y hash
- AND la referencia identifica la revisión sintética de contenido y source paths
- WHEN se comprueba la correspondencia
- THEN este requisito MUST quedar satisfecho para la prueba offline
- AND una referencia distinta de la revisión del protocolo MUST NOT bloquear por esa diferencia sola
- AND MUST NOT inferirse freeze real ni autorización de medición

#### Scenario: Draft o freeze de otra versión

- GIVEN el registro falta, declara un estado distinto de `frozen` o identifica otra versión/hash
- WHEN la nueva ruta evalúa el corpus
- THEN MUST bloquear sin modificar el corpus ni su manifiesto
- AND MUST NOT sustituir el registro con información del candidato real o del protocolo

### Requirement: Acreditar aislamiento de corrida y ambos inventarios antes de insertar

El registro previo `storage_scope.value` MUST contener exactamente `run_id`, `bridge_container_id`, `storage_id`, `isolation`, `observed_candidates` y `planned_candidates`. `run_id`, `bridge_container_id` y `storage_id` MUST ser identificadores strings no vacíos ni placeholders. `storage_id` MUST ser opaco y MUST NOT ser una ruta sensible. `isolation` MUST ser el string literal `run-exclusive-disposable-v1`.

`observed_candidates` MUST ser una lista exactamente vacía. Esa lista MUST representar el inventario global completo observado en el almacenamiento antes de insertar, no una consulta, sesión o subconjunto filtrado por corpus. Al estar dentro de un objeto `value` no vacío, esta lista vacía MUST NOT tratarse como un registro ausente. Cualquier almacenamiento con candidatos preexistentes MUST bloquear este slice, aunque los candidatos pertenezcan al corpus aprobado.

`planned_candidates` MUST ser una lista no vacía de objetos cerrados con exactamente `query_id` y `candidate_id`, ambos strings de identificadores válidos del corpus. Los pares MUST ser únicos y MUST coincidir exactamente con todos los pares del corpus validado y del scope aprobado, sin faltantes ni extras. El orden de esta lista MUST NOT gobernar ranking ni aceptación; la correspondencia MUST comparar conjuntos solo después de rechazar duplicados. El total de pares MUST ser menor o igual que `search_config.limit` aprobado. Un candidate ID compartido entre consultas MUST contar como una inserción por cada par y MUST NOT fusionarse globalmente.

`storage_scope.value.run_id` MUST coincidir con el argumento `run_id` y con `run_id.value` observado. `bridge_container_id` MUST coincidir con la observación externa homónima de la misma ejecución. La evidencia de aislamiento MUST acreditar almacenamiento desechable exclusivo de ese run, sin escritores ajenos durante la corrida. Un snapshot vacío por sí solo MUST NOT acreditar esa exclusividad. Si el proveedor no puede acreditar aislamiento y ambos inventarios, el preflight MUST bloquear.

El runner MAY calcular los pares esperados para compararlos, pero MUST NOT presentarlos como inventario global observado ni fabricar la evidencia del proveedor copiando el corpus. El token de aislamiento MUST NOT autorizar crear, borrar o resetear almacenamiento real. El preflight MUST NOT reutilizar el incremento automático de límite del piloto ni ajustar la configuración para admitir el total.

#### Scenario: Almacenamiento sintético vacío y plan completo compatible

- GIVEN evidencia falsa explícita de aislamiento exclusivo del run y un inventario global observado vacío
- AND los pares previstos son únicos y coinciden con corpus y scope
- AND su total no excede el límite aprobado
- WHEN se comprueba `storage_scope`
- THEN este requisito MUST permitir continuar el preflight offline
- AND MUST conservar separadas las inserciones de un candidate ID compartido entre consultas

#### Scenario: Contaminación, plan discordante o límite insuficiente

- GIVEN hay candidatos preexistentes, el inventario solo cubre una sesión o no se acredita exclusividad durante la corrida
- OR el plan tiene pares extra, ausentes o duplicados, corresponde a otro run/container o excede el límite
- WHEN se comprueba el scope
- THEN MUST bloquear antes de insertar
- AND MUST NOT limpiar almacenamiento, deduplicar el plan, cambiar el límite ni aplicar fallback piloto

### Requirement: Corroborar configuración externa previa sin fabricar configuración final

El registro `effective_config.value` previo de la nueva ruta MUST contener exactamente `run_id`, `retrieval_path`, `candidate_scope` y `search_config`. `run_id` MUST coincidir con el argumento y el registro de run observado. `retrieval_path` MUST ser `/h5/diagnostic/search` y `candidate_scope` MUST ser `same-query-candidates-v1`; ambos MUST coincidir con el scope ANN–exact aprobado.

`search_config` MUST ser un objeto cerrado con exactamente `limit` y `threshold`. `limit` MUST ser entero entre 10 y 100; `threshold` MUST ser número finito entre 0 y 1; los booleanos MUST rechazarse. Ambos valores MUST coincidir exactamente con `run_inputs.ann_exact_comparison_scope.value.search_config`. Si se proporciona `search_config_override`, MUST tener esa misma forma y los mismos valores aprobados; cualquier discrepancia o tipo inválido MUST bloquear sin coerción ni ajuste silencioso.

Este registro previo MUST representar únicamente configuración externa disponible antes de la respuesta. Copiar valores esperados del protocolo MUST NOT satisfacer observación independiente ni probar que el bridge ejecutó ya una solicitud. MUST NOT completarse la parte dependiente de respuesta ni `encoder_loaded_identity` a partir de este objeto. La configuración final completa y todos los requisitos finales de procedencia MUST seguir siendo obligatorios en U11; la elegibilidad previa MUST NOT sustituirlos.

#### Scenario: Configuración previa coherente sin datos futuros

- GIVEN observación externa sintética previa con los mismos run, ruta, scope y configuración aprobados
- AND no existen todavía identidad loaded ni configuración dependiente de respuesta
- WHEN se realiza el preflight
- THEN la falta esperada de esos datos futuros MUST NOT bloquear por sí sola la elegibilidad previa
- AND MUST NOT afirmarse configuración final completa ni procedencia `verified`

#### Scenario: Override o configuración externa discrepante

- GIVEN un override difiere del protocolo, usa booleanos o contiene claves extra
- OR la configuración observada pertenece a otro run o no coincide con el scope aprobado
- WHEN se realiza el preflight
- THEN MUST bloquear con configuración sin modificar
- AND MUST NOT sustituir la observación con la expectativa del protocolo

### Requirement: Reutilizar corroboración del operador y comprobar todos los bindings previos

`operator_corroboration.value` MUST reutilizar sin ampliar el objeto cerrado existente de `operator_record`: exactamente `protocol_sha256`, `commit`, `review_reference`, `corroborated_at` y `corroborated_by`. El valor observado MUST ser exactamente igual al `operator_record` recibido por la nueva API. La aprobación MUST corroborarse con `corroborate_approval` y el hash canónico del documento completo, conservando sus comprobaciones existentes de readiness, identidad de revisión y tiempos de aprobación/corroboración. MUST NOT añadirse `run_id`, aprobación de corpus ni nuevos campos a ese contrato, ni duplicarse o sustituirse su lógica temporal.

La nueva ruta MUST comprobar conjuntamente los siguientes bindings de las observaciones previas de la misma llamada:

- `protocol_hash.value` y `operator_record.protocol_sha256` MUST coincidir con el hash canónico calculado del protocolo completo.
- `review_reference.value` y `operator_record.review_reference` MUST coincidir con `repository_review.review_reference`; `operator_record.commit` MUST coincidir con `repository_review.commit`.
- `corpus_hash.value` MUST coincidir con el hash calculado del corpus validado y el hash del scope aprobado; el registro freeze/review MUST cumplir sus correspondencias independientes.
- `encoder_expected_identity.value` MUST cumplir la allowlist existente y coincidir exactamente con `run_inputs.encoder_identity.value` aprobado.
- `run_id.value` MUST coincidir con el argumento de la API, `storage_scope.value.run_id` y `effective_config.value.run_id`; el container de `storage_scope` MUST coincidir con el registro externo `bridge_container_id`.

Los valores esperados MUST servir para comparación, no para reemplazar observaciones discordantes. Una ausencia, contradicción, malformación o `conflict=true` en un requisito previo MUST bloquear; MUST NOT sobrescribirse silenciosamente con el valor aprobado. La corroboración del protocolo MUST vincular el corpus a través del scope incluido en el hash completo, sin sustituir el freeze/review independiente. La correspondencia de run MUST provenir de las observaciones de esa ejecución, sin atribuir al protocolo o al bridge un run ID que no contienen.

#### Scenario: Documento completo y observaciones previas coherentes

- GIVEN un protocolo sintético completo cuya corroboración cumple el contrato existente
- AND las observaciones de la misma llamada coinciden con documento, corpus, identidad esperada, run y container
- WHEN la nueva ruta comprueba los bindings
- THEN MUST poder completar el preflight sin ampliar `operator_record`
- AND MUST NOT elevar los tokens o los metadatos a prueba de autoridad real

#### Scenario: Corroboración o binding de otro contexto

- GIVEN cualquier campo del protocolo cambia después de su corroboración
- OR un registro corresponde a otro hash, commit, revisión, corpus, identidad, run o container
- OR `operator_corroboration.value` no es igual al `operator_record` recibido
- WHEN se realiza el preflight
- THEN MUST bloquear y conservar la discrepancia en findings seguros
- AND MUST NOT completar la evidencia con valores esperados ni registros de otra ejecución

## MODIFIED Requirements

### Requirement: Terminar la entrega intermedia U9 con elegibilidad previa y gates no evaluados

La entrega U9 MUST comprobar únicamente los requisitos previos y devolver un informe en memoria. Al superar todas las comprobaciones, MUST añadir `preflight` con exactamente `status: "eligible"` y `findings: []`; `gates.ann_exact` y `gates.absolute_relevance` MUST tener cada uno exactamente `status: "not_evaluated"` y `results: {}`; `measurement_started` MUST ser `false`. El informe MUST conservar contexto seguro de protocolo, corpus y run, sin textos, UUID de inserciones, secretos ni aceptación global. La elegibilidad de procedencia previa MAY adjuntarse fuera de esos objetos cerrados como contexto de preflight, pero MUST NOT etiquetarse como procedencia final. (Previously: el escenario de preflight válido permitía comenzar el flujo sintético sin definir el punto de parada de la entrega U9.)

Ante cualquier rechazo previo, `preflight` MUST contener exactamente `status: "blocked"` y `findings` no vacíos; `gates.ann_exact` MUST ser exactamente `{status: "blocked", results: {}}`, `gates.absolute_relevance` MUST ser exactamente `{status: "not_evaluated", results: {}}` y `measurement_started` MUST permanecer `false`. Los findings MUST ser seguros y deterministas, con paths y razones estables; las categorías y paths `missing`, `pending` y `malformed` del validador del protocolo MUST conservarse en el contexto estructural, sin reemplazarse por una excepción genérica. MUST NOT incluirse entrada cruda o datos sensibles en los errores.

En ambos casos U9 MUST terminar sin invocar `Bridge.post`, `after_response` ni `after_run`; MUST NOT insertar, buscar, calcular métricas, leer annotations, invocar helpers de relevancia, crear directorios ni crear, truncar o modificar archivos de medición. Un output centinela preexistente MUST permanecer intacto. MUST NOT llamar al piloto como fallback, añadir un flag público `preflight_only`, crear CLI nueva ni cambiar `protocol_preflight` histórico para resolver esta salida intermedia.

U10 MAY extender posteriormente el camino positivo de la misma API para comenzar exclusivamente el flujo sintético autorizado, manteniendo el significado de `preflight` y los requisitos posteriores completos de U10/U11. El éxito intermedio MUST NOT informar `PASS`, procedencia final `verified`, ejecución completada ni autorización operativa real. La entrega U9 MUST NOT implementar U10 anticipadamente.

#### Scenario: U9 supera preflight y se detiene antes de U10

- GIVEN entradas previas sintéticas completas, válidas, compatibles y corroboradas
- AND no hay evidencia futura de resultados, respuesta, loaded ni finalización
- WHEN termina la entrega U9 de la nueva API
- THEN `preflight.status` MUST ser `eligible` y `preflight.findings` MUST ser una lista vacía
- AND ambos gates MUST ser `not_evaluated` con resultados vacíos y `measurement_started` MUST ser `false`
- AND los spies MUST registrar cero llamadas a `Bridge.post`, `after_response`, `after_run`, escritura y métricas
- AND MUST NOT ejecutarse U10 ni fabricarse evidencia futura

#### Scenario: U9 bloquea y preserva estado externo

- GIVEN falla cualquier requisito previo o binding
- AND existe un archivo centinela anterior
- WHEN termina la nueva API en U9
- THEN `preflight.status` MUST ser `blocked` con findings no vacíos, seguros y deterministas
- AND ANN–exact MUST ser `blocked`, calidad absoluta MUST ser `not_evaluated` y ambos resultados MUST estar vacíos
- AND `measurement_started` MUST ser `false` y el centinela MUST permanecer intacto
- AND MUST registrarse cero llamadas a `Bridge.post`, `after_response`, `after_run`, escritura y métricas
- AND MUST NOT ejecutarse piloto, fallback ni limpieza destructiva
