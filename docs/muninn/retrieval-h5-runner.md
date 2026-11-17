# Muninn H5 retrieval pilot runner

The one-shot harness in `dev/muninn/retrieval_h5_runner.py` calls the authenticated product
bridge routes `add_message` and, in the H5 diagnostic image only, `h5/diagnostic/search`. The product bridge runs the pinned local
MiniLM bundle and uses its existing Neo4j vector index. The harness does not call setup,
teardown, clear, or delete routes. Reports are descriptive (`measured_not_accepted`) and do not
establish a passing threshold or general hardware support.

The diagnostic image is built with the explicit `h5-diagnostic` Cargo feature. The default build
does not compile or register this route. The feature-gated route uses the same encoder, normalized
query, token-derived bridge owner, query-specific session, limit, and threshold for ANN and
`Store::search_messages_exact`. The exact search also receives that encoder's embedding-space ID.
Both calls use typed `Store` methods. The route returns only corpus aliases, scores, ranks, and a
closed `encoder_identity` object. The identity is obtained from the same `App.encoder` instance
after it encodes the query.
The route requires bridge bearer authentication and is reachable only from the internal H5 Compose
network; the H5 Compose file publishes no ports. The one-shot runner correlates internal IDs to
corpus aliases and drops the IDs before writing the report. The report includes no text, vectors,
internal IDs, owner identifiers, or credentials. The identity contains the declared artifact bundle
ID, pinned model ID and revision, embedding space ID, validated profile ID, and a SHA-256 fingerprint
of the exact `modules.json` and `1_Pooling/config.json` buffers read and validated during encoder
loading. The fingerprint uses length-prefixed labels and content lengths in fixed file order. It
does not identify weights, memory state, or the complete runtime configuration. This identity is
process self-report, not cryptographic attestation. The descriptive runner validates the exact
allowlist and records the identity with the same response and query. Without independent expected
protocol identity and Docker/container/run/query correlation, it records identity provenance as
`partial`; caller claims cannot replace those observations or enable PASS. The diagnostic remains
descriptive and does not change normal ANN ranking or establish acceptance.

## Safe local run prerequisites

Do not start this run until every item below is verified locally:

1. Obtain the completed annotation CSV from its approved local location. Do not edit or copy its
   completed judgments. Compute its raw SHA-256 with `shasum -a 256 <csv>`.
2. Verify the exact corpus and CSV with the existing validators. Supply the immutable local model
   bundle at `.cache/minilm-artifacts/ee963db150ae720729aebee53794830f1d0e60960c9253e575a049b799758ac1`.
3. Provision dedicated, ephemeral Neo4j auth and bridge token files under ignored
   `tmp/muninn/h5-secrets/`. Keep that directory mode `0700` and both files mode `0600`. The
   network-disabled staging container receives only those two read-only file mounts and uses
   `DAC_OVERRIDE` to read their host-owned contents before copying them into the project volume.
   Do not reuse `dev/muninn/.cache/neo4j.auth`,
   `dev/muninn/.cache/bridge.token`, or any shared/pre-existing credentials. Set
   `MUNINN_H5_NEO4J_AUTH_FILE` and `MUNINN_H5_BRIDGE_TOKEN_FILE` to those files' paths. This task
   does not generate credentials. Do not proceed if the dedicated files are unavailable.
4. Build the diagnostic bridge image separately only under the approved isolated build procedure.
   Use `--build-arg MUNINN_BRIDGE_FEATURES=h5-diagnostic` and a new local image tag. This Compose
   file does not build or pull images. Set `MUNINN_H5_BRIDGE_IMAGE` to a locally present immutable
   digest reference; verify it with `docker image inspect`. Set `MUNINN_H5_MODEL_BUNDLE` to the exact
   existing local bundle directory. The one-shot `artifacts` service copies this read-only bind mount
   without preserving host ownership or modes, sets directory modes to 0500 and file modes to 0400,
   then changes ownership to uid/gid 7474 in a project-scoped named volume. The bridge mounts only that
   staged volume read-only. No host-owned bundle is mounted directly into the bridge. The
   network-disabled `credentials` service copies the bytes from the two dedicated host credential
   files into a separate project-scoped named volume, changes ownership to uid/gid 7474, and sets
   both files to mode 0400. Neo4j and the bridge wait for that service and mount the credential
   volume read-only at `/run/secrets`. Compose file-backed secret mounts are not used. Never set a database URI; the bridge connects to its fixed
   `neo4j:7687` service.
5. Verify `dev/muninn/compose.h5.yaml` resolves to project `muninn-h5-pilot-v1`, that its only
   writable database storage is `/data` tmpfs, that the network is internal, and that no service
   publishes a host port. The runner uses the bridge service alias only on the internal network.
   If host resource paths cannot be proven disposable, stop.
6. Start only the named project with `docker compose -p muninn-h5-pilot-v1 -f
   dev/muninn/compose.h5.yaml up -d --pull never`. Do not use an existing Muninn/Neo4j project.
7. Set `MUNINN_H5_RUNNER_FILE` to `dev/muninn/retrieval_h5_runner.py`,
   `MUNINN_H5_QUALITY_MODULE_FILE` to `dev/muninn/retrieval_quality.py`, and
   `MUNINN_H5_CORPUS_FILE` to the exact pilot corpus,
   and `MUNINN_H5_ANNOTATIONS_FILE` to the completed annotation CSV. Set the raw CSV hash,
   verified bridge image identity, and source/build identity using the required Compose variables.
   The runner reads the token from the staged read-only credentials volume at
   `/run/secrets/bridge_token`; do not pass the token in environment variables or Compose config.
   The runner accepts `http://bridge:8080` only when the exact H5 disposable, project, and internal
   runner markers are present. The supplied bridge and source/build identities are caller-supplied
   inputs; the runner does not inspect Docker or independently verify them. The report marks the
   bridge identity `caller_supplied_not_verified`. Verify the bridge container's image digest
   locally before the run. The runner records a SHA-256 over its current runner and metrics source
   files. It raises the ANN search limit to at least the total corpus candidate count because ANN
   selects globally before session filtering. It refuses candidate sets above the bridge maximum of
    100. Metric cutoffs remain K={1,3,5}.
9. Use a unique runner container name. After the pilot bridge and prerequisites are already running,
   initialize the output volume explicitly, then create the one-shot runner with `--no-deps`.
   This prevents Compose from rerunning completed artifact and credential staging jobs. The
   evaluation profile's `runner-output-init` service has no network and mounts only the named
   output volume. The actual bridge and Neo4j services must already be up. Replace `<unique-id>`
   with a fresh identifier and use the same value for both commands:

   ```sh
    docker compose -p muninn-h5-pilot-v1 -f dev/muninn/compose.h5.yaml --profile evaluation \
      run --no-deps runner-output-init
    runner="muninn-h5-pilot-v1-runner-<unique-id>"
    docker compose -p muninn-h5-pilot-v1 -f dev/muninn/compose.h5.yaml --profile evaluation \
      run -d --no-deps --name "$runner" runner
   status="$(docker wait "$runner")"
   test "$status" = 0 || { echo "runner exited with status $status" >&2; exit 1; }
   mkdir -p tmp/muninn
   docker cp "$runner:/out/report.json" "tmp/muninn/retrieval-h5-<unique-id>.json"
   docker rm "$runner"
   ```

   Keep the copied report in `tmp/muninn/` until review. Do not remove the project output volume
   before the copy. The report records its runner source digest, so dirty source is not
   misrepresented by a Git commit ID.
10. Review the JSON for expected provenance and absence of query/candidate text, internal IDs,
    owner identifiers, and credentials. The report includes
    per-query and macro Recall@K, MRR, and nDCG@K, raw CSV SHA-256, rubric/reviewer/adjudication,
    corpus hash, model and source/build identities with their verification status, search
    configuration, and host/runtime facts. Its H5 diagnostic section records both ANN and exact
    alias/score/rank lists for each query. It emits no query or candidate text or embeddings.

The runner rejects all non-loopback endpoints except the exact internal bridge alias in marked
runner mode, absent H5 markers, and non-H5 loopback ports. It disables proxy-derived transport
configuration. It does not inspect or control Docker, and it does not verify
the supplied Docker image identity. The caller must verify image identity before the run. A missing
ability to prove host resource paths or image identity is a preflight failure, not permission to
proceed.

## Teardown

After review and when authorized, discard only the disposable project by running
`docker compose -p muninn-h5-pilot-v1 -f dev/muninn/compose.h5.yaml down --volumes --remove-orphans`.
This removes only the explicitly named pilot's containers and its `artifacts`, `credentials`,
and `runner-output` volumes. Never use a database reset or teardown API, and never target another
Compose project.

## H5 acceptance protocol framework (not an execution procedure)

`retrieval_h5_protocol.py` validates a versioned protocol document and computes its canonical
content hash. A document with pending decision fields is a non-executable example. No metric, K,
tolerance, run count, aggregation rule, corpus, adjudication method, or comparison scope is supplied
as a default. A completed protocol-run must also pin the exact allowlisted `encoder_identity` expected
from the diagnostic response. Unknown fields are rejected, including fields that could weaken the
evidence baseline.

A complete protocol document and repository review reference are not, by themselves, approval. The
operator must corroborate that the reviewed commit/hash approved the exact protocol content before
any confirmatory measurement. This is process evidence only. The framework does not claim a
cryptographic signature or attestation.

The protocol preflight reports each gate separately and never reports a global acceptance state.
It does not contact the bridge, write a measurement report, or start a measurement. The local
provenance collector can call Docker Engine `inspect` and hash local file bytes. Its inputs can be
tested with an inspect-compatible fake; this does not require a running Docker service. The collector
records container/image identities, inspect-derived mounts and Compose labels, run correlation, and
local file hashes. It does not treat caller-supplied encoder or configuration claims as observations.

Docker `inspect` and local file hashes cannot establish which encoder or effective retrieval
configuration the bridge loaded into memory. The response self-report can be correlated only when
its strict identity matches the protocol-run expectation and the external container/image/run and
query binding. This correlation does not attest to process memory. Missing or conflicting identity
evidence remains `partial` or `insufficient` and preflight blocks PASS even when protocol fields are
complete. Undefined results also remain blocked until their treatment is explicitly approved for the
protocol-run; the framework does not choose a policy. The historical pilot report remains
descriptive, not pre-registered, and not accepted. Preflight returns a separate report object and
does not overwrite or reclassify historical reports.

`retrieval_h5_provenance.py` assesses each gate against the fixed evidence baseline. `verified` is
limited to local observations within the repository/operator/host/Docker Engine trust boundary. It is
not cryptographic verification and does not prove that inspected bytes were loaded in process. This
framework does not authorize running services or a confirmatory measurement. Retrieval behavior,
Store, APIs, rankings, filters, scope, threshold, and fallback remain unchanged.

## Contrato estructural v2 offline

`h5-acceptance/v1` conserva su validación estructural y las cuatro combinaciones de gates.
El piloto conserva sus annotations obligatorias y su carácter descriptivo. No hay migración
ni promoción automática de v1 a un contrato ejecutable.

`h5-acceptance/v2` añade un contrato cerrado para ANN–exact: K=10 en ambas métricas,
`threshold=1.0`, una corrida y `policy_id=ann-exact-per-query-equality-v1`.
Los tokens de agregación, empates e indefinidos son, respectivamente,
`per-query-governing-macro-descriptive-v1`, `observed-ranking-no-retrospective-tiebreak-v1`
y `required-undefined-blocks-pass-v1`. `criterion` y `rationale` siguen informativos;
el validador no interpreta prosa.

El scope añade `corpus_hash` (`sha256:` y 64 hexadecimales minúsculos). La comparación usa
`retrieval_path=/h5/diagnostic/search`, `candidate_scope=same-query-candidates-v1` y
`search_config` cerrado: `limit` entero de 10 a 100 y `threshold` finito de 0 a 1.
No hay defaults; los booleanos no son números. Como en JSON Schema, v2 acepta representaciones
enteras como `10.0`; NaN e infinito no tienen representación JSON y Python los rechaza.
La comparación por subprocess usa serialización JSON estricta: el CLI instalado de `jsonschema`
acepta NaN con su decoder permisivo, por lo que no acredita por sí solo validez de sintaxis JSON.

La validación estructural no concede aprobación ni permiso de ejecución. Todo el documento,
incluida su versión y prosa, participa en el hash; cualquier cambio invalida su corroboración
anterior. La plantilla y el candidato real siguen en v1, incompletos y sin aprobación de corrida.
Esta unidad solo incorpora el contrato y pruebas sintéticas offline; no añade aún el flujo
protocol-aware, no mide y no modifica la clasificación histórica ni los bloqueos QA/H5.

## Elegibilidad previa de procedencia (offline)

`assess_preflight_provenance(gate, observations)` comprueba registros `LocalObservations`
y devuelve `eligible` o `blocked`, nunca procedencia final `verified`, autorización ni `PASS`.
Reutiliza las comprobaciones de presencia/metadatos/conflicto de la evaluación final.
El marcador local rechaza un `dict` de claims; no constituye una barrera contra código hostil
ni acredita por sí mismo que un adaptador haya observado los valores.

El baseline previo deriva del final, sin exigir `gate_results`, `encoder_response_identity`,
`encoder_response_binding`, `finished_at` o `encoder_loaded_identity`. No fabrica esos registros.
`effective_config` sigue siendo obligatorio como observación externa disponible antes de responder;
no completa ni acredita configuración efectiva que dependa de la respuesta. La evaluación final
sigue exigiendo configuración completa y todos los registros posteriores, junto con la correlación
existente. Identidad recibida significa self-report correlacionado, no atestación.

La elegibilidad comprueba registros, no la igualdad de sus valores con protocolo/corpus/run.
Esas comparaciones y la autorización corroborada pertenecen al preflight del runner futuro;
no hay baseline público configurable, ejecución nueva ni permiso de medición en esta unidad.
`collect_observations` permanece intacto y no se invoca con finalización ficticia para este chequeo.

## U9 offline: preflight protocol-aware sin efectos

`run_protocol_evaluation` acepta protocolo, corpus, registro de operador, run ID, bridge y proveedor
`ProtocolRunEvidence`. Valida localmente y corrobora antes de llamar `before_run` una sola vez.
Comprueba las cuatro envolturas especializadas, sus bindings contra protocolo/corpus/run y los
registros previos U8. El proveedor debe devolver `LocalObservations`; un diccionario plano bloquea.
Las entradas y observaciones se copian antes de comprobarlas. Las excepciones de adquisición,
incluido `TypeError` del proveedor, bloquean con categorías fijas y sin publicar mensajes crudos.
Un `TypeError` de `before_run` se trata como fallo de contrato del proveedor: retorna el finding
fijo `observations` / `preflight_evidence_missing_or_conflicting` en vez de propagar la excepción.

El límite sin efectos descrito aquí aplica al **paso de preflight**: antes de que el preflight sea elegible,
no se llama a `Bridge.post`, `after_response`, `after_run` ni a métricas; todo rechazo local devuelve
`measurement_started=false`. Cuando el preflight bloquea, la ejecución termina con ANN–exact `blocked`
y relevancia `not_evaluated`. Cuando el preflight es elegible, el flujo continúa por U10 y el informe
final refleja esa ejecución; U11 añade después las métricas y el veredicto ANN–exact. La sección U9
describe el límite del preflight, no el resultado final de la API tras completar U10/U11.

La matriz pública de U9 incluye findings ordenados/deduplicados con categorías y paths fijos. Un
protocolo inválido, cíclico o demasiado profundo produce `protocol_invalid` en `protocol`; observaciones
cíclicas o demasiado profundas producen `preflight_evidence_missing_or_conflicting` en `observations`.
No se añade un límite arbitrario de profundidad a los valores polimórficos U8. Las tareas U9 offline (14.1–14.3) están completas y revisadas. Esto no autoriza mediciones reales ni acepta H5. El piloto y
`protocol_preflight` histórico permanecen intactos.

## U11 offline: criterio ANN, procedencia final y veredicto acotado

`run_protocol_evaluation` integra en su retorno final el veredicto del gate `ann_exact`
mediante `gate_result`. Reutiliza la serialización ANN con K=10 y `quality_queries=[]`.
El veredicto evalúa `rule_satisfied`: `True` únicamente si todas las consultas tienen
`ExactTopKRecall@10=1.0` y `OrderedTopKMatch@10=1.0`; `False` si todas están definidas y
alguna no cumple; `None` si la cobertura es incompleta o alguna métrica requerida es
indefinida (e.g. ranking exacto vacío).

El informe incluye `macro_descriptive` con la media aritmética exacta de cada métrica
únicamente si todas las consultas están definidas; en caso contrario reporta `value: null`
junto con el recuento exacto de consultas indefinidas (`undefined_count`). No se redondea
ni se descartan consultas indefinidas.

La procedencia final (`after_response` y `after_run`) exige que todos los registros obligatorios
estén presentes, no conflictivos y correlacionados para cada consulta (incluyendo
`encoder_response_identity`, `encoder_response_binding`, `run_id`, `bridge_container_id`,
`protocol_hash` y `corpus_hash`). Cualquier omisión, conflicto o discrepancia produce
`provenance_status = "insufficient"`, bloqueando el gate (`status: "blocked"`) aun con
métricas numéricas perfectas.

Un veredicto `PASS` en esta API es estrictamente un resultado sintético del contrato técnico
offline frente a dobles de prueba; no constituye certificación de hardware, validez de embeddings,
representatividad de corpus ni aceptación global H5. `absolute_relevance` permanece
`{status: "not_evaluated", results: {}}`.
