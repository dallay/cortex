# Especificación de aceptación de recuperación H5

## Propósito

Esta especificación define dos gates de aceptación independientes para H5: equivalencia entre recuperación ANN y exacta, y calidad absoluta de recuperación frente a etiquetas. Los valores estadísticos y operativos que no cuentan con aprobación explícita son entradas obligatorias del protocolo, no valores predeterminados. Esta especificación no aprueba por sí misma ningún valor ni autoriza una medición confirmatoria.

## Requisitos

### Requisito: Mantener separados los gates H5

El sistema de evaluación MUST emitir un resultado independiente para el gate de equivalencia ANN–exact y el gate de calidad absoluta etiquetada. Un resultado favorable en uno MUST NOT implicar, sustituir ni mejorar el resultado del otro.

#### Escenario: Evaluación de ambos gates

- GIVEN un protocolo aprobado que incluye ambos gates
- WHEN se evalúa una ejecución confirmatoria
- THEN el informe MUST presentar un resultado independiente para cada gate
- AND el informe MUST NOT derivar la aceptación de un gate del resultado del otro

#### Escenario: Solo se evalúa un gate

- GIVEN una ejecución que cubre solo uno de los gates
- WHEN se genera su informe
- THEN el gate no evaluado MUST figurar como no evaluado
- AND el informe MUST NOT declarar aceptación global de H5

### Requisito: Preinscribir el protocolo ANN–exact antes de medir

Para que una medición pueda producir un veredicto de aceptación del gate ANN–exact, el protocolo MUST registrar antes de la medición confirmatoria: objetivo; versión e hash del corpus de consultas y candidatos; alcance de datos y condiciones de comparación; métrica o criterio y K; regla de aceptación y tolerancia, si aplica; número de ejecuciones y método de agregación; ruta de comportamiento exacta bajo prueba, incluidos scope, filtros, threshold, fallback y tratamiento de empates; e identidad y procedencia verificable independientemente del encoder/modelo, build/código, configuración y entorno. El usuario o responsable de producto MUST aprobar explícitamente cada decisión gobernante y la versión completa del protocolo antes de iniciar la medición confirmatoria.

Ningún criterio estadístico, tolerancia ni número de ejecuciones se fija aquí. La igualdad de orden, la comparación numérica de scores y cualquier otra regla de aceptación son decisiones que deben quedar registradas y aprobadas para esta medición. La tolerancia H4 `1e-9` queda explícitamente fuera del alcance como umbral H5 y MUST NOT ser importada como criterio H5.

#### Escenario: Protocolo ANN–exact completo y aprobado

- GIVEN que todos los campos de preinscripción ANN–exact están completos
- AND cada decisión gobernante tiene aprobación explícita anterior a la medición
- AND la ruta de comportamiento fija el mismo corpus, consulta, scope, filtros, threshold, fallback, K y tratamiento de empates que se evaluarán
- WHEN comienza la medición confirmatoria
- THEN el evaluador MUST vincular los resultados a esa versión inmutable del protocolo
- AND MUST evaluar ANN y exact conforme a la ruta registrada

#### Escenario: Falta una decisión o aprobación ANN–exact

- GIVEN que falta un campo requerido o su aprobación explícita anterior a la medición
- WHEN se ejecuta o informa la comparación ANN–exact
- THEN el resultado MUST permanecer descriptivo y no aceptado
- AND el gate MUST NOT recibir PASS, aunque la salida observada parezca coincidir

#### Escenario: Alcance de comparación incompleto

- GIVEN que el protocolo no fija si la comparación cubre salidas directas o el comportamiento público completo
- OR omite cualquiera de scope, filtros, threshold, fallback o tratamiento de empates aplicable
- WHEN se solicita un veredicto ANN–exact
- THEN el gate MUST quedar no aceptado por protocolo incompleto
- AND el informe MUST identificar el alcance o comportamiento omitido

### Requisito: Preinscribir el protocolo de calidad absoluta antes de medir

Para que una medición pueda producir un veredicto de aceptación del gate de calidad absoluta, el protocolo MUST registrar antes de la medición confirmatoria: objetivo de calidad; versión e hash del corpus de consultas y candidatos; procedencia y versión de las etiquetas y rúbrica; método de adjudicación y tratamiento de desacuerdos y consultas sin candidatos relevantes; métrica o métricas gobernantes y valores K; regla de aceptación y tolerancia, si aplica; número de ejecuciones y método de agregación entre consultas y ejecuciones; ruta de recuperación exacta bajo prueba, incluidos scope, filtros, threshold, fallback y empates; e identidad y procedencia verificable independientemente del encoder/modelo, build/código, configuración y entorno. El usuario o responsable de producto MUST aprobar explícitamente cada decisión gobernante y la versión completa del protocolo antes de iniciar la medición confirmatoria.

Este requisito no designa Recall, MRR, nDCG, K, corpus, adjudicación, umbral, tolerancia, tamaño de muestra, número de ejecuciones ni método de agregación como criterio aprobado. Los datos indefinidos o no calculables MUST tener tratamiento preinscrito; el evaluador MUST NOT descartarlos o reinterpretarlos después de observar resultados para obtener aceptación.

#### Escenario: Protocolo de calidad completo y aprobado

- GIVEN que los corpus, etiquetas, rúbrica, adjudicación, métrica/K, regla, ejecuciones, agregación, ruta de comportamiento y procedencia están registrados
- AND cada decisión gobernante tiene aprobación explícita anterior a la medición
- WHEN se ejecuta la medición confirmatoria
- THEN el evaluador MUST calcular y agregar las métricas según el protocolo aprobado
- AND el informe MUST conservar resultados por consulta y por ejecución suficientes para verificar la agregación

#### Escenario: Falta aprobación o definición del criterio de calidad

- GIVEN que falta la aprobación previa de cualquier campo requerido, incluida la métrica/K o la regla de aceptación
- WHEN se calcula o informa la calidad etiquetada
- THEN el resultado MUST permanecer descriptivo y no aceptado
- AND el gate MUST NOT recibir PASS

#### Escenario: Etiquetas sin adjudicación o sin tratamiento aprobado

- GIVEN que las etiquetas carecen de la versión, rúbrica o proceso de adjudicación requerido
- OR el protocolo no establece cómo tratar consultas sin relevantes o resultados indefinidos
- WHEN se solicita un veredicto de calidad absoluta
- THEN el gate MUST quedar no aceptado por protocolo incompleto
- AND los resultados observados MAY informarse solo como descriptivos, con esas limitaciones visibles

### Requisito: Clasificar la procedencia de forma verificable

Cada gate MUST informar el estado de procedencia por separado. La procedencia MUST incluir evidencia verificable independientemente para la identidad y versión del encoder/modelo, build/código/bridge, corpus y etiquetas pertinentes, configuración completa de recuperación y entorno de ejecución. Una declaración del caller sin evidencia independiente MUST NOT contar como verificación.

El estado MUST ser `verified` solo si toda la procedencia requerida está presente, corresponde a la ejecución y se valida independientemente. El estado MUST ser `partial` si existe evidencia verificable, pero falta o no se ha validado uno o más elementos requeridos. El estado MUST ser `insufficient` si no hay evidencia suficiente para identificar de forma confiable la ejecución o si la procedencia contradice la ejecución observada. Los estados `partial` e `insufficient` MUST impedir PASS y dejar el gate no aceptado; el informe MUST nombrar los elementos faltantes, no verificados o contradictorios. La procedencia `verified` no sustituye la aprobación previa del protocolo ni el cumplimiento del criterio estadístico.

#### Escenario: Procedencia completa

- GIVEN que todas las identidades requeridas tienen evidencia independiente y corresponden a la ejecución
- WHEN se determina el estado de procedencia
- THEN el estado MUST ser `verified`
- AND el gate solo podrá ser aceptado si también se satisfacen todos los demás requisitos

#### Escenario: Procedencia parcial

- GIVEN que algunos elementos tienen evidencia independiente, pero al menos uno requerido falta o no está validado
- WHEN se determina el estado de procedencia
- THEN el estado MUST ser `partial`
- AND el gate MUST NOT recibir PASS
- AND el informe MUST enumerar lo que falta o requiere verificación

#### Escenario: Procedencia insuficiente o contradictoria

- GIVEN que la evidencia no permite identificar confiablemente una o más identidades requeridas
- OR la evidencia contradice la configuración o ejecución observada
- WHEN se determina el estado de procedencia
- THEN el estado MUST ser `insufficient`
- AND el gate MUST NOT recibir PASS
- AND el informe MUST describir la insuficiencia o contradicción

### Requisito: Limitar los veredictos y preservar la evidencia exploratoria

Un gate MUST recibir `PASS` únicamente si su protocolo se aprobó explícitamente antes de la medición confirmatoria, la preinscripción está completa, la procedencia está `verified` y el resultado satisface la regla aprobada. Ante cualquier condición ausente o incumplida, el gate MUST permanecer no aceptado; el informe MUST distinguir entre resultado descriptivo, no evaluado, bloqueado por protocolo incompleto y fallo del criterio, sin presentar ninguno como PASS. El informe MUST preservar el historial de la corrida y no reclasificar evidencia retrospectivamente.

La corrida exploratoria H5 de seis consultas y una ejecución MUST permanecer descriptiva, no preinscrita y no aceptada. MUST NOT usarse para aprobar retrospectivamente un protocolo ni para inferir umbral, tolerancia, número de ejecuciones, suficiencia del corpus o PASS. La regla H4 `1e-9` MUST permanecer fuera de alcance como umbral H5.

#### Escenario: Criterios cumplidos con preinscripción y procedencia completas

- GIVEN una medición confirmatoria posterior a la aprobación explícita
- AND el protocolo está completo, la procedencia es `verified` y se satisfacen sus reglas
- WHEN se emite el informe
- THEN solo el gate correspondiente MUST recibir `PASS`
- AND el resultado del otro gate MUST permanecer independiente

#### Escenario: Se reutiliza la corrida exploratoria de seis consultas

- GIVEN la corrida descriptiva existente, realizada antes de la aprobación de un protocolo confirmatorio
- WHEN se informa esa corrida bajo esta especificación
- THEN MUST conservarse como descriptiva, no preinscrita y no aceptada
- AND MUST NOT recibir PASS ni justificar aprobación retrospectiva

#### Escenario: Se intenta usar H4 `1e-9` como umbral H5

- GIVEN que una propuesta de protocolo H5 usa H4 `1e-9` sin una nueva aprobación explícita para H5
- WHEN se valida la preinscripción
- THEN el protocolo MUST rechazarse como incompleto para aceptación
- AND el valor MUST NOT tratarse como umbral H5
