# Diseño: protocolo de aceptación H5

## Vigencia de la enmienda offline ANN-only

La sección final «Diseño actualizado: integración técnica offline ANN-only» aplica la enmienda final de `specs/retrieval-acceptance/spec.md` y prevalece únicamente donde contradice el diseño anterior. Las secciones previas se conservan como WIP e historial de decisiones; no describen por sí solas el estado actual del código. No se autoriza medición real, aprobación/freeze del corpus, resolución de C0, aceptación H5 ni archivo. La evidencia histórica de `qa-report.md` y `verify-report.md`, incluido QA BLOCKED y Clippy con `-D warnings`, permanece intacta y no verifica el slice nuevo.

## Enfoque técnico

Mantener separados los gates ANN–exact y de calidad absoluta, y completar la procedencia de H5 con el cambio mínimo en el bridge existente y su runner. `/h5/diagnostic/search` incluirá una identidad allowlisted en la misma respuesta que ANN/exact. El valor se obtendrá del objeto encoder que está en `App` y que codificó la consulta; no se añadirá una ruta, una llamada de observación ni una fuente basada en logs.

La identidad será self-report del proceso, no attestation. La respuesta puede sostener hoy `artifact_bundle_id` y `space_id`. El encoder ya conserva el primero y el port de embedding expone el segundo. Al cargar, `CandidateCpuSentenceEncoder::load` lee los bytes verificados de `modules.json` y `1_Pooling/config.json`, y `profile` comprueba que su contenido corresponde exactamente al perfil esperado. No conserva hoy esos bytes, un fingerprint ni una identidad del perfil validado. Se puede calcular y retener un SHA-256 durante esa misma carga sobre los bytes de configuración efectivamente validados, con encuadre inequívoco de ambos archivos; el reporte no debe reabrir archivos. El identificador de perfil debe nombrar esa validación concreta; no se debe reutilizar `minilm-cpu-f32-v1`, que identifica el contrato del espacio vectorial y no es por sí mismo una versión del perfil de configuración. La identidad del perfil solo se añadirá si el código define y prueba un identificador para la
comprobación exacta de esos archivos.

El fingerprint solo identifica los bytes de configuración que la ruta de carga leyó y validó. No identifica por sí solo la configuración efectiva completa del proceso ni demuestra el estado de memoria. El `artifact_bundle_id` es el identificador declarado que deriva del recibo/manifiesto verificado; no es un hash recién calculado de los pesos residentes. No hay soporte actual para un hash de todo el modelo en memoria ni para una identidad de build derivada del encoder. No se afirmará ninguna de esas cosas.

La enmienda de validación estructural del protocolo-run se implementará dentro del validador y fixture existentes. El JSON Schema dejará de aceptar `{}` para campos de decisión y declarará tipos y propiedades permitidas en cada nivel. El validador Python producirá un informe determinista y por JSON path para campos ausentes, pendientes y malformados; los estados no se inferirán de `None`, cadenas vacías o la mera presencia de cualquier valor. La plantilla seguirá siendo explícitamente incompleta. La validación de forma y estado no autoriza una corrida ni decide aceptación.

## Decisiones de arquitectura

### Decisión: obtener la identidad desde el mismo encoder que atiende la consulta

**Elección**: añadir una capacidad de lectura de identidad a la instancia de encoder guardada en `App` (manteniendo su acceso mediante `EmbeddingPort` o una extensión compatible). La implementación concreta debe delegar a `CandidateCpuSentenceEncoder` para el bundle y al adaptador para `space_id`; el handler toma esa identidad de `app.encoder` y serializa los resultados y la identidad juntos. La carga retiene asimismo el fingerprint de configuración calculado sobre los buffers ya leídos y validados.

**Alternativas consideradas**: reconstruir los metadatos en el handler desde variables de entorno, leerlos de nuevo del filesystem, calcularlos desde el caller o incluirlos en logs. Se descartan porque no prueban que la misma instancia que codificó la consulta los originó, abren una ventana de cambio entre lecturas o no forman parte de la respuesta correlacionable.

**Justificación**: el handler ya codifica la consulta mediante `app.encoder` y usa ese vector para ANN y exact. Obtener el reporte desde ese mismo objeto establece el vínculo de proceso más estrecho disponible sin cambiar el flujo de búsqueda.

### Decisión: limitar campos reportados a los que acredita el camino de carga

**Elección**: objeto cerrado con dos grupos: `declared` (`artifact_bundle_id`, `model_id`, `revision`, `space_id`) y `loaded` (`validated_profile`, `config_sha256`). `artifact_bundle_id` y `space_id` ya se pueden leer desde la instancia construida; `model_id` y `revision` están disponibles en el manifiesto pinned que `load_encoder` lee antes de moverlo a la caché, aunque no se retienen hoy en el encoder. La carga puede retener esos valores declarados junto con la identidad construida. `validated_profile` debe ser un identificador nuevo, estable y probado para el predicado exacto de la función `profile`; no debe confundirse con la versión del contrato de embedding. `config_sha256` se calcula durante la carga a partir de los bytes verificados de `modules.json` y `1_Pooling/config.json`, en orden fijo y con longitud/etiqueta de archivo incluidas en la entrada hash. El encoder retiene el resultado inmutable hasta terminar el proceso.

El código actual ya hace accesibles `artifact_bundle_id` desde `CandidateCpuSentenceEncoder` y `space_id` desde `EmbeddingPort`; el manifiesto pinned dispone de `model_id` y `revision`, pero ninguno de estos valores se expone hoy como identidad común. Tampoco se retiene fingerprint/configuración validada. Los dos campos `loaded` son implementables porque el perfil se valida con esos buffers en el camino de carga. El código futuro debe definir el encuadre hash sin ambigüedad y probarlo con bytes conocidos. Si el camino real no permite capturar exactamente dichos bytes, esos campos no se inventan ni se sustituyen por hash de disco: se omiten de la respuesta y la procedencia queda `partial`/`insufficient`, nunca `PASS`.

No se incluyen un supuesto hash del manifiesto separado del `artifact_bundle_id`, hashes de pesos como estado de memoria, rutas, nombre de archivos adicionales, identidad Docker, build ID o run ID del proceso. Estos valores no los acredita actualmente el encoder construido. La imagen/container/run se observa externamente en el runner.

**Alternativas consideradas**: reportar hashes leídos después desde el bundle; llamar al bundle ID fingerprint de pesos en memoria; presentar el perfil como configuración completa. Se descartan porque confundirían observación del disco, declaración de bundle y estado cargado.

**Justificación**: mantener una identidad pequeña y verdadera es más útil que completar campos de procedencia con claims no sustentados por interfaces reales.

### Decisión: conservar el límite de diagnóstico existente

**Elección**: extender únicamente `H5DiagnosticOutput` y `h5_diagnostic_search`, bajo `h5-diagnostic`. Se mantienen autenticación, validación de entrada, proyección de alias y ANN/exact actuales. El build sin la feature continúa sin registrar la ruta.

**Alternativas consideradas**: endpoint nuevo, middleware de observación, cambio al router normal, product retrieval o Store. Se descartan por alcance y porque crearían fuentes/caminos de ejecución no requeridos.

**Justificación**: la especificación exige que el self-report viaje con los resultados de la misma búsqueda autenticada.

### Decisión: correlacionar el self-report con evidencia externa sin elevar su confianza

**Elección**: el runner acepta solo la forma y tipos allowlisted; rechaza campos extra, grupos combinados, valores ausentes, tipos inválidos y valores no esperados. Correlaciona la identidad de la respuesta de la consulta con el bundle esperado del protocolo-run, la observación Docker externa de imagen/container y la ejecución/run que originó esa respuesta. La respuesta por sí sola no verifica procedencia. Ausencia o discrepancia deja el estado en `partial` o `insufficient` y bloquea `PASS` en ambos gates aplicables.

Los valores esperados del protocolo-run y la observación Docker son entradas independientes del self-report. No se fija aquí ningún identificador, protocolo de run, umbral, K ni política de aceptación. La identidad no es atestación criptográfica y no resiste compromiso del proceso, bridge, host u operador.

**Alternativas consideradas**: permitir que el caller sobrescriba la identidad, aceptar cualquier JSON de identidad, o considerar el self-report evidencia independiente suficiente. Se descartan por permitir sustitución, ambigüedad o PASS sin corroboración externa.

**Justificación**: la correlación sirve para verificar consistencia dentro del trust boundary local acordado, no para prometer una raíz de confianza que el sistema no tiene.

## Flujo de datos

```text
load_encoder()
  ├─ VerifiedArtifactCache: verifica bundle/manifiesto/archivos
  ├─ CandidateCpuSentenceEncoder::load
  │    ├─ lee y valida modules.json + pooling config
  │    └─ retiene fingerprint de esos bytes y perfil validado
  └─ CandidateMiniLmEmbeddingAdapter (bundle + space_id + encoder)
             │ misma instancia en App.encoder
             ↓
/h5/diagnostic/search ── encode(query) ──→ vector ──→ ANN y exact
             └── identidad de App.encoder + mismos resultados ──→ respuesta JSON
                                                           │
Runner: sanitiza estrictamente ── correlaciona expected bundle,
        Docker image/container y request/run observados ──→ procedencia por gate
```

El runner no hace llamadas adicionales al bridge para obtener metadatos y no usa logs como evidencia. La correlación con la consulta se obtiene porque identidad y resultados pertenecen a una sola respuesta autenticada. El contexto de request/run se registra en el runner alrededor de la llamada existente; Docker image/container se observa por separado. La imposibilidad de vincular una respuesta con esa ejecución impide PASS.

La salida de diagnóstico sigue excluyendo payloads y vectores: conserva la proyección actual `alias`, `score`, `rank` para resultados, junto con el objeto de identidad. La allowlist de identidad no amplía el conjunto de datos de búsqueda.

## Archivos previstos

| Archivo                                                                                             | Acción                                                  | Descripción                                                                                                                                                                                                         |
|-----------------------------------------------------------------------------------------------------|---------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `dev/muninn/bridge/src/main.rs`                                                                     | Modificar                                               | Añadir identidad al output de `/h5/diagnostic/search` desde `app.encoder`, sin cambiar auth, feature gate, ANN/exact ni rutas normales. Añadir cobertura de serialización/handler o adaptar las pruebas existentes. |
| `dev/muninn/bridge/src/embedding_port.rs`                                                           | Modificar                                               | Exponer una representación de identidad compatible a través del encoder de la aplicación; conservar `space_id` actual y evitar cambios de comportamiento de `encode`.                                               |
| `dev/muninn/bridge/src/minilm_encoder.rs`                                                           | Modificar                                               | Retener `validated_profile` y fingerprint generado durante `load` desde los mismos buffers verificados que consume `profile`; mantener accesible `artifact_bundle_id`. No releer artefactos para reportar.          |
| `dev/muninn/bridge/src/h5_diagnostic.rs`                                                            | Modificar si hace falta                                 | Definir/validar el tipo serializable cerrado de identidad diagnóstica; no introducir datos de mensaje, embedding ni ruta. Si el tipo vive naturalmente en `main.rs`, no crear modificación innecesaria aquí.        |
| `dev/muninn/retrieval_h5_runner.py`                                                                 | Modificar                                               | Consumir identidad en la respuesta existente y marcarla con la consulta/run observado; no emitir una segunda solicitud para identidad.                                                                              |
| `dev/muninn/retrieval_h5_provenance.py`                                                             | Modificar                                               | Sanitizador estricto y correlación con identidad esperada del protocolo-run y observaciones externas Docker de image/container/run; estados parciales/insuficientes bloquean PASS.                                  |
| `dev/muninn/retrieval_h5_protocol.py`                                                               | Modificar si se requiere por el esquema ya especificado | Validar presencia y tipos de campos de identidad esperada sin escoger sus valores ni aprobar un protocolo-run. No permitir configuración caller que debilite el baseline.                                           |
| `dev/muninn/fixtures/retrieval-h5-protocol.schema.json`                                             | Modificar                                               | Reemplazar esquemas vacíos de inputs por estructuras estrictas y strongly typed; cerrar propiedades anidadas y expresar aplicabilidad condicional por gate sin establecer valores de decisión.                      |
| `dev/muninn/fixtures/retrieval-h5-protocol.example.json`                                            | Modificar                                               | Mantener una plantilla incompleta en la que cada decisión aplicable tenga estado `pending` explícito y `value: null`; no introducir valores por defecto ni un ejemplo confirmatory-ready.                           |
| `dev/muninn/bridge/Cargo.toml` y tests del bridge                                                   | Sin cambio previsto                                     | Mantener dependencias y feature-gate actuales; modificar solo si los tests existentes requieren ajuste compatible. No añadir dependencias por fingerprint: `sha2` ya está usado por el bridge.                      |
| `dev/muninn/tests/test_retrieval_h5_provenance.py`                                                  | Modificar                                               | Pruebas TDD del sanitizador y correlación, incluyendo extras, campos ausentes, tipos y valores discrepantes.                                                                                                        |
| `dev/muninn/tests/test_retrieval_h5_runner.py`                                                      | Modificar                                               | Confirmar que el self-report pertenece a la respuesta de búsqueda/run existente y que no hay solicitud extra para identidad.                                                                                        |
| `dev/muninn/tests/test_retrieval_h5_protocol.py`                                                    | Modificar si el esquema esperado se implementa allí     | Comprobar validación de campos de identidad requeridos sin defaults ni valores de protocolo elegidos.                                                                                                               |
| `dev/muninn/tests/test_retrieval_h5_protocol.py`                                                    | Modificar                                               | Ampliar cobertura TDD de estructura estricta, hallazgos con JSON paths, estados pending, placeholders, propiedades desconocidas, campos condicionales y bloqueo de readiness/aprobación.                            |
| `dev/muninn/bridge/src/main.rs` tests feature-gated y tests de integración Rust existentes          | Modificar                                               | Verificar claves de respuesta, autenticación, presencia de feature route y ausencia de ruta en build normal, conservando expectativas existentes.                                                                   |
| `dev/muninn/retrieval_h5_diagnostics.py`, `dev/muninn/retrieval_h5_local.py`, Store y router normal | Sin cambio                                              | No alterar análisis, modos descriptivos existentes, API no diagnóstica, búsqueda ni persistencia salvo ajustes estrictamente necesarios para compatibilidad de fixtures.                                            |

Los caminos de archivos señalan integración planificada, no afirman que los cambios existan. La implementación debe inspeccionar y reutilizar las fixtures/helpers de pruebas actuales antes de modificar contratos. No se cambian documentos de resultados ni se ejecuta una medición confirmatoria.

## Interfaces y contratos

### Contrato estructural del protocolo-run

Los campos de decisión reutilizarán una envoltura cerrada y explícita, sin hacer global ningún campo aplicable solo a un gate:

```json
{
  "status": "pending",
  "value": null,
  "rationale": ""
}
```

Para `status: "pending"`, `value` MUST ser `null`; `rationale` conserva una explicación del estado pendiente y no una decisión inventada. Para `status: "approved"`, `value` MUST ajustarse al tipo y estructura específicos del campo, no puede ser nulo, vacío ni placeholder, y `rationale` MUST ser texto no vacío. `status` será una enumeración cerrada. Esta etiqueta JSON es contenido declarativo, no prueba de aprobación humana. La plantilla usa estados pendientes para toda decisión aún no resuelta y nunca queda confirmatory-ready.

La envoltura es común, pero su `value` se valida mediante un esquema específico por ruta. Una decisión numérica, un objeto de configuración y una lista de ítems no se validan como JSON arbitrario. Objetos y listas deben tener estructura y propiedades allowlisted expresas; se rechazan propiedades desconocidas recursivamente. No se fijan dominios de valores que dependan de decisiones de producto. La validación exige forma/tipo y estado, pero no decide si un valor es correcto para una corrida.

Los requisitos condicionales se derivan de `gates`: `labels_rubric_and_adjudication` solo aplica cuando `absolute_relevance` está habilitado; `ann_exact_comparison_scope` solo cuando `ann_exact` está habilitado. En `acceptance_rules_and_tolerances.value`, la decisión aprobada MUST incluir una regla por cada gate habilitado y MUST NOT incluir propiedades para gates deshabilitados; esas propiedades no aplican y se rechazan. La forma de `value` se valida contra la combinación explícita de gates, sin fijar reglas ni umbrales. Si ambos gates están deshabilitados, no se inventa una regla ni se exige habilitar un gate: se permite que la decisión siga `pending` con `value: null`, lo que mantiene `confirmatory_ready` bloqueado. Se conserva el rechazo fail-closed de una decisión `approved` con `value` vacío; esa forma no se acepta como decisión aprobada válida. Los campos comunes se mantienen separados. La implementación no convertirá campos condicionales en globales ni alterará la independencia
de los dos gates o el rol descriptivo de MRR.

El resultado del validador expondrá colecciones deterministas separadas, cada una con objetos que incluyan `path` JSON y categoría/razón estable: `missing`, `pending` y `malformed`. `missing` cubre rutas aplicables ausentes; `pending` cubre envolturas válidas con decisión sin resolver; `malformed` cubre tipos o formas inválidos, valores vacíos/placeholders y propiedades desconocidas. Un objeto/lista vacío no cuenta como contenido presente. Cualquier hallazgo bloquea `confirmatory_ready`; la respuesta no aplica defaults, no completa decisiones y no devuelve `PASS`. El validador puede seguir calculando el hash canónico para identificar el documento, pero el hash no valida ni aprueba su contenido.

La aprobación humana sigue acreditándose por el protocolo completo/versionado y su hash exacto, junto con la corroboración del operador existente. `approved_gates`, `status: "approved"`, rationale, metadatos o cualquier claim dentro del JSON por sí solos no bastan. Se conserva el flujo existente de `corroborate_approval`; la validación estructural no lo sustituye ni lo debilita. Una estructura válida tampoco equivale a aprobación, aceptación de gates ni permiso de medición.

La forma conceptual de la respuesta conserva `ann` y `exact` actuales y añade un único objeto cerrado `encoder_identity`:

```json
{
  "ann": [],
  "exact": [],
  "encoder_identity": {
    "declared": {
      "artifact_bundle_id": "...",
      "model_id": "...",
      "revision": "...",
      "space_id": "..."
    },
    "loaded": {
      "validated_profile": "<identificador-del-perfil-validado-por-el-loader>",
      "config_sha256": "..."
    }
  }
}
```

Los nombres son contrato de diseño; los tipos deben ser strings no vacíos con restricciones de formato para hashes/IDs donde el origen real las permita. Deserialización estricta del runner: keys exactas en cada nivel, tipos exactos, sin aceptar aliases desconocidos ni coerciones. Cada grupo debe estar presente y separado. Si un campo no puede generarse desde la ruta real de carga, no se emite un marcador que parezca válido: identidad incompleta y procedencia no verificable.

`artifact_bundle_id` identifica el bundle verificado que cargó el encoder; `model_id` y `revision` se leen del manifiesto pinned utilizado durante esa carga; `space_id` es el identificador derivado de ese manifiesto por `minilm_cpu_f32_v1_space_id`. `validated_profile` solo declara que `profile` aceptó los dos archivos de configuración consultados y solo puede emitirse con el identificador explícito y probado que se añada al loader. `config_sha256` es un fingerprint de esos bytes de configuración retenidos al cargar, no un digest de pesos, build, proceso ni memoria. No se expondrán rutas, secretos, variables de entorno, contenido de archivos, vectores o payloads.

La evidencia esperada del protocolo-run, inspect Docker y asociación request/run queda fuera del objeto self-report y se conserva como observación externa independiente. Ningún identificador inventado en la respuesta sustituye esa observación.

## Estrategia de pruebas TDD

Cada slice ejecutable comienza con una prueba focalizada que falla, seguida por la implementación mínima y refactor con pruebas verdes.

| Slice                                       | Prueba primero                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | Criterio de aceptación                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
|---------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Bridge: claves, auth y feature              | Probar el esquema completo de respuesta diagnóstica, solicitud sin token y ruta feature-gated; comprobar también build normal sin feature.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            | La identidad aparece solo junto a ANN/exact en la ruta existente autenticada; no aparece en respuesta no autenticada. Build normal no registra/serve una ruta H5. Ningún endpoint adicional.                                                                                                                                                                                                                                                                                                                                                                                            |
| Encoder: procedencia de la respuesta        | Con encoder de prueba que distinga identidad/codificación, hacer la consulta y verificar que identidad sale del mismo objeto usado para producir el embedding, no de constantes, entorno o caller.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    | Bundle, space/profile/config reportados proceden de la instancia de `App.encoder` utilizada por esa búsqueda. Los tests actuales de `EmbeddingPort` y encoder siguen compilando y pasando.                                                                                                                                                                                                                                                                                                                                                                                              |
| Fingerprint de carga                        | Validar el digest sobre bytes de fixture; alterar bytes de configuración cambia el digest y perfil inválido sigue fallando carga. Asegurar que el reporter no reabre el cache/paths.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Hash derivado durante load exclusivamente de buffers verificados que `profile` validó, retenido inmutable; no se presenta como estado de memoria/modelo.                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Runner: sanitizer/correlación               | Aceptar solo objeto allowlisted exacto y tipos esperados; rechazar keys adicionales, grupos fusionados, strings donde se requieren otros tipos, missing, malformed, caller substitution y mismatch. Correlacionar expected bundle e image/container/run observado con la respuesta ligada a consulta.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Correlación coherente puede registrarse solo como self-report observado; ausente, mismatch o run no correlacionado resulta `partial`/`insufficient` y nunca PASS. Caller no reemplaza respuesta ni inspect.                                                                                                                                                                                                                                                                                                                                                                             |
| Protocolo-run: esquema y validador estricto | Primero, añadir pruebas para cada campo de decisión: plantilla pending explícita, `value: null`, tipos por ruta, valores vacíos y placeholder, propiedades anidadas desconocidas y enumeración de categorías con JSON paths. Para `acceptance_rules_and_tolerances`, probar en Python y JSON Schema las cuatro combinaciones de gates: ambos habilitados, solo `ann_exact`, solo `absolute_relevance` y ambos deshabilitados. En las tres primeras, probar que `approved.value` exige exactamente una regla por gate habilitado y rechaza cualquier propiedad de un gate deshabilitado. En la última, probar que `pending` con `value: null` se acepta estructuralmente y bloquea readiness, sin exigir habilitar un gate; probar también que `approved.value` vacío sigue rechazado. | Ningún input arbitrario ni `{}` cuenta como presente. Se obtienen `missing`/`pending`/`malformed` separados y por ruta; cualquier categoría bloquea readiness. Las reglas solo aplican a gates habilitados; la pareja schema/validador aplica el mismo mapa condicional y rechaza reglas de gates deshabilitados. Con ambos gates deshabilitados no se inventa una regla ni se fuerza su habilitación: `pending` sigue permitido y no-ready, mientras que `approved` vacío permanece fail-closed. La aprobación requiere corroboración del operador contra hash del protocolo completo. |
| No PASS con evidencia incompleta            | Pruebas por gate con identity ausente/incompleta/contradictoria y con response/run no correlacionados.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | No existe camino de elegibilidad a PASS mientras la procedencia de identidad esté `partial` o `insufficient`; el faltante/discrepancia queda explicado en el informe.                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Compatibilidad y no regresión               | Ejecutar tests actuales del bridge y runner, incluyendo respuesta diagnostic existente, no-auth, alias projection, modos descriptivos y pruebas feature/no-feature.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Se conservan autorización, límites de input, `ann`/`exact`, formato de resultados no identidad y comportamiento normal. No hay llamada bridge adicional ni cambio de API pública normal.                                                                                                                                                                                                                                                                                                                                                                                                |

Las pruebas de runner deben mockear la llamada de búsqueda existente y afirmar exactamente una interacción de bridge en ese paso. La inspección Docker se modela como input externo separado, no como identidad declarada por el bridge. Las pruebas del protocolo deben contrastar JSON Schema y `validate_protocol` para que sus allowlists, tipos y aplicabilidad por gate coincidan, incluida la matriz de las cuatro combinaciones de gates para `acceptance_rules_and_tolerances` y las diferencias entre estado `pending` y `approved.value` vacío. Ningún test fija K numérico, reglas, umbrales, PASS, repetición/agregación, ties, tratamiento final de undefined, corpus, rubric/adjudication, evidencia operativa, protocolo concreto ni medición.

## Despliegue, compatibilidad y reversión

No hay migración de datos. La identidad nueva solo cambia la forma de respuesta de la ruta diagnóstica, que requiere la feature `h5-diagnostic`; la API normal y los builds sin feature permanecen intactos. Los clientes/fixtures del runner que exigen el formato anterior se actualizarán conjuntamente; la respuesta sin identidad se clasifica como procedencia incompleta, no se acepta como fallback PASS.

Si el fingerprint no se puede capturar desde los buffers cargados de forma segura, se deja ese campo sin soporte y se mantiene bloqueada la procedencia confirmatoria hasta que se diseñe una fuente sustentable. No se usará una lectura posterior del disco como sustituto. Reversión: retirar únicamente los campos de identidad y su consumo runner, manteniendo el comportamiento previo de búsqueda y la feature gate.

## Riesgos y tradeoffs

- El self-report depende del proceso/bridge y no resiste su compromiso; correlación externa mejora consistencia, no lo convierte en attestation.
- El `artifact_bundle_id` y `space_id` son datos disponibles en el encoder, pero no prueban por sí solos qué pesos continúan en memoria.
- El fingerprint de configuración cubre solo los bytes exactos de `modules.json` y `1_Pooling/config.json` que se leen y validan al cargar. No cubre todos los archivos del modelo ni dependencias o estado runtime.
- Informar un hash de archivos reabiertos después puede describir disco distinto del objeto cargado. Se prohíbe ese atajo; si no se retienen los bytes usados al cargar, el campo no es verificable.
- Incorporar identidad a una trait compartida puede afectar implementaciones y mocks de `EmbeddingPort`. Se debe preferir una extensión compatible (por ejemplo, método opcional con default no disponible) o encapsular capacidad en el objeto de App sin duplicar el encoder; comprobar todo implementador en tests antes de elegir.
- Esquema demasiado permisivo permitiría datos extra sensibles o identidad ambigua; esquema estricto puede clasificar respuestas antiguas como incompletas. En esta ruta de diagnóstico la compatibilidad se maneja bloqueando PASS, no relajando allowlist.
- La falta de expected identity o de evidencia Docker/request-run no se puede compensar con el self-report. Resultado `partial`/`insufficient`; nunca PASS.
- No se fijan valores de K, reglas, thresholds, número/agrupación de corridas ni protocolo. La medición confirmatoria permanece bloqueada hasta aprobación explícita de cada protocolo-run.
- Un contrato reusable de envoltura común reduce ambigüedad y duplicación, pero solo es seguro si cada `value` conserva un esquema específico por campo; un tipo genérico `object` permitiría contenido arbitrario. La alternativa de duplicar envolturas por campo maximiza especificidad local, a costa de divergencia entre reglas de estado. Se elige reutilizar la envoltura y especializar cada valor.
- El informe estructural detallado puede cambiar consumidores que esperen solo `missing_fields`; esa compatibilidad debe resolverse coordinadamente en implementación. No se debe conservar una interfaz antigua a costa de perder estados `malformed`/`pending` o sus rutas.
- La validación estructural prueba forma, presencia y estado declarado. No determina veracidad, aprobación humana, calidad de evidencia, cumplimiento de gates ni aceptación; un JSON estructuralmente válido puede continuar bloqueado.

## Preguntas abiertas

- Ninguna pregunta de diseño autoriza completar por suposición los campos de un protocolo-run. Los valores y aprobación de cada corrida permanecen pendientes según la especificación.
- En implementación se debe confirmar la superficie de compatibilidad más pequeña para exponer identidad a través del encoder sin duplicar instancias ni cambiar la API normal. Si no se puede hacer desde el objeto realmente usado, la identidad correspondiente no se reporta y la procedencia queda bloqueada.

## Updated Design: Candidate Protocol and Unlabeled Corpus

This section updates the design for the added candidate protocol and corpus preparation requirements. It takes precedence over earlier sections where they state that K, criteria, runs, the corpus, or undefined-value handling remain undecided. It does not change the encoder identity design or authorize measurements.

### Approach and Scope

Create two new versionable and reviewable artifacts: a candidate protocol that records the approved decisions for ANN–exact, and an independent, unlabeled candidate corpus in an explicitly draft state. Corpus creation is authorized only as preparation of repository material for review. It does not include a decision on sufficiency or representativeness, final owner review, freezing, labeling, measurement, or acceptance.

The candidate protocol sets `K=10`, one run, and requires both `ExactTopKRecall@10` and `OrderedTopKMatch@10` to be `1.0` for every query for the ANN–exact gate to pass. The report retains per-query values and may show macro averages only as a descriptive summary. Retrospective tie-breaking is not allowed. Any required undefined metric blocks `PASS`. The absolute-relevance gate remains `false`; no utility threshold is approved, and repository content is not converted into labels or measurements.

### Decision: Separate the Corpus from Its Provenance

**Choice**: Represent the corpus body in the exact form accepted by `retrieval_quality.validate_corpus`: `{schema_version, corpus_id, queries}`, with each query as `{id, text, candidates}` and each candidate as `{id, text}`. Store the `draft/not frozen` status, baseline commit, source-content hashes, and explicit mapping from each stable ID to its exact source path in a companion provenance manifest. Calculate `corpus_hash` using `retrieval_quality.corpus_hash` on the validated corpus JSON.

The companion manifest has a closed structure: `schema_version`, `corpus_id`, `corpus_hash`, `status`, `baseline_commit`, and `sources`. Each `sources` entry identifies `item_type` (`query` or `candidate`), `item_id`, the exact repository-relative `source_path`, and the SHA-256 of the source file bytes at the baseline commit. Reject IDs without an entry, duplicate entries, paths outside the eligible inventory, and hashes that do not match the baseline. Do not record owner review/freeze until an explicit decision exists; draft status must not imply approval.

**Alternatives considered**: Add `source_path`, hashes, or status metadata to the corpus JSON; store paths only in comments or free-form documentation. These are rejected because the validator rejects additional keys and traceability must be verifiable and reviewable without weakening the existing contract.

**Rationale**: The corpus remains consumable by the current validator, while a strict manifest can preserve provenance and status without contaminating the data schema being hashed. The hash identifies the canonical corpus content; the manifest links that content to the baseline and sources, and must also record the corpus hash to detect disagreement.

### Decision: Do Not Artificially Fill K=10 Coverage

**Choice**: Use only eligible material in versioned Markdown files under `docs/muninn/`, evaluated against inventory baseline `09bb21d5fd2c847ac7df5e48f7be06eb08cec445`. Exclude `docs/muninn/retrieval-h5-runner.md` because of its pre-existing modification. Also exclude all H5 pilot documents, data, and results; C0/C1 and hypothetical cases; and generated or temporary evidence. If any query has fewer than 10 eligible candidates, stop preparation of a usable corpus and record the blocker; never add filler candidates.

**Alternatives considered**: Reuse pilot/fixtures or lower the number of candidates to fit availability. These are rejected because they would break independence or contradict the approved K.

**Rationale**: The inventory of 29 versioned Markdown documents does not prove that sufficient eligible material exists or that the corpus is representative for K=10. Preparation cannot resolve this uncertainty through retrospective selection or irrelevant content. A candidate created before review remains explicitly draft, not frozen, unlabeled, and not representative.

### Preparation Flow

```text
baseline approved by the design
  └─ inventory only eligible and versioned sources
       └─ owner reviews query/candidate → source path mapping
            └─ draft IDs/text independent of the source passage
                 ├─ validate corpus with validate_corpus
                 ├─ calculate canonical corpus_hash
                 └─ write draft provenance manifest
                      └─ submit both artifacts for explicit review
```

The query must not copy text from its candidates or source passages; it must express an information need that allows retrieval to be assessed without revealing the answer. The owner reviews each source assignment and the content before any freeze decision. Stable IDs are opaque and unique within their scopes (`query_id` and `candidate_id`); they do not encode relevance judgments. Do not generate grades, labels, rankings, scores, or search results. If the inventory does not support queries with ten eligible candidates each, retain the blocker report, not a corpus presented as ready.

### Planned Change Files

| File                                                                                          | Action                        | Description                                                                                                                                                                                                                         |
|-----------------------------------------------------------------------------------------------|-------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `dev/muninn/fixtures/retrieval-h5-protocol.candidate.json`                                    | Create                        | Candidate protocol separate from the generic template. Records ANN–exact and absolute relevance `false`; leaves all unresolved operational inputs and approvals pending. It is not approvable or executable by itself.              |
| `dev/muninn/fixtures/retrieval-h5-corpus-candidate.json`                                      | Create, subject to coverage   | Independent, unlabeled corpus in a strict form validatable by `validate_corpus`; includes texts and stable IDs, not inline provenance. Create as a candidate corpus only if every proposed query has ten eligible candidates.       |
| `dev/muninn/fixtures/retrieval-h5-corpus-candidate.provenance.json`                           | Create                        | Closed companion manifest with `draft/not frozen` status, baseline commit, corpus hash, source hashes, and exact mapping of each query/candidate ID to the reviewed versioned path. Contains no labels or results.                  |
| `dev/muninn/tests/test_retrieval_h5_corpus_candidate.py`                                      | Create if automation is added | Verifies form with the existing validator, reproducible hash, provenance completeness, and fail-closed behavior for excluded sources, duplicate IDs, or fewer than ten candidates per query. Does not calculate or infer relevance. |
| `dev/muninn/retrieval_quality.py` and `dev/muninn/fixtures/retrieval-h5-protocol.schema.json` | No change planned             | Reuse existing contracts and validators; do not expand the corpus schema or add new acceptance logic to them.                                                                                                                       |

These names are proposed destinations for implementation artifacts, not claims that the files already exist. Do not modify the already-modified H5 runner or sources under `docs/muninn/`. The file table above supplements, and for these artifacts takes precedence over, the earlier planned-change table.

### Candidate Protocol Contract

The candidate file must retain the current protocol's closed structure. It must make explicit `K=10`, the per-query exact-equality criterion of `1.0`, one run, per-query reporting and descriptive macro averages, the no-retrospective-tie-breaking policy, and the undefined-value blocking policy. Absolute relevance must remain disabled, with no invented threshold, labels, or approval of that gate. Any field that the schema cannot express without changing the existing contract is recorded in the candidate artifact only through already-defined fields; the schema is not changed in this design phase. Candidate protocol status does not equal protocol-run approval, readiness, or execution authorization.

In the current decision wrappers, `metrics_and_k` records `ExactTopKRecall@K: 10` and `OrderedTopKMatch@K: 10`; the ANN–exact rule states that both metrics must be exactly `1.0` for every query; `run_count` is `1`; and aggregation states that per-query values govern while macro averages are descriptive. `tie_policy` specifies use of the observed ranking without retrospective tie-breaking. `undefined_policy` specifies that any required undefined metric blocks `PASS`. Set `gates.ann_exact` to `true` and `gates.absolute_relevance` to `false`. Do not fill encoder, environment, corpus freeze, or approval metadata with invented values. The artifact may record these approved decisions and remain not ready for a run.

The corpus and its manifest are linked by `corpus_id`, version, and `corpus_hash`. The freeze decision comes later: it requires the owner's review of all text and all source paths, followed by explicit approval of an exact version/hash. Without that decision, the protocol remains not ready. Future labeling is a separate phase after freeze and is not part of this change's artifacts.

### Artifact Verification

Verification is offline and deterministic: validate the corpus body with `validate_corpus`, recalculate and compare `corpus_hash`, check that every ID has a versioned source path from the baseline and that every source is in the eligibility allowlist, and check that there is no excluded material or label/result fields. Also verify that every query has ten or more candidates; any failure blocks the candidate corpus and must not be corrected by adding filler. Manually review that query text does not duplicate candidate/source passages and that the complete mapping is submitted to the owner. These checks confirm structure and declared provenance, not sufficiency, representativeness, relevance, freeze, readiness, or acceptance.

Verify the protocol's approved policy by structural inspection/validation: required K and thresholds, one run, the two per-query values, descriptive macro summary, no retrospective tie-breaking, blocking on undefined metrics, and disabled absolute relevance. Do not run retrieval or any measurement to verify these artifacts.

## Diseño actualizado: integración técnica offline ANN-only

### Enfoque técnico y código inspeccionado

Este slice implementará únicamente el contrato técnico de la enmienda final de la spec, con TDD sintético. No se ejecuta código en esta fase de diseño. No se convierte el candidato real a un protocolo listo ni se completa evidencia operativa.

El código actual ofrece límites reutilizables:

- `retrieval_h5_protocol.py`: `validate_protocol`, hallazgos deterministas, `canonical_protocol_hash` y `corroborate_approval`; actualmente reconoce solo v1.
- `retrieval_h5_runner.py`: `Bridge.post`, sanitización estricta de respuesta, `protocol_preflight`, serialización e integración de métricas. `run_evaluation` y `main` son el piloto descriptivo con CSV obligatorio. No constituyen un camino ANN-only.
- `retrieval_h5_provenance.py`: `LocalObservations`, sanitización, correlación y `assess_gate_provenance`. El baseline final `_COMMON` incluye resultados, identidad de respuesta y finalización. `collect_observations` requiere `finished_at` y no obtiene identidad loaded ni configuración efectiva; no debe usarse para inventar esas observaciones previas.
- `retrieval_quality.py`: `validate_corpus`, `corpus_hash`, `serialize_gate_metrics`, `evaluate_ann_exact_query` y `gate_result`. La serialización ANN ya llama a la fórmula correcta y conserva el orden recibido. `compose_report` requiere juicios, calcula relevancia y omite indefinidos de sus medias: no sirve para el informe ANN-only.
- El handler existente usa un mismo embedding para ANN y exact, proyecta aliases y devuelve identidad en la misma respuesta. ANN aplica top-K global antes del filtro de sesión. El piloto compensa con `max(limit, candidate_count)`; el slice nuevo no puede copiar ese ajuste silencioso.

No se propone refactor arquitectónico general. Se amplían los módulos existentes en sus límites actuales: protocolo puro, runner orquestador, procedencia y métricas puras. No se añaden dependencias, endpoint, Store ni cambios Rust para este slice.

### Decisión: separar selección explícita y transporte operativo

**Elección**: añadir `run_protocol_evaluation` como API Python distinta de `run_evaluation`. La nueva API recibe `Bridge` y un proveedor de observaciones inyectado. Las pruebas usan exclusivamente dobles. El slice no añade un comando que cargue un JSON de claims como `LocalObservations`, ni un selector que active `HttpBridge` para una medición real.

**Alternativas**: hacer CSV opcional dentro del piloto; añadir un modo implícito cuando falta CSV; crear ahora una CLI operativa con Docker. Se descartan por ambigüedad, regresión y falta de autorización operacional.

**Justificación**: dos entradas explícitas impiden que un protocolo inválido caiga al piloto. `main(argv)` y su CLI existente se preservan, con todos sus argumentos y salida descriptiva. La selección protocol-aware se hace por la nueva API en el harness offline. Una CLI operativa futura necesita diseño de adquisición independiente de evidencia y autorización; no está incluida aquí. No se presenta esta API como permiso para medir datos reales.

### Decisión: v2 cerrado, v1 estructural sin promoción

**Elección**: extender el validador existente con selección explícita por `schema_version`, reutilizando wrappers y validación recursiva. El esquema existente tendrá ramas v1/v2 discriminadas, con definiciones comunes y valores específicos por versión; no se mezcla `additionalProperties: false` de v1 con campos v2 mediante una extensión que vuelva imposible validar v2. V1 conserva la matriz estructural general de cuatro combinaciones; su validación nunca lo convierte en ejecutable. No hay migración ni herencia automática de aprobación.

V2 conserva las claves externas y añade únicamente los campos que fija la spec:

- `acceptance_rules_and_tolerances.value.ann_exact.policy_id`: `ann-exact-per-query-equality-v1`.
- `corpus_and_query_candidate_scope.value.corpus_hash`: patrón `^sha256:[0-9a-f]{64}$`.
- `ann_exact_comparison_scope.value.search_config`: objeto cerrado `limit` y `threshold`.

En v2 se validan exactamente las dos métricas con K entero 10, threshold finito 1.0, una corrida y estos identificadores literales:

```text
aggregation = per-query-governing-macro-descriptive-v1
tie_policy = observed-ranking-no-retrospective-tiebreak-v1
undefined_policy = required-undefined-blocks-pass-v1
retrieval_path = /h5/diagnostic/search
candidate_scope = same-query-candidates-v1
```

`search_config.limit` es entero de 10 a 100; `search_config.threshold` es número finito de 0 a 1. Python rechaza booleanos en todos los números y enteros. La suite de contrato comprueba la misma aceptación/rechazo en JSON Schema, incluida la representación numérica de enteros: el cargador no debe convertir booleanos ni normalizar tokens. No hay tolerancia, aliases de tokens ni defaults. `criterion` y `rationale` siguen siendo texto no vacío sujeto al validador existente; no se analizan para descubrir contradicciones semánticas. Una discrepancia humana detectada exige revisar el documento, no implementar un intérprete de prosa.

**Alternativas**: reconocer palabras del criterio v1; motor genérico de reglas; cambiar el candidato para simular aprobación. Se descartan porque romperían la vinculación entre documento revisado y política ejecutada.

**Justificación**: solo hay una política autorizada. Las restricciones se concentran en protocolo; el runner consume sus valores validados y no replica otro validador de reglas. El hash canónico sigue incluyendo todo el documento, incluso la prosa y la versión. Modificar cualquier campo o convertir a v2 invalida la corroboración previa.

### Decisión: preflight temporal sin relajar procedencia final

**Elección**: añadir en `retrieval_h5_provenance.py` una evaluación previa de requisitos disponibles, compartiendo la validación de registros con el evaluador actual. `assess_gate_provenance` mantiene su firma, todos sus requisitos y su clasificación final. No se le añade un flag público que permita al caller elegir un baseline reducido.

El nuevo `assess_preflight_provenance` no devuelve `verified` final: devuelve elegibilidad previa y faltantes/conflictos. Su baseline deriva del baseline final, separando expresamente:

- Posteriores obligatorios: `gate_results`, `encoder_response_identity`, `encoder_response_binding`, `finished_at`, `encoder_loaded_identity`.
- `effective_config`: antes se comprueban configuración aprobada y observaciones externas ya disponibles; la parte cuya fuente depende de respuesta queda pendiente. La configuración final completa continúa siendo obligatoria y no se completa copiando valores esperados.
- Previos restantes: hash/revisión/corroboración, corpus, fuentes runner/quality, imagen/container/binario, bundle, mounts/inspect, run y comienzo realmente observado. Deben corresponder a la misma ejecución. La evidencia de dependencias y entorno del requisito normativo debe estar acreditada en las observaciones externas de build/inspect/configuración, no en las descripciones de `provenance.value`.

El baseline y el chequeo de presencia/registro se comparten internamente; no se duplican las fórmulas de clasificación ni la correlación. Los registros conservan `{value, source, method, observed_at}` y `conflict` existente. La presencia de strings arbitrarios no prueba consistencia: el runner compara valores con protocolo/corpus/run y el adaptador identifica la fuente independiente. `LocalObservations` es un marcador del límite local Python, no una barrera contra código hostil ni una prueba criptográfica. No se eleva un `dict` de claims recibido del caller a ese tipo.

`protocol_preflight` existente permanece como reporte diagnóstico compatible, sin llamadas ni escritura y sin PASS. La nueva API usa un preflight de ejecución separado, que no llama a ese diagnóstico como requisito de autorización porque hoy exige finalización. El código común de validación/aprobación se reutiliza dentro del módulo; no se mantienen dos implementaciones divergentes de corroboración.

**Alternativas**: aportar valores ficticios para satisfacer `_COMMON`; quitar requisitos de `_COMMON`; exigir procedencia final `verified` antes de empezar. Se descartan por fabricar evidencia o hacer imposible la separación temporal.

**Justificación**: elegibilidad previa y procedencia final son hechos distintos. La falta prevista de evidencia futura permite solo iniciar el flujo sintético, nunca PASS. El collector actual puede seguir intacto; no se llama con un `finished_at` inventado.

### Interfaces Python y límites de confianza

Las siguientes firmas son contratos de implementación planificados, no código ya disponible:

```python
class ProtocolRunEvidence(Protocol):
    def before_run(self, *, protocol: dict, corpus: dict,
                   run_id: str) -> LocalObservations: ...
    def after_response(self, *, run_id: str, query_id: str,
                       diagnostic: dict) -> LocalObservations: ...
    def after_run(self, *, run_id: str) -> LocalObservations: ...


def run_protocol_evaluation(*, protocol: object, corpus: object,
                            operator_record: object, run_id: str,
                            bridge: Bridge, evidence: ProtocolRunEvidence,
                            search_config_override: dict | None = None) -> dict: ...
```

Este port pequeño permite probar el límite temporal sin Docker ni reloj real obligatorio; no oculta una implementación operacional nueva. El runner no entrega al proveedor identidades esperadas como prueba de identidad observada. `after_response` recibe solo el diagnóstico sanitizado, y sus datos observados deben distinguir la respuesta del bridge de evidencia externa independiente. Ningún callback reemplaza la aprobación comprobada con `corroborate_approval`.

La evidencia previa debe incluir un registro independiente de freeze/review con `corpus_id`, `version`, `corpus_hash`, estado congelado explícito y referencia de revisión, además del registro de hash de corpus existente. Este registro pertenece al adaptador de evidencia, no al cuerpo del corpus ni al protocolo; se valida su forma cerrada y su igualdad con el scope. Un manifiesto draft, la licencia MIT, el número de candidatos o un hash correcto no satisfacen freeze/review. La aprobación sintética de un test no modifica el candidato real.

La evidencia previa del scope debe acreditar un almacenamiento desechable aislado para ese run, sin candidatos ajenos, y el inventario esperado. Si el adaptador no puede acreditar aislamiento e inventario, se bloquea antes de insertar. Es una condición técnica de compatibilidad con top-K global, no autorización para crear/resetear una base real.

### Flujo de datos y coherencia de identidad

```text
API protocol-aware (sin CSV ni output)
  -> copia validada de protocolo/corpus + hash canónico
  -> validate_protocol v2 + corroborate_approval
  -> evidencia previa independiente + freeze/review + scope/config
  -> preflight: blocked | eligible (sin PASS ni verified final)
       blocked -> diagnóstico en memoria; cero Bridge.post y cero escrituras
       eligible -> insertar candidatos -> buscar una vez por consulta
         -> sanitize_diagnostic -> correlacionar respuesta/run/consulta
         -> serialize_gate_metrics(ann_queries, [], 10)
         -> evidencia final realmente observada
         -> assess_gate_provenance por consulta y cobertura total
         -> gate_result -> informe en memoria
```

El runner hace una copia de entradas al comenzar y comprueba nuevamente hashes y bindings antes del veredicto; mutaciones o evidencia asociada a otro documento/run bloquean. `corpus_hash` se calcula solo mediante el helper existente, sin algoritmo paralelo. El scope exige igualdad exacta de conjuntos de IDs, unicidad local y cobertura completa. La versión se corrobora externamente porque el cuerpo del corpus no tiene campo de versión. Al menos diez candidatos por consulta es una condición estructural; no acredita elegibilidad humana ni representatividad.

La configuración aprobada se envía sin ajuste de `limit` ni `threshold`. Se admite únicamente el caso conservador en que el total de inserciones del run no excede `search_config.limit` y el inventario aislado previo está acreditado. Así top-K global no elimina candidatos de otras consultas antes del filtro; si no se puede garantizar, `scope_unsupported` bloquea. No se baja K, no se rellena el corpus, no se modifica Store ni se reutiliza el aumento automático de límite del piloto.

Las sesiones se identifican por run y consulta con codificación inequívoca de la tupla, sin concatenación ambigua. El mapa de aliases pertenece a `(query_id, candidate_id)`; cada consulta guarda exclusivamente sus UUID devueltos por inserción. No se filtra un mapa global por candidate ID como hace el piloto: un ID repetido entre consultas no mezcla aliases. Se rechazan UUID duplicados en todo el run, resultados fuera del mapa, aliases/ranks duplicados, resultados extra o respuesta inválida. No se exponen UUID ni textos en el reporte público.

Para cada consulta existe una sola respuesta diagnóstica y una sola llamada de búsqueda. No hay petición adicional de identidad, reordenamiento por score, desempate ni reintento selectivo. El binding se registra alrededor de la solicitud efectuada; no se atribuye al bridge un run ID que no devuelve. Se conserva una evaluación de procedencia por consulta; no se sobrescribe toda la corrida con la identidad de la última respuesta. Una consulta no correlacionada bloquea el gate completo.

`encoder_loaded_identity` o `effective_config` finales sin fuente suficiente permanecen faltantes. La coincidencia entre `encoder_response_identity` y la identidad esperada solo produce self-report correlacionado; no llena automáticamente los otros registros ni eleva su confianza. El camino sintético positivo requiere observaciones falsas explícitas de todas las fuentes finales necesarias. En producción esas fuentes siguen siendo una limitación y este slice no las inventa.

### Informe, estados y errores

Se conserva la forma `protocol`, `gates` y `measurement_started` del informe existente, con contexto adicional de corpus/run y política. `absolute_relevance` es siempre `{status: "not_evaluated", results: {}}`; no se calcula ni serializa un informe de relevancia. Se llama al serializador con `quality_queries=[]` y se integra solo su rama ANN, reutilizando `serialize_protocol_gate_metrics`/`integrate_protocol_gate_metrics` donde corresponda. No se llama a `compose_report`, `annotations_from_csv` ni `evaluate_query`.

ANN conserva `results.k`, `results.per_query` y nombres existentes `exact_top_k_recall`/`ordered_top_k_match`. Puede añadirse resumen `macro_descriptive` con valor y conteo de indefinidos por métrica: media aritmética de todas las consultas solo si todas están definidas; de lo contrario `null`, sin omitir consultas. Es agregación nueva acotada de valores ya calculados, no otra implementación de métricas ni reutilización de la media del piloto que omite indefinidos.

El runner aporta a `gate_result` un único `rule_satisfied`: `None` si falta cobertura o cualquier métrica requerida es indefinida; `False` si todas están definidas y alguna no es exactamente 1.0; `True` solo si todas cumplen. Cualquier fallo de protocolo/aprobación/procedencia prevalece y produce `blocked`. Se conservan por separado razones de criterio y procedencia; nunca se devuelve aceptación global H5. La salida del harness identifica un PASS como prueba sintética del contrato, no resultado real.

Los rechazos esperados se devuelven como `blocked` con `findings` seguros `{path, reason}` y `measurement_started=false` si fallan antes de efectos. Categorías estables del runner: `protocol_invalid`, `contract_unsupported`, `approval_mismatch`, `corpus_mismatch`, `scope_unsupported`, `configuration_mismatch`, `preflight_evidence_missing_or_conflicting`, `response_invalid`, `response_binding_mismatch`, `coverage_incomplete`, `provenance_incomplete`, `required_metric_undefined`. Los hallazgos estructurales `missing/pending/malformed` se preservan sin reemplazarlos por una excepción genérica. No se incluyen entrada cruda, tokens, rutas sensibles ni textos en errores.

Tras el primer efecto, una excepción de transporte/inserción/sanitización produce un informe bloqueado con cobertura parcial y `measurement_started=true`; no reintenta ni intenta rollback destructivo del almacenamiento. Se conserva lo observado, sin inventar finalización exitosa. Los errores inesperados de programación no se convierten silenciosamente en éxito.

### Escritura y fallo de persistencia

La API nueva devuelve el informe en memoria y no acepta `output`, ni crea directorios/archivos. Este límite garantiza cero escrituras de medición en todo preflight bloqueado, incluso si existe un output anterior. El harness captura informes en memoria. No hay archivo de PASS antes de completar evaluación final.

La CLI piloto conserva su manejo actual: `OSError` durante `mkdir`/`write_text` produce código 2 y mensaje de rechazo, sin afirmar persistencia exitosa. Su escritura actual no es atómica; este slice no la refactoriza ni afirma que preserve un output si falla tras truncarlo. No se reutiliza ese escritor para protocol-aware. Si una integración futura persiste un informe protocol-aware, necesita publicación atómica solo después de preflight/ejecución y debe informar fallo de escritura sin publicar éxito; esa integración no forma parte de esta API offline ni se deja implícita en apply.

### Archivos afectados en el slice nuevo

- Modificar `dev/muninn/retrieval_h5_protocol.py`: validación v2 específica, selección por versión y reutilización de hash/aprobación existentes.
- Modificar `dev/muninn/fixtures/retrieval-h5-protocol.schema.json`: ramas cerradas v1/v2, contratos y condicionalidad coherentes con Python.
- Modificar `dev/muninn/retrieval_h5_provenance.py`: evaluación temporal previa con validación común de registros; mantener baseline final y collector existente sin fabricar finalización.
- Modificar `dev/muninn/retrieval_h5_runner.py`: API nueva, port de evidencia, preflight, scope seguro, bindings por consulta, integración de métricas y veredicto acotado. Preservar WIP, piloto y CLI.
- Modificar `dev/muninn/tests/test_retrieval_h5_protocol.py`, `test_retrieval_h5_provenance.py` y `test_retrieval_h5_runner.py`: contrato v2, temporalidad y flujo con dobles.
- Modificar posteriormente `docs/muninn/retrieval-h5-runner.md` mediante edición selectiva: documentar API offline, CLI piloto intacta y límites de evidencia; preservar su WIP.
- Sin cambios previstos en `retrieval_quality.py`, sus fórmulas/tests actuales, Rust/bridge, Docker/Compose, C0, corpus candidato, manifiesto, protocolo candidato v1, plantilla incompleta, resultados o informes históricos. Las fixtures v2 completas se construyen dentro de tests y no promueven artefactos reales.

### Estrategia RED → GREEN → Refactor

Apply comienza con tests fallidos, no con implementación. Cada grupo termina con regresión focalizada:

1. **Contrato v2**: fixture sintética completa; probar versión, tokens exactos, campos extra, pending/missing, K/threshold/run distintos, booleanos, NaN/infinito, discrepancias schema/Python y las cuatro combinaciones generales v1. V1 válido sigue estructural, pero falla la elegibilidad ejecutable. Prosa distinta no cambia la regla; cualquier cambio del documento invalida su aprobación/hash.
2. **Temporalidad**: preflight válido sin registros futuros; falta/conflicto de cada requisito previo bloquea. Spy bridge y spies de filesystem prueban cero búsquedas/inserciones/creación/truncamiento, con output centinela intacto. No invocar `collect_observations` con finalización ficticia. Baseline final conserva todos los registros originales.
3. **Corpus/config/scope**: hash de texto alterado con mismos IDs, versión/freeze discrepante, consultas omitidas/extra, IDs duplicados, menos de diez candidatos, límite insuficiente, inventario contaminado, overrides discrepantes. Candidate ID compartido entre consultas usa mapas separados. Ningún caso ajusta el límite ni usa fallback piloto.
4. **ANN-only positivo**: corpus generado de al menos dos consultas con diez candidatos cada una y configuración sintética compatible; bridge/evidencia falsos independientes. Parchear lectura/validación CSV y helpers de relevancia para que fallen si se invocan. Verificar una corrida, exactamente una búsqueda por consulta, identidad en la misma respuesta, dos métricas 1.0, relevancia vacía y PASS exclusivamente sintético.
5. **Resultado/procedencia adversos**: orden distinto con recall completo produce FAIL; exact vacío bloquea con dos `null`; underfill usa disponibles; ties preservan orden y scores no gobiernan. Falta/conflicto en cada evidencia final, binding de otra consulta/run, identidad inválida, cobertura incompleta o respuesta extra/duplicada bloquean incluso con métricas perfectas. Resúmenes no redondean ni eliminan indefinidos. Fallo de bridge deja bloqueo parcial, sin reintentos.
6. **No regresión**: tests descriptivos existentes, CLI requerida intacta, informes históricos no mutados y `protocol_preflight` diagnóstico sin PASS. Un fallo de escritura piloto conserva retorno de error, sin claim de publicación exitosa.

Comando unitario de discovery existente, para ejecutar solamente en apply/verify con sus dependencias disponibles, no en esta fase:

```sh
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=dev/muninn python3 -m unittest discover \
  -s dev/muninn/tests -p 'test_retrieval*.py'
```

La suite actual de protocolo invoca `jsonschema` por subprocess para contrastar el schema; ese requisito se conserva y no se oculta si falta. Los nuevos tests de flujo usan `unittest` y dobles en memoria, sin modelos, Docker, bridge vivo, DB ni mediciones. No se atribuye una corrida RED/GREEN a este diseño: aquí solo se inspeccionaron archivos.

### Compatibilidad, rollout y riesgos abiertos

No hay migración de datos. El despliegue de este slice es la API offline y sus tests, después de `sdd-tasks`; no activa una medición. Revertir solo esa integración y la rama v2 mantiene piloto, WIP y evidencia histórica, sin borrar datos ni artefactos. Tasks debe separar contratos, temporalidad y orquestación en unidades revisables y evaluar el presupuesto de 400 líneas antes de apply.

No hay una decisión humana de K, política o threshold de utilidad que deba inventarse: la spec ya fijó los valores ejecutables del slice. Permanecen pendientes las decisiones reales de corpus/freeze/review, parámetros operativos reales y aprobación de protocolo-run. La capacidad de obtener evidencia independiente de loaded/configuración y de aislamiento real no está resuelta por un fake; si falta, el flujo real continúa bloqueado. El marcador Python `LocalObservations` y un self-report consistente no son atestación. El soporte conservador de scope puede rechazar configuraciones válidas para otros experimentos: se prefiere bloquear a cambiar silenciosamente límites o filtros. QA histórico y Clippy permanecen bloqueados; los tests futuros solo comprobarán el contrato técnico offline.

## Diseño actualizado U9: preflight cerrado y parada antes de efectos

### Vigencia, alcance y evidencia de inspección

Esta sección aplica la «Enmienda U9: contrato cerrado de evidencia previa y salida sin efectos» de `specs/retrieval-acceptance/spec.md` (líneas 675–850 al redactar). Prevalece sobre las secciones anteriores únicamente respecto de registros previos especializados, bindings y salida intermedia U9. Conserva WIP, historial y U7/U8 implementadas y revisadas. No sustituye la ejecución futura U10 ni los requisitos finales U11. Esta fase modifica solo diseño; no implementa U9 ni ejecuta comprobaciones de código.

El código inspeccionado permite integración local, sin un framework nuevo:

- `retrieval_h5_runner.py:69–70` define `Bridge`; `256–328` implementa el diagnóstico histórico `protocol_preflight`. Este último usa procedencia final y sustituye identidad esperada en sus observaciones: no se reutiliza como autorización nueva ni se modifica.
- `retrieval_h5_protocol.py:170–289` valida v1/v2, devuelve hash completo y hallazgos agrupados; `292–317` contiene la única comprobación temporal de corroboración que necesita U9. La validación numérica v2 admite representaciones integrales finitas como `10.0`, sin aceptar booleanos.
- `retrieval_h5_provenance.py:184–222` comprueba presencia/metadatos y elegibilidad previa. No valida valores semánticamente, no incorpora los dos registros nuevos y no prueba independencia de fuentes. `225–262` conserva el baseline y clasificación final.
- `retrieval_quality.py:35–75` valida el cuerpo cerrado y calcula su hash. Los IDs repetidos entre consultas requieren el mismo texto, aunque cada par sea una inserción distinta.
- Los tests de procedencia actuales usan valores polimórficos deliberados. Los fixtures estructurales v1 de runner no se convierten en entradas ejecutables U9.

### Decisión: validación especializada privada en el runner

**Elección**: implementar los chequeos semánticos como helpers privados puros de `retrieval_h5_runner.py`, junto al nuevo orquestador. Reutilizar `validate_protocol`, `corroborate_approval`, `validate_corpus`, `corpus_hash`, `sanitize_encoder_identity` y `assess_preflight_provenance`; no cambiar sus contratos. Para strings de decisiones, reutilizar el predicado existente `_is_placeholder` del módulo de protocolo como dependencia interna explícita, sin ampliar su API pública ni implementar otra lista de placeholders. Los helpers especializados no interpretan prosa.

**Alternativas**: endurecer `_check_records` para todos sus consumidores, añadir los registros nuevos a `_COMMON` o crear un validador genérico nuevo. Se descartan: alterarían U8 y la clasificación final o ampliarían innecesariamente el alcance.

**Justificación**: los dos registros nuevos son condiciones de autorización/scope de esta ruta; la evaluación genérica U8 debe conservar su polimorfismo y sus 16 requisitos previos. El éxito requiere simultáneamente comprobación U8 y validación especializada, no una u otra. `assess_gate_provenance` y `collect_observations` quedan intactos; no se fabrica `finished_at` para invocarlos.

Los helpers privados se organizan por responsabilidad, sin clases o módulos nuevos obligatorios:

- Validar envoltura y claves/tipos de valores especializados.
- Comparar corpus/scope, freeze, configuración, plan y bindings de evidencia.
- Crear findings seguros y construir el informe intermedio.

No se importa el validador recursivo de decisiones para aceptar registros arbitrarios: su regla de arrays no vacíos sería incorrecta para `observed_candidates=[]`. La lista vacía se valida expresamente dentro del objeto `storage_scope.value`, que sí es no vacío.

### Interfaces conservadas y frontera temporal

Se mantienen las firmas ya diseñadas, sin selector operacional adicional:

```python
class ProtocolRunEvidence(Protocol):
    def before_run(self, *, protocol: dict, corpus: dict,
                   run_id: str) -> LocalObservations: ...
    def after_response(self, *, run_id: str, query_id: str,
                       diagnostic: dict) -> LocalObservations: ...
    def after_run(self, *, run_id: str) -> LocalObservations: ...


def run_protocol_evaluation(*, protocol: object, corpus: object,
                            operator_record: object, run_id: str,
                            bridge: Bridge, evidence: ProtocolRunEvidence,
                            search_config_override: dict | None = None) -> dict: ...
```

U9 llama `before_run` exactamente una vez solo después de validar las entradas locales y la corroboración. El callback recibe copias separadas del protocolo/corpus para que no pueda modificar la copia de trabajo del runner. Se exige `isinstance(result, LocalObservations)` antes de copiar las observaciones para inspección; un dict plano no se promueve. El proveedor conserva la responsabilidad de adquisición independiente. Recibir las expectativas como argumentos no le permite devolverlas como si las hubiera observado.

`bridge` se acepta para mantener la firma futura, pero U9 no lo utiliza. `after_response` y `after_run` forman parte del port futuro y no se invocan en U9. No se añade `preflight_only`, comando, CLI, adaptador Docker ni mecanismo para cargar claims JSON. El proveedor offline no escribe ni interactúa con el bridge. El marcador Python y las copias protegen el flujo cooperativo, no son aislamiento frente a código Python hostil.

### Contratos especializados cerrados

Los cuatro registros tienen exactamente `value`, `source`, `method`, `observed_at`, y opcionalmente `conflict`. Las claves de objetos deben ser strings. Los tres metadatos deben ser strings con contenido tras comprobar `strip()`, pero se conservan sin transformar. `conflict` ausente o `False` es admisible; cualquier otro tipo o `True` bloquea. No se analiza timestamp, antigüedad ni orden de `observed_at`.

Todos los objetos `value` se cierran recursivamente. IDs/referencias son strings no vacíos ni placeholders. Se compara el contenido original, sin trim, case folding, aliases, defaults ni coerción. Los hashes de corpus usan full-match de `sha256:` y 64 hexadecimales minúsculos, sin sufijos. Los campos no previstos y los tipos no JSON se rechazan antes de acceder o comparar su contenido.

#### `corpus_freeze_review.value`

Claves exactas: `corpus_id`, `version`, `corpus_hash`, `status`, `review_reference`.

- `status` es exactamente `frozen`.
- ID/versión/hash coinciden con `corpus_and_query_candidate_scope.value`; ID y hash coinciden también con el corpus validado y el hash del helper.
- La referencia sustenta revisión explícita de contenido y source paths para esa versión/hash. Puede diferir de la revisión del protocolo: no se impone igualdad entre ambas referencias.
- No se añade run ID al registro ni metadatos al cuerpo del corpus. El scope incluido en el hash completo vincula corpus y protocolo; los otros registros vinculan la llamada con el run.
- Draft, referencia pendiente, hash correcto sin revisión o cantidad suficiente de candidatos no satisfacen freeze.

#### `storage_scope.value`

Claves exactas: `run_id`, `bridge_container_id`, `storage_id`, `isolation`, `observed_candidates`, `planned_candidates`.

- Los tres IDs son strings válidos; `storage_id` es opaco, no una ruta de host.
- `isolation` es exactamente `run-exclusive-disposable-v1`.
- `observed_candidates` es exactamente una lista vacía y representa el inventario global completo observado antes de inserciones. No admite inventario filtrado por consulta, sesión o corpus; candidatos ya existentes, incluso del corpus aprobado, bloquean.
- `planned_candidates` es lista no vacía de objetos exactos `{query_id, candidate_id}`. Se comprueban tipos y duplicados antes de formar conjuntos. Los pares coinciden con todos los del corpus y del scope, sin extras/omisiones. Su orden no modifica ranking.
- El total de pares es el número de inserciones previsto y debe ser ≤ `search_config.limit`. Un candidate ID compartido no reduce ese total; cada par cuenta una vez.
- Run y container coinciden con los argumentos/registros externos indicados abajo. No se generan UUID, sesiones ni inserciones en U9.

Un inventario vacío y un token de aislamiento no demuestran exclusividad durante la corrida. El contrato del proveedor exige observación independiente de almacenamiento desechable asignado exclusivamente al run y ausencia de escritores ajenos durante su uso. U9 valida forma y consistencia; no inventa un mecanismo de prueba operacional ni consulta Store. Sin una fuente suficiente, el proveedor no debe emitir evidencia satisfactoria y el flujo permanece bloqueado. No se crea ni limpia almacenamiento para satisfacer el requisito.

#### `effective_config.value` previo

Claves exactas: `run_id`, `retrieval_path`, `candidate_scope`, `search_config`; este último contiene exactamente `limit` y `threshold`.

- Ruta `/h5/diagnostic/search` y scope `same-query-candidates-v1`, iguales al protocolo.
- `limit` tiene dominio entero JSON entre 10 y 100, coherente con U7: entero o representación float finita integral, excluyendo booleanos, sin convertir el token recibido. `threshold` es número finito entre 0 y 1, sin booleanos. Comprobar rango antes de conversiones que puedan desbordar, como ya hace U7.
- Los valores coinciden numéricamente con la configuración aprobada después de validar tipos/dominios; no se usa igualdad Python sin esa validación porque `True == 1`. No se normalizan valores para aceptar tipos incompatibles.
- El override, cuando no es `None`, exige exactamente las mismas claves, dominios y valores. No se ajusta el límite según el corpus ni se copia el comportamiento piloto.
- Es configuración externa previa, no prueba de ejecución ni configuración loaded completa. No completa identidad loaded, configuración dependiente de respuesta o procedencia final.

#### `operator_corroboration.value`

Claves exactas existentes: `protocol_sha256`, `commit`, `review_reference`, `corroborated_at`, `corroborated_by`. Se comprueba igualdad exacta con la copia de `operator_record` recibida. La decisión de aprobación se obtiene exclusivamente mediante `corroborate_approval(protocol, expected_sha256=validated["sha256"], operator_record=operator_record)`. No se replica parsing de fechas ni comparación de aprobación/corroboración. La envoltura no añade run, corpus ni nuevos campos al objeto del operador.

### Bindings y orden de evaluación

```text
run_protocol_evaluation
  -> copias de entradas; validar run ID no vacío/no placeholder
  -> validate_protocol: findings + hash canónico completo
  -> exigir v2, ANN=true, relevancia=false, readiness
  -> corroborate_approval con operator_record original copiado
  -> validate_corpus + corpus_hash; comparar scope exacto y ≥10 por consulta
  -> validar override y total previsto ≤ limit aprobado
  -> evidence.before_run con copias independientes (una llamada)
  -> exigir LocalObservations; copiar sin promover claims
  -> assess_preflight_provenance("ann_exact", observaciones)
  -> validar los cuatro registros cerrados + bindings semánticos
  -> blocked | eligible; informe en memoria; return (fin U9)
```

Un fallo local anterior a `before_run` retorna sin invocarlo. Tras recibir evidencia se reúnen los hallazgos seguros disponibles, sin acceder a objetos que no pasaron su validación de forma. No es necesario invocar callbacks posteriores para completar un fallo. Una excepción esperada de adquisición (`OSError`, `ValueError`, `RuntimeError`) bloquea con razón fija; no se incluye su mensaje crudo. Errores inesperados de programación no se convierten en éxito ni se silencian mediante captura indiscriminada.

Comparaciones obligatorias, con valores originales preservados:

- `protocol_hash.value` = `validated.sha256` = `operator_record.protocol_sha256`.
- `review_reference.value` = revisión del protocolo = `operator_record.review_reference`; commit del operador = commit del protocolo.
- `operator_corroboration.value` = copia de `operator_record` corroborado.
- `corpus_hash.value` = hash calculado = hash del scope = hash de freeze. ID/versión de freeze corresponden al scope; ID del corpus también corresponde.
- `encoder_expected_identity.value`, validado por `sanitize_encoder_identity`, = identidad aprobada del protocolo. No se sustituye un valor ausente/discrepante por el esperado como hace el diagnóstico histórico.
- `run_id.value` = argumento de la API = run de `storage_scope` = run de `effective_config`.
- Container de `storage_scope` = `bridge_container_id.value` externo. No se atribuye un run ID al bridge ni al protocolo.
- Configuración externa previa = configuración aprobada; plan = pares del corpus = pares del scope; total ≤ límite.

Los conjuntos de scope solo se forman después de comprobar unicidad local de consultas/candidatos y forma de listas; el hash comprueba textos alterados con IDs iguales. `validate_corpus` conserva su restricción sobre texto consistente para IDs compartidos. La igualdad de conjuntos no sustituye comparación de hash ni revisión humana.

Los demás registros previos de build/inspect, dependencias, entorno, bundle, fuentes y comienzo observado siguen sujetos al baseline U8 y al contrato de adquisición independiente existente. U9 no define valores operativos nuevos para ellos ni convierte textos `observed` de fixtures antiguas en evidencia independiente suficiente.

### Findings: categorías, paths y datos públicos

Los findings del runner usan exactamente `{path, reason}`. `reason` es una categoría fija ya prevista; no incluye valores recibidos ni mensajes de excepciones. Se ordenan por `(path, reason)` y se eliminan duplicados exactos. Los hallazgos estructurales del protocolo se conservan además sin pérdida en `protocol.findings.missing/pending/malformed`, con sus paths/razones originales validadas. Un error de forma que impida `validate_protocol` produce `protocol_invalid` en `protocol`, sin interpolar el error.

Mapa de categorías U9:

- `protocol_invalid`: protocolo ausente/malformado/no-ready; los grupos estructurales detallan missing/pending/malformed. Path `protocol` para error global o path del validador para decisiones incompletas.
- `contract_unsupported`: versión o gates fuera del slice; paths `protocol.schema_version` o `protocol.gates`.
- `approval_mismatch`: helper de corroboración falso (`operator_record`), objeto observado distinto (`observations.operator_corroboration.value`) o hash/revisión discordantes (path del registro).
- `corpus_mismatch`: cuerpo/hash/scope de corpus o freeze incorrectos; paths `corpus`, `protocol.run_inputs.corpus_and_query_candidate_scope.value`, `observations.corpus_hash.value` o subcampo de `observations.corpus_freeze_review.value`.
- `scope_unsupported`: contaminación, aislamiento, plan, pares duplicados o total incompatible; subcampo de `observations.storage_scope.value`, o `protocol.run_inputs.ann_exact_comparison_scope.value.search_config.limit` si el total local ya excede el límite.
- `configuration_mismatch`: override/configuración discrepante; `search_config_override` o subcampo de `observations.effective_config.value`.
- `preflight_evidence_missing_or_conflicting`: falta, envoltura malformada, conflicto, tipo de observaciones inválido, fallo de adquisición o binding previo sin categoría más específica. Paths `observations`, `observations.<nombre-conocido>` o subcampo fijo. Identidad esperada inválida/discrepante usa `observations.encoder_expected_identity.value`; run discordante usa el path de la observación discrepante.

La forma inválida de cualquiera de las cuatro envolturas usa `preflight_evidence_missing_or_conflicting`; las discrepancias de su valor bien formado usan la categoría especializada anterior. Por ejemplo, propiedad extra de freeze señala el objeto `observations.corpus_freeze_review.value` y no reproduce el nombre desconocido. Para extras o claves no string se señala siempre el objeto contenedor, nunca claves arbitrarias que puedan contener secretos. Los índices de listas son enteros de posición, no IDs introducidos por el caller. Los faltantes de U8 se convierten en paths de nombres conocidos, no se copian observaciones crudas a findings.

### Informe intermedio U9

Resultado positivo mínimo, sin procedencia final:

```json
{
  "preflight": {"status": "eligible", "findings": []},
  "gates": {
    "ann_exact": {"status": "not_evaluated", "results": {}},
    "absolute_relevance": {"status": "not_evaluated", "results": {}}
  },
  "measurement_started": false
}
```

El rechazo cambia `preflight.status` a `blocked`, añade findings no vacíos y cambia únicamente ANN–exact a `{status: "blocked", results: {}}`. Relevancia sigue `{status: "not_evaluated", results: {}}`. No se llama a `gate_result` para producir el estado intermedio: ese helper representa evaluación final y devolvería bloqueo por regla aún no evaluada, confundiendo elegibilidad con veredicto.

Se adjuntan como contexto hermano seguro:

- `protocol`: hash calculado, versión y schema_version conocidos, booleanos de readiness/corroboración y hallazgos estructurales. Si el documento no pudo validarse, esos valores no disponibles quedan `null`; nunca se inventa un hash exitoso.
- `corpus`: ID, hash calculado y versión del scope solo si se validaron; en otro caso `null` para el valor no disponible.
- `run_id`: argumento validado, o `null` si era inválido.

No se serializan las observaciones completas ni UUID, textos, metadatos arbitrarios del proveedor o configuración loaded inventada. `preflight` y cada gate conservan sus claves exactas. Puede omitirse el detalle de elegibilidad U8 del reporte público: no hace falta incluirlo para preservar su resultado como condición interna. No hay `overall_status`, PASS ni aceptación global.

U9 retorna aquí tanto en éxito como en fallo: cero `Bridge.post`, `after_response`, `after_run`, métricas, CSV, helpers de relevancia, creación de directorios o escrituras. El archivo centinela permanece intacto; la API no acepta `output`. Tampoco efectúa rollback o reset. Este punto de parada es una entrega incremental del contrato, no un modo público nuevo.

### Extensión posterior y plan de archivos

U10 extenderá únicamente la rama elegible después de este preflight para insertar y buscar con los bindings ya diseñados. No introduce flag `preflight_only` ni nueva CLI; U9 no implementa anticipadamente esa continuación. U11 conservará configuración final completa, loaded, resultados, tiempos, binding por consulta y baseline final sin reducción. La salida elegible U9 no se reutiliza como evidencia de ejecución o veredicto.

Para apply U9 se prevén únicamente cambios focalizados en `retrieval_h5_runner.py`, `tests/test_retrieval_h5_runner.py` y documentación técnica pertinente mediante edición selectiva. No se necesitan cambios de protocolo/schema, corpus, candidatos, U8, Rust, Docker, C0 o informes históricos. Las tareas 14.1–14.3 deben leerse con esta parada positiva explícita y los contratos cerrados; si el orquestador actualiza su redacción, debe mencionar esa frontera sin marcar casillas ni añadir U10 a U9.

### Pruebas futuras y cierre del contrato

TDD de U9 comienza con fixtures v2 independientes en tests, dos consultas de al menos diez candidatos y proveedor falso con registros estructurados explícitos. Las fixtures no se generan copiando expectativas dentro de `before_run` ni completan el candidato real. El test positivo debe detenerse sin efectos; los tests de PASS y búsquedas quedan para U10/U11.

Cubrir en RED, luego GREEN y Refactor:

- Cada campo obligatorio, extra, tipo inválido, placeholder, conflicto y binding incorrecto de los cuatro registros, preservando inputs.
- Freeze ausente/draft/otra versión/hash; revisión de corpus distinta de revisión del protocolo aceptada cuando corresponde.
- Inventario global contaminado, plan extra/omitido/duplicado, candidate ID compartido y total contado por pares; límite insuficiente sin ajuste.
- Configuración exacta, override discrepante, booleanos/no finitos/enteros enormes y representaciones integrales coherentes con U7, sin coerción.
- Corroboración del helper existente, observación distinta del operador, mutación del documento y rechazo de dict plano; sin nueva política temporal para metadatos de observación.
- Informe positivo/negativo exacto, findings deterministas y seguros ante claves extra con nombres sensibles; cero llamadas a bridge/callbacks posteriores/escritores/métricas con spies y centinela intacto.
- Compatibilidad histórica `protocol_preflight`/piloto y polimorfismo U8 sin cambios; los requisitos finales siguen completos.

No se ejecutaron esas pruebas en diseño. El contrato técnico U9 queda definido para reanudar apply TDD después de la comprobación del orquestador; no queda una decisión humana nueva de nombres o formato. Freeze real, aprobación de protocolo-run, evidencia operacional de aislamiento/configuración y aceptación H5 siguen pendientes. Un snapshot vacío o un token coherente no resuelve esas limitaciones ni convierte fuentes falsas de tests en autoridad.
