# Catálogo de modelos y elegibilidad

Rook separa tres hechos: la presencia de un identificador, las capacidades
declaradas del modelo y las capacidades que conserva el adaptador. La presencia
de un modelo no demuestra que admita herramientas, imágenes o streaming.

## Declaraciones

El catálogo incluido en `models-catalog/src/models.json` conserva los
identificadores anteriores. No contiene garantías de capacidades: sus estados
iniciales son `unknown`. Las declaraciones administrativas de `rook.toml`
permiten registrar capacidades comprobadas:

```toml
[[model_catalog]]
provider_kind = "openai"
model_id = "deployment-verificado"

[model_catalog.capabilities]
tools = "supported"
streaming = "supported"
vision = "unsupported"
reasoning = "unknown"
context_tokens = 8192

[[model_catalog]]
provider_kind = "openai"
model_id = "deployment-verificado"
connection_id = "c56a4180-65aa-42ec-a945-5fd21dec0538"

[model_catalog.capabilities]
tools = "unknown"
```

Los identificadores y el límite del ejemplo son ilustrativos, no garantías de
un proveedor real. El administrador debe verificar el modelo y el endpoint antes
de declarar `supported`. Las declaraciones no contienen credenciales.

La resolución aplica cada campo en este orden de prioridad:

1. Declaración de la conexión exacta y del modelo.
2. Declaración del tipo de proveedor y del modelo.
3. Declaración incluida en el catálogo.
4. `unknown`.

Un campo omitido hereda el valor anterior. Un estado explícito `unknown`
elimina una afirmación de soporte de menor prioridad. `context_tokens` es
opcional y debe ser positivo; su ausencia significa que el límite no se conoce.
Las declaraciones duplicadas para la misma conexión, proveedor y modelo son
inválidas. Los cambios requieren reiniciar Rook.

Los modelos adicionales de una declaración de proveedor se añaden al catálogo
de ese proveedor. Los modelos declarados para una conexión no se publican en
otras conexiones. Azure conserva únicamente su deployment configurado;
OpenAI-compatible conserva únicamente `supported_models`. Los metadatos no
amplían esas dos listas.

## Procedencia y descubrimiento

Cada capacidad y límite conocido conserva `source`: `bundled`,
`provider_configuration`, `connection_configuration`, `adapter` o `unknown`.
`ModelCatalogPort::resolve` acepta también identificadores obtenidos mediante
descubrimiento verificado por el llamador. Combina ese identificador con las
declaraciones aplicables, pero no deduce capacidades por nombre o presencia.
Las comprobaciones de salud no se convierten en descubrimiento de capacidades.
Esta implementación no añade consultas automáticas a proveedores.

Los metadatos resueltos permanecen asociados a la misma instancia del adaptador
durante las actualizaciones del registro y atraviesan el límite de concurrencia.

## Política conservadora

Una capacidad obligatoria necesita soporte positivo del modelo **y** del
adaptador. Rook excluye candidatos con estado `unsupported` o `unknown` antes de
ejecutar una llamada al proveedor. Los descartes no consumen intentos de llamada
ni abren circuitos. Si no queda un candidato elegible, se conserva el error de
agotamiento de proveedores; los combos incluyen motivos de descarte distintos
para incompatibilidad y falta de evidencia.

- Las declaraciones de herramientas, `tool_choice` distinto de `none` y los
  mensajes con historial de herramientas requieren `tools`.
- Una solicitud de streaming directo requiere `streaming`.
- El texto ordinario no requiere capacidades opcionales no utilizadas.
- Los combos ejecutan `complete` y sintetizan SSE. No necesitan streaming nativo.
- Los permisos del alias y de sus pasos físicos se comprueban antes del filtrado
  por capacidades. Un paso posterior no autorizado sigue rechazando el combo.
- Las solicitudes que requieren herramientas no leen ni escriben la caché de
  respuestas. Una respuesta guardada antes de cambiar la declaración no permite
  eludir la política actual.

Los adaptadores actuales no conservan imágenes de entrada. OpenAI rechaza los
contenidos no textuales al deserializar la solicitud, con HTTP 422, antes de
convertirla al dominio o realizar llamadas externas. Anthropic también rechaza
bloques de imagen no representables. Declarar `vision = "supported"` no habilita
esa funcionalidad. El soporte de controles explícitos de razonamiento tampoco
se habilita con una declaración.

Los adaptadores OpenAI, Azure OpenAI, OpenAI-compatible y Anthropic pueden
conservar solicitudes de herramientas; Gemini, Groq y Ollama no conservan
actualmente todos los campos necesarios y no pueden habilitarse por configuración.
Gemini no implementa streaming nativo. Estas restricciones del adaptador no
constituyen evidencia sobre las capacidades del modelo remoto.

## Respuesta de `GET /v1/models`

Se conservan `id`, `object`, `created` y `owned_by`. La extensión aditiva `rook`
incluye capacidades, procedencia de presencia y política de desconocidos:

```json
{
  "id": "connection-id/deployment-verificado",
  "object": "model",
  "created": 0,
  "owned_by": "connection-id",
  "rook": {
    "capabilities": {
      "tools": { "state": "supported", "source": "provider_configuration" },
      "vision": { "state": "unsupported", "source": "provider_configuration" },
      "reasoning": { "state": "unknown", "source": "provider_configuration" },
      "streaming": { "state": "supported", "source": "provider_configuration" },
      "context_tokens": { "value": 8192, "source": "provider_configuration" }
    },
    "presence_source": "runtime_registry",
    "unknown_policy": "reject_required"
  }
}
```

La lista respeta `allowed_models` y `allowed_providers` del contexto autenticado.
Un alias `rook/*` solo es visible si sus pasos físicos también están autorizados.
Los aliases conservan sus identificadores y publican capacidades `unknown`:
Rook no atribuye a todo el combo el soporte de uno de sus pasos.

## Migración

**Antes de actualizar un despliegue que usa herramientas o streaming directo,
declare las capacidades verificadas de sus modelos.** Esto también aplica a los
identificadores incluidos en el catálogo: sus capacidades anteriores no estaban
verificadas y no se convierten automáticamente en `supported`.

No use una declaración global de proveedor para endpoints personalizados que no
comparten sus garantías. Prefiera `connection_id` para esos casos. Una solicitud
de texto sin herramientas y sin streaming conserva su elegibilidad anterior.

Para probar sin proveedores reales:

```sh
cargo test -p models-catalog
cargo test -p rook-usecases --test capability_eligibility
cargo test -p rook-usecases --lib route_request::tests::capabilities
cargo test -p transport-axum --test capability_contract --test gateway_e2e
cargo test -p rook --test config_tests --test di_tests
```
