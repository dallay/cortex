# Diseño: protocolo de aceptación H5

## Resumen de alcance

La propuesta aprobada fija dos gates independientes: equivalencia ANN–exact y calidad absoluta de recuperación frente a etiquetas. Este diseño define el marco técnico y los requisitos de aprobación y evidencia que se aplicarán a cada protocolo-run futuro, sin escoger métricas, K, umbrales, tolerancias, tamaños de muestra, corpus ni reglas de adjudicación.

Este SDD diseña el framework de protocolo, observación, evaluación e informe. No implementa ni pone en marcha un servicio, no ejecuta mediciones y no autoriza una nueva medición confirmatoria. Los valores concretos de cada protocolo-run son entradas futuras: no bloquean completar el diseño, las tareas ni el framework, pero sí bloquean aprobar un protocolo-run confirmatorio, medir de forma confirmatoria o emitir PASS hasta que estén definidos y aprobados explícitamente.

## Enfoque técnico

Extender el runner local aislado existente (`dev/muninn/retrieval_h5_runner.py`) y el módulo de métricas (`dev/muninn/retrieval_quality.py`) con un framework que valide protocolos versionados, recoja observaciones locales de ejecución, evalúe cada gate de forma independiente y produzca un informe trazable. El artefacto de protocolo tendrá versión, hash de contenido, referencia de commit/revisión y aprobación previa corroborada por el operador. El runner validará integridad y consistencia, pero no afirmará verificar criptográficamente la aprobación.

La confianza se limita al repositorio, el operador y el host/Docker Engine. La revisión del repositorio y su commit/hash son evidencia de proceso; el operador debe corroborar que la referencia de revisión aprobó exactamente ese contenido antes de la corrida. No se requieren firmas, claves, raíces de confianza ni atestaciones. Esas opciones solo podrían evaluarse en un trabajo futuro.

Cada gate tendrá su propio estado de procedencia y su propio resultado. No habrá un estado de procedencia único compartido ni estado agregado que habilite PASS. Cada gate evalúa su conjunto obligatorio y no degradable de evidencias. Si no se puede establecer, dentro del límite local, qué encoder/configuración cargó realmente el bridge, la procedencia de ese gate será `partial` y ese gate no podrá recibir PASS.

La corrida exploratoria existente permanece descriptiva, no preinscrita y no aceptada. El diseño no la reclasifica ni deriva criterios de ella.

## Decisiones de arquitectura

### Decisión: preservar los dos gates aprobados sin combinarlos

**Elección**: informar por separado equivalencia ANN–exact y calidad absoluta etiquetada. El estado de cada gate se calcula únicamente con sus propias precondiciones, evidencia y criterio aprobado. El gate no ejecutado figura como no evaluado. No se define un PASS global ni una agregación que pueda habilitar aceptación.

**Alternativas consideradas**: score compuesto, gate único o una decisión global derivada de estados parciales. Se descartan porque mezclarían propiedades distintas y permitirían que un resultado favorable compensara evidencia o resultado faltante en otro gate.

**Justificación**: paridad con la búsqueda exacta no prueba relevancia; relevancia no prueba equivalencia algorítmica. La propuesta ya aprobó esta separación.

### Decisión: protocolo-run versionado y aprobado como evidencia de proceso

**Elección**: cada protocolo-run futuro identifica su versión inmutable, hash del contenido, commit y referencia a la revisión del repositorio. El usuario o responsable autorizado aprueba explícitamente las decisiones aplicables y el protocolo completo antes de la medición confirmatoria. El operador corrobora que la revisión aprobó el commit/hash exacto que se ejecutará. Un cambio de contenido requiere nueva versión y nueva aprobación.

El runner valida esquema, completitud, canonicalización/hash y coherencia de referencias. Los campos de revisión son trazabilidad del proceso; no son por sí solos prueba criptográfica de identidad, aprobación ni estado remoto de una revisión. Una declaración del caller no sustituye la corroboración del operador.

**Alternativas consideradas**: aprobación inferida de una identidad CLI, o firmas y atestaciones obligatorias. Se descartan: la primera no demuestra aprobación y las segundas no forman parte del modelo local elegido.

**Justificación**: permite auditar qué versión se aprobó y ejecutó, sin atribuir al runner capacidades de verificación que no tiene.

### Decisión: baseline de evidencia obligatoria por gate, no configurable por caller

**Elección**: fijar en el framework un baseline obligatorio y no degradable. El protocolo no acepta campos tipo `evidence_requirement` ni opciones de caller que reduzcan, sustituyan o desactiven evidencia. Cada gate debe validar todas las categorías aplicables de su baseline antes de ser elegible para PASS.

Baseline común de ambos gates:

- Identidad/hash del protocolo, referencia de revisión y corroboración del operador de la aprobación previa.
- Hash y versión del corpus de consultas y candidatos usado en la corrida.
- Hashes del runner y del módulo de calidad (`retrieval_h5_runner.py` y `retrieval_quality.py`).
- ID de la imagen y del contenedor bridge, y hash de los bytes del binario bridge observables localmente.
- Hashes del bundle y archivos efectivos del modelo staged; mounts de solo lectura corroborados mediante inspección del contenedor.
- Configuración efectiva de retrieval y salida de inspección de Compose/contenedores, incluidos los mounts y modos pertinentes.
- Run ID, marcas de tiempo de inicio y fin, y resultados del gate vinculados a ese run ID.

Evidencia adicional obligatoria para calidad absoluta:

- Hash/versión de etiquetas, versión/hash de la rúbrica y evidencia del proceso de adjudicación aplicable.

La evidencia se deriva de observaciones locales —Docker Engine/inspect y lectura/hash de bytes— y se separa de los valores declarados por quien invoca el runner. El informe conserva origen, método y momento de observación, faltantes y contradicciones. Cuando los bytes o la configuración realmente cargados por el bridge no puedan establecerse dentro del límite de confianza local, el estado será `partial`; el informe debe identificar el faltante y no puede emitir PASS. No se sustituye esta limitación por claims del caller.

**Alternativas consideradas**: hacer configurable el nivel de evidencia, aceptar claims del bridge/caller como autoridad o requerir atestación firmada. Se descartan por permitir degradar el mínimo obligatorio o por exceder el modelo local acordado.

**Justificación**: los requisitos de procedencia de la especificación se conservan como invariantes del framework, no como preferencias de cada corrida.

### Decisión: estado de procedencia y elegibilidad de PASS por gate

**Elección**: cada objeto de gate incluye `provenance.status` con valor `verified`, `partial` o `insufficient`, además de `status` del resultado. `verified` significa que todo el baseline aplicable a ese gate está presente, correlacionado con la corrida y coherente según observaciones locales independientes de los claims del caller. `partial` significa que hay evidencia útil, pero falta o no se puede establecer un elemento obligatorio. `insufficient` indica que no se puede identificar fiablemente la ejecución o que hay contradicciones. Ambos estados no verificados impiden PASS.

El informe puede incluir un resumen textual de faltantes, pero no tendrá un estado agregado de procedencia o aceptación que pueda habilitar PASS. La regla de elegibilidad se aplica individualmente: aprobación previa completa para el gate + protocolo-run completo + baseline de ese gate `verified` + criterio aprobado satisfecho. Si falta cualquiera, ese gate no recibe PASS.

**Alternativas consideradas**: un único `provenance.status` global, un booleano o permitir PASS desde una agregación. Se descartan porque ocultan qué evidencia corresponde a cada gate y podrían permitir que el estado de un gate cubra la insuficiencia del otro.

**Justificación**: el gate evaluado determina el conjunto de evidencia pertinente y la decisión; la ausencia de evidencia nunca se convierte en resultado favorable.

### Decisión: observación local sin ampliar la confianza declarada

**Elección**: recoger, antes, durante y después de cada ejecución, los datos disponibles mediante Docker Engine/`docker inspect` y lectura local de artefactos. Capturar configuración efectiva observable, IDs de Compose/servicios, imagen y contenedores, mounts/modos, estado y tiempos. Calcular hashes de los bytes accesibles de runner, módulo de calidad, bridge, bundle/archivos staged, corpus, etiquetas, rúbrica y configuración efectiva. Correlacionar todas las observaciones con un run ID.

Estas observaciones comparten el límite de confianza del host y del operador. Un hash demuestra identidad de los bytes observados, no que esos bytes se ejecutaron ni qué quedó cargado en memoria. `verified` no significa verificación criptográfica ni resistencia a un host comprometido.

**Alternativas consideradas**: depender de argumentos CLI/env, tratar inspect de imagen como prueba de ejecución, añadir endpoint diagnóstico o exigir atestaciones firmadas. No se aceptan como sustitutos del baseline local. Si no se puede establecer la carga efectiva del encoder/config, se conserva el estado `partial` y se bloquea PASS.

**Justificación**: distingue observación de claims y explicita los límites sin introducir una nueva infraestructura de confianza.

### Decisión: mantener intactos retrieval y los datos persistentes

**Elección**: limitar el cambio al framework de protocolo, colector, integración del runner, informe y pruebas/documentación asociadas. Mantener Compose H5 y el modo descriptivo. No cambiar ANN, búsqueda exacta, ranking, filtros, scope, threshold, fallback, Store ni contratos públicos. No efectuar mediciones durante diseño o implementación del framework.

**Alternativas consideradas**: adaptar retrieval para satisfacer criterios aún no aprobados o añadir un servicio de medición. Se descartan por alcance y porque alterarían el objeto evaluado.

**Justificación**: el framework debe observar un comportamiento fijado y preparar gates, no decidir ni alterar sus criterios.

## Flujo de datos

```text
Protocolo-run futuro ──→ revisión/commit/hash ──→ operador corrobora aprobación
       │                                              │
       ├── campos aplicables completos y aprobados ──┤
       │                                              ↓
Docker inspect + bytes locales ──→ baseline obligatorio por gate
       │                                              │
       └── run ID + timestamps ──→ runner ──→ bridge/datos observados
                                          │
                     ┌────────────────────┴───────────────────┐
                     ↓                                        ↓
             gate ANN–exact                         gate calidad absoluta
          evidencia propia/status                   evidencia propia/status
                     └────────────────────┬───────────────────┘
                                          ↓
                    informe sin estado agregado habilitante
```

1. El framework valida esquema, hash y completitud estructural. Los valores aún no aprobados de métricas, K, reglas, tolerancias, ejecuciones, agregación, corpus o adjudicación son campos futuros del protocolo-run. No impiden completar el framework, pero bloquean un protocolo-run confirmatorio y cualquier PASS hasta que se completen y aprueben.
2. Antes de una corrida confirmatoria, el operador corrobora que la revisión del repositorio aprobó el contenido exacto identificado por commit/hash y que el protocolo cubre el gate que se ejecutará. El runner registra la evidencia de proceso, sin afirmar verificación criptográfica.
3. El preflight captura observaciones de Compose/contenedores, mounts de solo lectura y hashes accesibles. Asigna un run ID y fija marcas de tiempo. No toma parámetros del caller como evidencia suficiente de identidad.
4. Para cada gate solicitado, el colector evalúa su baseline propio. Una categoría no observable produce `partial`; contradicciones o imposibilidad de identificar la ejecución producen `insufficient`. Si la carga real del encoder o de la configuración efectiva del bridge no puede establecerse dentro del límite local, el gate queda `partial` y sin PASS.
5. La evaluación aplica solo las reglas aprobadas para ese protocolo-run. ANN–exact y calidad absoluta reciben resultados independientes. Errores técnicos o evidencia incompleta no se convierten en PASS ni en fallo estadístico.
6. El informe presenta por gate el estado de protocolo/aprobación, `provenance.status`, resultados, faltantes/contradicciones y referencias de evidencia. No incluye estado global que habilite aceptación. Conserva la corrida exploratoria histórica como descriptiva.

## Archivos previstos

| Archivo | Acción | Descripción |
|---|---|---|
| `dev/muninn/retrieval_h5_protocol.py` | Crear | Framework de esquema, canonicalización/hash y validación de completitud por gate, sin valores de aceptación predeterminados ni opciones para debilitar el baseline. |
| `dev/muninn/retrieval_h5_provenance.py` | Crear | Colector local para inspección Docker/Compose, hashes, mounts, timestamps/run ID y estado de evidencia por gate. |
| `dev/muninn/retrieval_h5_runner.py` | Modificar | Integrar preflight, corroboración registrada de aprobación, evaluación independiente por gate e informe; conservar modo descriptivo. |
| `dev/muninn/retrieval_quality.py` | Modificar | Mantener métricas existentes y parametrizarlas solo desde protocolo-run completo y aprobado. |
| `dev/muninn/tests/test_retrieval_h5_protocol.py` | Crear | Validar campos, hash, revisión y bloqueo confirmatorio cuando falten valores/aprobación; comprobar que no se pueda degradar evidencia. |
| `dev/muninn/tests/test_retrieval_h5_provenance.py` | Crear | Validar observaciones Docker, hashes, mounts, campos obligatorios por gate, estado parcial/insuficiente y claims no verificables. |
| `dev/muninn/tests/test_retrieval_h5_runner.py` | Modificar | Probar orden de preflight, estados de procedencia independientes, ausencia de PASS agregado y bloqueo si encoder/config no se establecen. |
| `dev/muninn/tests/test_retrieval_quality.py` | Modificar | Verificar que métricas/reglas procedan de un protocolo-run aprobado y no de valores del piloto. |
| `dev/muninn/fixtures/retrieval-h5-protocol.schema.json` | Crear | Esquema del protocolo sin defaults estadísticos ni campos caller-configurables que rebajen evidencia. |
| `dev/muninn/fixtures/retrieval-h5-protocol.example.json` | Crear | Plantilla no ejecutable que identifique campos pendientes; no representa valores elegidos ni puede producir PASS. |
| `docs/muninn/retrieval-h5-runner.md` | Modificar | Documentar framework, gates, evidencia obligatoria, aprobación corroborada y límite de confianza local. |
| `docs/muninn/retrieval-h5-local-results.md` | Sin cambio | Conservar resultados históricos descriptivos y sus limitaciones. |
| `crates/muninn/neo4j/src/queries/short_term.rs` | Sin cambio | No alterar consultas ANN/exact. |
| `crates/muninn/neo4j/src/short_term.rs` | Sin cambio | No alterar flujo público ni fallback. |
| Store y contratos públicos de retrieval | Sin cambio | Fuera de alcance. |

Los archivos previstos describen el framework a implementar; este diseño no afirma que se hayan creado ni ejecutado. No se añade un servicio de medición ni se realiza una medición.

## Interfaces y contratos

Esquema conceptual. Los marcadores de campos pendientes son requisitos para futuros protocolo-runs, no decisiones de valores tomadas en este diseño. No se acepta una propiedad caller-configurable como `evidence_requirement`.

```json
{
  "schema_version": "h5-acceptance/v1",
  "protocol_id": "identificador-de-producto",
  "protocol_version": "version-inmutable",
  "protocol_sha256": "sha256:...",
  "repository_review": {
    "commit": "referencia-de-commit",
    "review_reference": "referencia-a-la-revision",
    "approved_at": "RFC3339",
    "approved_gates": ["ann_exact", "absolute_relevance"]
  },
  "run_inputs": {
    "metrics_and_k": "REQUIRED_PER_RUN_NO_DEFAULT",
    "acceptance_rules_and_tolerances": "REQUIRED_PER_RUN_NO_DEFAULT",
    "run_count_and_aggregation": "REQUIRED_PER_RUN_NO_DEFAULT",
    "corpus_and_query_candidate_scope": "REQUIRED_PER_RUN_NO_DEFAULT",
    "labels_rubric_and_adjudication": "REQUIRED_FOR_ABSOLUTE_QUALITY",
    "ann_exact_comparison_scope": "REQUIRED_FOR_ANN_EXACT"
  },
  "gates": {
    "ann_exact": {"enabled": "explicit per-run decision"},
    "absolute_relevance": {"enabled": "explicit per-run decision"}
  }
}
```

El baseline de procedencia se mantiene en la implementación/esquema del framework y no puede ser rebajado por el documento de protocolo ni por argumentos del caller:

```text
common: protocol hash + review reference/corroboration;
        query/candidate corpus hash + version;
        runner + quality-module hashes;
        bridge image/container ID + observable binary bytes hash;
        actual staged model bundle/file hashes + read-only mounts;
        effective retrieval config + Compose/container inspect;
        run ID + timestamps + gate results.
absolute_relevance additionally: labels + rubric + adjudication evidence.
```

Ejemplo de informe; el estado de procedencia vive dentro de cada gate y no hay estado global:

```json
{
  "run_id": "identificador-unico",
  "protocol": {
    "version": "...",
    "sha256": "...",
    "commit": "...",
    "review_reference": "...",
    "operator_corroborated": true
  },
  "gates": {
    "ann_exact": {
      "status": "not_evaluated | blocked | descriptive | PASS | FAIL",
      "provenance": {
        "status": "verified | partial | insufficient",
        "observations": [],
        "missing_or_conflicting": []
      },
      "results": {},
      "started_at": "RFC3339",
      "finished_at": "RFC3339"
    },
    "absolute_relevance": {
      "status": "not_evaluated | blocked | descriptive | PASS | FAIL",
      "provenance": {
        "status": "verified | partial | insufficient",
        "observations": [],
        "missing_or_conflicting": []
      },
      "results": {},
      "started_at": "RFC3339",
      "finished_at": "RFC3339"
    }
  }
}
```

Los nombres de estado del ejemplo son etiquetas conceptuales que deben alinearse con la implementación; no eligen reglas estadísticas. No se acepta que un estado o resumen global convierta `partial`, `insufficient`, `blocked` o `not_evaluated` en PASS. La aprobación de revisión/commit/hash es evidencia de proceso corroborada por el operador, dentro del límite de confianza del repositorio, operador y host.

## Estrategia de pruebas

| Capa | Qué probar | Enfoque |
|---|---|---|
| Unit | Protocolo-run | Campos requeridos, hash/versionado, revisión, ausencia de defaults y bloqueo confirmatorio por valores o aprobaciones pendientes. |
| Unit | Baseline no degradable | Rechazar `evidence_requirement` y cualquier mecanismo que elimine o reduzca evidencias obligatorias; verificar conjuntos distintos por gate. |
| Unit | Observación local | Parseo de inspect, hashes de bytes, bridge image/container, mounts de solo lectura, bundle staged, config efectiva, run ID y timestamps. |
| Unit | Estado por gate | `verified` solo con baseline completo; `partial` ante observación faltante, incluyendo encoder/config no establecidos; `insufficient` ante identidad no fiable o contradicción. |
| Unit | Elegibilidad | PASS solo para el gate cuya aprobación previa, protocolo-run completo, evidencia `verified` y regla aprobada se cumplen; nunca derivarlo de un agregado. |
| Unit | Métricas | Selección y agregación solo desde valores aprobados; ningún valor inferido del piloto, H4 o referencia externa Proposed. |
| Integración | Runner con Docker/bridge simulados | Preflight antes de confirmación, correlación por run ID, verificación de mounts, y no PASS si no se demuestra encoder/config cargados. |
| Integración | Informe | Estados y observaciones separados por gate, faltantes visibles, referencias/hash, redacción de secretos y ausencia de estado global habilitante. |
| Regresión | Modo descriptivo | Preservar la corrida exploratoria como descriptiva, no preinscrita y no aceptada. |

El trabajo ejecutable futuro seguirá TDD. No se ejecutará una medición confirmatoria durante el diseño o construcción del framework.

## Despliegue y reversión

No se requiere migración de Store ni cambio de API pública. El framework puede completarse y probarse sin fijar criterios por corrida. La medición confirmatoria y cualquier PASS permanecen bloqueados hasta que cada protocolo-run aplicable tenga sus valores definidos y aprobados, y la evidencia obligatoria del gate correspondiente esté `verified` dentro del límite local.

Reversión: desactivar el flujo confirmatorio y conservar el modo descriptivo. Mantener protocolos, referencias de revisión, informes y resultados como registros; no borrar ni sobrescribir evidencia ni reclasificar resultados históricos. No tocar Store ni código de retrieval.

## Preguntas abiertas

Los valores de cada protocolo-run siguen pendientes de aprobación y no bloquean el diseño, las tareas ni el framework. Sí bloquean cada protocolo-run confirmatorio, la medición confirmatoria y PASS. Incluyen métricas y K, reglas/tolerancias, número de ejecuciones y agregación, tratamiento de resultados indefinidos, corpus/representatividad, etiquetas/rúbrica/adjudicación y unidad/alcance ANN–exact (salidas directas o comportamiento público completo).

La propuesta de dos gates está aprobada y no queda abierta como cuestión de alcance. Este diseño y sus ejemplos no eligen ninguno de los valores anteriores. La revisión del repositorio más commit/hash y corroboración del operador son evidencia de proceso, no verificación criptográfica. El modelo de confianza está limitado a repositorio, operador y host; firmas/atestaciones no son requisitos.

## Riesgos

- El modelo local no resiste manipulación de un repositorio, operador, host o Docker Engine comprometido. `verified` no debe describirse como verificación criptográfica.
- `docker inspect` y el hash de imagen/binario no demuestran por sí solos qué encoder/config cargó el proceso. Si esa carga no se puede establecer dentro del límite local, el estado por gate será `partial` y no habrá PASS.
- Los hashes identifican bytes observados, pero no prueban que los bytes se ejecutaron ni su procedencia de build. El informe debe distinguir integridad observada de atestación.
- La referencia a revisión, commit/hash y la aprobación corroborada por el operador son evidencia de proceso, no una firma verificada por el runner.
- Los metadatos y hashes de configuración pueden revelar rutas o datos sensibles. El colector debe excluir secretos y redactar valores sensibles sin omitir hechos necesarios para verificar configuración y mounts.
- Los valores aún no aprobados no impiden diseñar o construir el framework; sí impiden medir de forma confirmatoria o emitir PASS. No inferirlos del piloto, H4 ni de la especificación externa Proposed.
