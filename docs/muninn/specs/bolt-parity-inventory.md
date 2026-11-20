# Inventario candidato de paridad funcional sobre Bolt

Primer hito ([RPI-001][rpi]). La comparación es entre el comportamiento público de
Agent Memory configurado con el backend Bolt y el de Muninn sobre Neo4j. Se
comparan retornos, errores y efectos persistidos; no la sintaxis de Python o
Cypher. Este inventario no fija el denominador ni puntúa la paridad.

## Fuentes y tipos de evidencia

- **Documental:** el [TCK normativo][spec] está fijado a
  `b6c72c36bb7486881b87053ee1610f2caf972fb6`. Su `SPEC.md` indica qué exige
  cada cláusula; el código de cada caso indica qué se comprobó realmente.
  Para `SPEC-5.x`, identificar también volumen y módulo: Gold (referencias
  entre memorias) y Platinum (servicio alojado) reutilizan IDs.
- **Estática:** el checkout local de referencia coincide con
  [Agent Memory `9d45af55`][upstream]
  (`9d45af5529ecb5f2fe352013468a11087f3163d4`). Los enlaces públicos remiten a
  ese commit. Usar [api-profile.csv](api-profile.csv), con `backend=bolt`, para
  seguir `operation` → `binding_source` → `implementation_source` y distinguir
  código activo, declaraciones, condicionales y rechazos explícitos. Una firma
  o un cuerpo no demuestran ejecución; sus estados no son un veredicto actual
  sobre Muninn.
- **Oráculo:** ejecuciones de la referencia Python. La [línea base del
  oráculo](../reference-tck.md) distingue `adapted` y `native-delete`. El
  adaptador reemplaza el borrado e inyecta componentes controlados; sus
  resultados no prueban semántica nativa ni resultados de Muninn.
- **Producto:** recibos auditados por perfil y caso. En
  [tck-baseline.csv](tck-baseline.csv), usar `profile`, `test_id`, `clauses`,
  `source_ref` y `gap_id` para seguir la evidencia histórica y los perfiles
  `product-b6c7-*`. Confirmar la cláusula en el pin normativo si la celda está
  vacía o la fuente apunta al pin histórico `e60b410f`. Silver38 y Gold18 se
  consultan en los recibos RPI nuevos, no se atribuyen a las filas `adapted`.
- **Faltante:** ausencia de evidencia suficiente para el criterio indicado.
  Un caso no seleccionado, bloqueado u omitido no es un aprobado ni un fallo
  del producto. Ninguna de estas etiquetas decide elegibilidad definitiva.

## Recibos de producto y límites

- [Reproducción TCK de producto][rpi-tck]: `silver-long-term` (**Silver38**:
  38 seleccionados, observados y aprobados) y `gold-cross-memory`
  (**Gold18**: 18 seleccionados, observados y aprobados). Ambos registran
  `valid=true` y `collection_matches=true`. Consultar [resumen][product-summary],
  [colección][product-collection] y resultados por caso
  [Silver][silver] y [Gold][gold].
- El RPI atribuye el binario a Cortex `c17ac76e306fbfa588916d411acac83e52d24fe5`;
  el resumen registra SHA-256
  `2f7b5cc606eaeae70cf26f6afd051553c9a69cb8b0b409ed7baf86f1bcd7ceef` e imagen
  Neo4j `sha256:249a1ec6e3c52d82101a5ed33d6ed2b5ed5b0017b21a28c0bfebc335e87fb177`.
  El adaptador ejecutó el binario de producto con proveedor `hash` y sin
  inferencia. Python 3.15 no coincidía completamente con `requirements-tck.txt`.
  El resultado no certifica Silver completo (no ejecutó la suite de
  razonamiento), todos los contratos Gold ni recuperación con modelos reales.
- **GM-01 `run-7`** es un diferencial de ciclo de vida, no otra ejecución
  del TCK del binario Silver/Gold. El [manifiesto][gm-manifest] identifica
  la referencia y el binario de la captura; registra `status=captured` y
  `h1_acceptance=not_established`. La [comparación][gm-comparison] registra
  `comparison=pass`, `execution=offline_capture_comparison`; el
  [ciclo de vida][gm-lifecycle] contiene 61 casos sin errores ni pendientes y
  `gm01_acceptance=pass_with_authorized_deviations`. Cubre operaciones y
  persistencia de conversación en el escenario fijado, incluidas cadena,
  borrado y sesiones intercaladas. No prueba paridad exacta de borrado,
  autorización entre propietarios, otros GM, ANN, MiniLM, C1 ni H1/H2/H3.

## Capacidades candidatas y aceptación verificable

Los casos enlazados son representativos: no sustituyen el conjunto de filas
relacionadas. **Crítico** señala un criterio candidato que no debe quedar
oculto por un agregado (aislamiento, pérdida de datos o integridad del grafo).

### BOLT-01 · Conexión, esquema y persistencia

- **Documental y estática:** TCK Vol. 3 «Lifecycle Methods» y Vol. 1
  `SPEC-1.1` a `SPEC-1.5`; [casos de esquema][case-schema]. Python enlaza
  el backend en [`MemoryClient._connect_bolt`][connect] y usa el
  [cliente Neo4j][graph-client]. El [inventario DDL](ddl-inventory-audit.md)
  desglosa restricciones, índices y fuentes; no es un recibo de inicialización.
- **Aceptación — crítico:** sobre una base aislada, inicializar dos veces
  sin perder datos, comprobar restricciones e índices requeridos por la
  configuración y releer los mismos IDs y relaciones después de reconectar.
  Ante configuración incompatible, comprobar error identificable y ausencia
  de escrituras de datos de usuario.
- **Producto y faltante:** GM-01 verifica persistencia de conversaciones en su
  escenario. Los perfiles de esquema del CSV verifican sus aserciones, no
  instalación y migración completas ni compatibilidad de índices vectoriales.

### BOLT-02 · Conversaciones, mensajes y orden

- **Documental y estática:** `SPEC-1.1`, `SPEC-2.1`, `2.2`, `2.4`, `2.7` y
  `2.8`; por ejemplo, [`test_get_conversation_preserves_metadata`][case-meta].
  Ver [escritura y lectura de `ShortTermMemory`][short] y su
  [listado de sesiones][list-sessions].
- **Aceptación — crítico para identidad y orden:** intercalar dos sesiones;
  comprobar apertura/reutilización de conversación, UUID distintos incluso
  con contenido duplicado, fidelidad de rol, Unicode y metadatos, y orden de
  inserción con timestamps no decrecientes. Verificar límites, conteos por
  sesión y ausencia de cruces. Contrastar el DTO público con la cadena
  persistida, incluso tras reiniciar.
- **Producto y faltante:** GM-01 y los perfiles de mensajes y cadenas del CSV
  dan evidencia para sus fixtures; no extrapolar a todas las opciones de
  escritura por lotes, metadatos o concurrencia.

### BOLT-03 · Borrado, limpieza y aislamiento

- **Documental y estática:** `SPEC-2.5`, `2.6`, `2.7.5` y `5.5.2–5.5.3`
  (Vol. 4); [`test_delete_middle_message`][case-delete] y
  [`test_conversations_isolated_but_entities_shared`][case-sharing]. Ver
  [`clear_session`][clear] y [`delete_message`][delete].
- **Aceptación — crítico:** con estados previos conocidos, borrar primero,
  medio y último; exigir `true` si existía y `false` al repetir o si no existe.
  Inspeccionar ausencia física, reparación de `FIRST_MESSAGE`/`NEXT_MESSAGE`,
  orden y conteos de supervivientes, sin relaciones colgantes. Limpiar solo
  la sesión objetivo, repetir y comprobar el estado tras el reinicio. Usar
  además dos propietarios con el mismo ID lógico de sesión: un acceso o
  borrado fuera de ámbito no debe devolver datos ajenos ni mutar su grafo.
- **Oráculo y producto:** `native-delete`, `adapted` y GM-01 no son
  intercambiables (véase la discrepancia de borrado). Gold18 verifica
  separación entre sesiones y compartición de entidades en su fixture;
  no establece autorización entre propietarios.
- **Faltante:** una verificación común del alcance de limpieza y borrado
  entre propietarios, con negativos de lectura, búsqueda, escritura y
  aristas. Mantener separados sesión, propietario y entidades compartidas;
  véanse [espacio de nombres](../owner-session-namespace.md) y
  [borrado de largo plazo](../long-term-isolation-deletion.md).

### BOLT-04 · Entidades, preferencias y hechos

- **Documental y estática:** `SPEC-1.2–1.4`, `3.1–3.3`, `3.6–3.7`;
  [`test_add_fact_basic`][case-fact] y
  [`test_get_related_entities_with_type_filter`][case-related]. Ver
  [altas de entidad, preferencia y hecho][long] y
  [recorrido de entidades relacionadas][related].
- **Aceptación:** crear, releer y distinguir entidades homónimas de distinto
  tipo. Comparar campos obligatorios y opcionales de preferencias y hechos,
  UUID, selección por nombre, filtros y respuesta vacía. Comprobar que el
  recorrido respeta dirección y tipo cuando el contrato lo exige.
- **Producto y faltante:** Silver38 cubre las altas, búsquedas y consultas
  seleccionadas. No verifica por sí solo todas las reglas de deduplicación,
  filtros por usuario y entidad, actualizaciones ni borrados de largo plazo.

### BOLT-05 · Trazas, pasos y llamadas de herramienta

- **Documental y estática:** `SPEC-1.5`, `4.1–4.7`;
  [`test_record_tool_call_timeout_status`][case-tool] y
  [`test_get_tool_stats_success_rate`][case-stats]. Ver
  [`ReasoningMemory`][reasoning], desde el inicio de traza hasta pasos y llamadas.
- **Aceptación — crítico para integridad:** registrar pasos ordenados,
  múltiples llamadas con distintos estados y un desenlace; releer la traza
  completa y comprobar `HAS_STEP`, `HAS_TOOL_CALL`, numeración y pertenencia
  correctas. Verificar filtros, límites, traza inexistente y agregados frente
  a llamadas conocidas, sin mezclar sesiones.
- **Producto y faltante:** Gold18 usa trazas y filtros en flujos compuestos;
  falta un recibo de toda la suite `test_reasoning.py` para el mismo candidato.
  Silver38 solo cubre largo plazo.

### BOLT-06 · Recuperación y composición de contexto

- **Documental y estática:** `SPEC-2.3`, `3.4–3.5` y `5.4` (Vol. 4);
  [`test_search_messages_session_filter`][case-search-messages],
  [`test_search_entities_finds_relevant`][case-search-entities] y
  [`test_get_similar_traces`][case-similar]. Ver [búsqueda de mensajes][search],
  [búsqueda de entidades y preferencias][search-long],
  [trazas similares][similar] y [`MemoryClient.get_context`][context].
- **Aceptación:** con datos fijados, exigir coincidencias y vacíos esperados,
  límites, umbrales, filtros y exclusión de datos ajenos antes de entregar
  resultados (**crítico**). Para contexto compuesto, comprobar procedencia y
  selección de cada memoria. Para equivalencia de búsqueda semántica,
  fijar modelo, dimensión, corpus y regla de comparación de ranking; comparar
  por separado búsqueda exacta y aproximada.
- **Producto y faltante:** el CSV registra perfiles de búsqueda; Silver38
  y Gold18 prueban solo sus aserciones con `hash`. Falta evidencia equivalente
  con inferencia real, contexto compuesto y ANN (búsqueda aproximada de
  vecinos); véase el [bloqueo ANN/exacta](../retrieval-h5-ann-exact-blocker.md).
  El contrato de contexto Python no se deduce del caso Platinum homónimo.

### BOLT-07 · Grafo entre memorias, relaciones y fusión

- **Documental y estática:** TCK §§1.6–1.7 y `SPEC-5.1–5.3`, `5.5` del
  Vol. 4; [`test_end_to_end_conversation_with_entities_and_reasoning`][case-e2e]
  y [`test_merge_preserves_relationships`][case-merge]. Ver
  [consultas de enlace][links], [relación tipada][relationship] y
  [`merge_duplicate_entities`][merge].
- **Aceptación — crítico para integridad:** persistir mensaje, entidad y
  traza, y luego verificar los enlaces solicitados por tipo, dirección,
  extremos y propiedades. Fusionar dos entidades con vecinos distintos;
  comprobar identidad resultante, conservación de relaciones requeridas y
  ausencia de referencias colgantes o cruces de ámbito. Distinguir
  compartición de entidades de acceso a conversaciones y trazas ajenas.
- **Producto y faltante:** Gold18 cubre los casos del flujo, recorridos,
  fusión y compartición seleccionados. Crear dos objetos en el mismo caso
  no prueba una arista. Falta contrastar el grafo completo y acordar qué
  aristas exige el contrato en el caso de `ABOUT` y `APPLIES_TO`.

### BOLT-08 · Operaciones Bolt fuera de las aserciones básicas

- **Estática:** en [api-profile.csv](api-profile.csv) quedan rutas públicas
  como el [procesamiento de extracción por sesión][extract], el
  [resumen de conversación][summary], [búsquedas geográficas][geo],
  `client.buffered.submit`, `client.consolidation.dedupe_entities`,
  `client.schema.adopt_existing_graph` y `client.eval.run`. No confundir
  una función declarada, un alias o un rechazo con otra capacidad positiva
  implementada.
- **Criterios candidatos adicionales:** para escritura diferida, distinguir
  aceptación de la solicitud de persistencia efectiva; para consolidación y
  adopción, comprobar que `dry_run` no escribe y contrastar el informe con
  los cambios observados en una ejecución real. Para evaluación, comprobar
  la selección de dimensiones y el informe con datos controlados. Clasificar
  estos escenarios de forma individual antes de fijar el denominador.
- **Aceptación y faltante:** agrupar por efecto observable antes de decidir
  qué medir. Para extracción, comprobar menciones/procedencia y el resultado
  con extracción desactivada; para resumen, comprobar la conversación de
  origen y persistencia; para filtros geográficos, interior, borde y exterior.
  Para una ruta no soportada, identificar el error público y comprobar que
  no hay efectos laterales. Los dos recibos TCK nuevos no validan estos
  escenarios ni deciden su inclusión definitiva.

## Discrepancias abiertas de contrato

### Borrado de mensajes encadenados

El TCK exige borrar y preservar el orden; el caso de borrado intermedio
comprueba los supervivientes en la lectura pública. El [código Bolt
fijado][delete-queries] no repara la cadena por sí mismo. En la línea base
histórica `native-delete` pasó un caso de trece; `adapted` sustituye la
operación y puede devolver `True` incluso para un ID ausente. Son contextos
distintos del diferencial GM-01: no trasladar al pin actual como explicación
única el fallo de la ruta histórica con propietario.

En GM-01 el upstream de la captura devuelve
`Neo.ClientError.Schema.ConstraintValidationFailed` para un mensaje encadenado.
[GM01-DEV-001](../gm01-deviations.md#gm01-dev-001--deletion-of-a-chained-message)
acepta la diferencia solo en ese escenario y si un oráculo independiente,
derivado del estado anterior, verifica la reparación y la conservación de
los supervivientes en Rust. GM01-DEV-002 solo permite descontar `owner_scope`
cuando falta upstream y vale exactamente `"unowned"` en Rust. Son límites
del diferencial; no son un fallo de Muninn ni una decisión general de paridad.

### `ABOUT` frente a `APPLIES_TO`

El [TCK §1.6][spec-edges] enumera `Preference → Entity` y `Fact → Entity`
mediante `ABOUT`. En Python, [`LINK_PREFERENCE_TO_ENTITY`][about] define el
`MERGE` de `ABOUT`, pero no tiene un llamador en el pin. La ruta pública de
preferencias [sí escribe `APPLIES_TO`][applies-to], y el filtro correspondiente
usa esa arista. No se identificó un escritor Bolt equivalente para
`Fact → Entity` mediante `ABOUT` en el pin.

La [documentación del producto](../cross-memory.md) describe `APPLIES_TO`
existente y `ABOUT` optativo mediante `about_entities`, no una sustitución.
Gold18 pasa [`test_preference_stored_alongside_entity`][case-pref], pero ese
caso solo comprueba el texto de la preferencia creada; no inspecciona aristas.
El caso de hechos comprueba la referencia por nombre, no una arista `ABOUT`.
Antes de aceptar esta capacidad, separar preferencia y hecho, escritura y
lectura, opción por defecto y explícita, e inspeccionar el grafo. Queda por
acordar qué comportamiento se compara; no se registra un fallo de Muninn.

## Integraciones: verificación aparte

- **MCP y clientes externos:** usar [mcp-baseline.csv](mcp-baseline.csv) y la
  [referencia MCP](../reference-mcp.md) para herramientas, esquemas y errores.
  Verificar descubrimiento e invocación por su propio transporte. Un resultado
  Bolt no certifica interoperabilidad MCP ni de frameworks o SDK.
- **Servicio alojado, NAMS y otros backends:** distinguir el [Volumen 5
  Platinum][platinum] y el perfil `nams` del CSV de API. Revisar solapamientos
  funcionales sin tratar el caso alojado como prueba Bolt ni usar su carácter
  opcional como exclusión definitiva del inventario.
- **Proveedores y extensiones:** registrar aparte configuración, identidad y
  comportamiento de embeddings, LLM, extractores, enriquecimiento y plugins.
  El efecto observable que produzcan en Bolt puede requerir una prueba Bolt
  adicional. Un fixture con `hash` no acredita una integración real.

## Siguiente decisión

Revisar los límites de los grupos, los criterios críticos y las dos
discrepancias antes de fijar el universo elegible. Para cada criterio aprobado
para evaluación, conservar pin, binario, entorno, datos de entrada, caso y
resultado, con retorno público y estado antes/después cuando corresponda.
Este hito no ejecuta builds ni pruebas de producto ni modifica el registro
`tests/parity/coverage.yaml`.

[rpi]: ../../../.agents/rpi/plan/tasks/muninn-bolt-parity.md
[rpi-tck]: ../../../.agents/rpi/plan/tasks/muninn-product-tck-reproduction.md
[product-summary]: ../../../.agents/rpi/plan/tasks/muninn-product-tck-reproduction-run-20261120-01/summary.json
[product-collection]: ../../../.agents/rpi/plan/tasks/muninn-product-tck-reproduction-run-20261120-01/collection.json
[silver]: ../../../.agents/rpi/plan/tasks/muninn-product-tck-reproduction-run-20261120-01/silver-long-term.json
[gold]: ../../../.agents/rpi/plan/tasks/muninn-product-tck-reproduction-run-20261120-01/gold-cross-memory.json
[gm-manifest]: ../../../.agents/rpi/plan/tasks/muninn-h4-tck-checkout-and-preflight/evidence/gm01-capture-20261120-diagnostic/run-7/manifest.json
[gm-comparison]: ../../../.agents/rpi/plan/tasks/muninn-h4-tck-checkout-and-preflight/evidence/gm01-capture-20261120-diagnostic/run-7/comparison.json
[gm-lifecycle]: ../../../.agents/rpi/plan/tasks/muninn-h4-tck-checkout-and-preflight/evidence/gm01-capture-20261120-diagnostic/run-7/lifecycle.json
[spec]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/SPEC.md
[spec-edges]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/SPEC.md#L168-L176
[platinum]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/SPEC.md#L585
[case-schema]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_schema.py
[case-meta]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_short_term.py#L198
[case-delete]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_short_term.py#L461
[case-sharing]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_cross_memory.py#L273
[case-fact]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_long_term.py#L167
[case-related]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_long_term.py#L334
[case-tool]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_reasoning.py#L164
[case-stats]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_reasoning.py#L393
[case-search-messages]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_short_term.py#L267
[case-search-entities]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_long_term.py#L219
[case-similar]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_cross_memory.py#L209
[case-e2e]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_cross_memory.py#L37
[case-merge]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_cross_memory.py#L188
[case-pref]: https://github.com/neo4j-labs/agent-memory-tck/blob/b6c72c36bb7486881b87053ee1610f2caf972fb6/tck/tests/v1/test_cross_memory.py#L96
[upstream]: https://github.com/neo4j-labs/agent-memory/tree/9d45af5529ecb5f2fe352013468a11087f3163d4
[connect]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/__init__.py#L498-L540
[graph-client]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/graph/client.py#L64
[short]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/short_term.py#L894-L1167
[list-sessions]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/short_term.py#L1327
[clear]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/short_term.py#L1286
[delete]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/short_term.py#L1378
[delete-queries]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/graph/queries.py#L225-L239
[long]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/long_term.py#L663-L1242
[related]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/long_term.py#L1524
[reasoning]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/reasoning.py#L425
[search]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/short_term.py#L1172
[search-long]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/long_term.py#L1403-L1523
[similar]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/reasoning.py#L1024
[context]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/__init__.py#L1289
[links]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/graph/queries.py#L541
[relationship]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/long_term.py#L1243
[merge]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/long_term.py#L1952
[extract]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/short_term.py#L1409
[summary]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/short_term.py#L2120
[geo]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/long_term.py#L2730
[about]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/graph/queries.py#L677-L682
[applies-to]: https://github.com/neo4j-labs/agent-memory/blob/9d45af5529ecb5f2fe352013468a11087f3163d4/src/neo4j_agent_memory/memory/long_term.py#L991-L1125
