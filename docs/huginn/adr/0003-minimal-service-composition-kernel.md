# ADR-0003: Minimal service composition kernel

- Status: Accepted
- Date: 2026-10-06
- Product: Cortex agent (provisional name)
- Decision basis: Explicit choices made during the agent MVP planning conversation.

**Subsequent decision (2026-10-09):** [ADR-0010](0010-ratatui-plugin-first-interactive-terminal.md) requires that the Ratatui frontend and user-visible features be registered as first-party plugins from the start of the new interactive phase, with an authorable extension path. The original MVP deferral of a stable public SDK, hot reload and native dynamic loading remains historical; it is not a mandate to build monolithic new features.

## Context

The research already accepts composability, dependency-driven lifecycle,
generation-owned effects and a replaceable agent loop. Implementing the whole
plugin framework before a usable agent would delay validation of the product.

## Decision

The MVP includes a real, minimal composition kernel, not only a collection of
statically wired interfaces.

The kernel owns service resolution, plugin lifecycle, generation identity,
effect cleanup and diagnostics. Concrete model access, tools, session storage and
reasoning belong to replaceable capabilities outside the kernel.

Start with one process-level service-resolution scope and one selected provider
per service. Contracts have explicit major versions. Missing hard dependencies
block activation; ambiguous providers and dependency cycles fail explicitly.

Activation must publish registrations only on success and clean up partial effects
on failure. Dependents stop accepting new work and stop before a required provider
is removed. Plugin-owned tasks and registrations are disposed during teardown.

The concrete agent loop is replaceable without changing kernel implementation.
First-party capabilities may be statically linked native Rust.

## Decisión posterior — sustitución cooperativa de proveedores (2026-10-29)

El supervisor puede sustituir un proveedor de servicio activo sin reiniciar el
proceso. La alternativa debe seguir publicando todos los servicios del proveedor
anterior. Puede añadir servicios solo si ningún otro plugin registrado declara
esos IDs. Así se conserva el conjunto de servicios existente y se permite una
extensión segura.

Antes del commit, el supervisor cierra la admisión de leases para la generación
anterior, ejecuta los hooks de drenaje disponibles y espera a que terminen las
leases activas dentro de un único plazo finito. El plazo de los hooks y la espera
de leases no cancelan operaciones válidas protegidas por leases.
`RequireFullDrain` rechaza la solicitud si algún dependent activo de cualquier
servicio de esa generación no tiene hook de drenaje. `ReportUndrained` permite
esos dependientes y los registra. Si falla el drenaje, vence el plazo de leases o
falla la activación alternativa, se reabre la generación anterior y se reanudan
los dependientes cuyo drenaje se intentó. El kernel registra el resultado de la
sustitución y los IDs de dependientes drenados y no drenados.

If the substitution future is cancelled before commit, the supervisor reopens the
previous gates and schedules cleanup of the alternative and resumption of attempted
dependents in the current runtime. After commit, finalization runs inline, not in a
background task. Dropping the future at that point leaves the alternative active,
can interrupt retired-plugin cleanup or dependent resumption, and prevents recording
the substitution diagnostic.

`Supervisor::substitute` can return `Err` after commit when retired-provider
deactivation or dependent resumption fails. The alternative remains active. Callers
must inspect `provider_of(service)` and the corresponding entry in
`substitution_diagnostics()` instead of treating this error as a rollback. When the
`Supervisor` is dropped, the kernel revokes its gates and clears its slots, so
retained handles cannot acquire new leases. The lifecycle order places the
alternative after its required providers and before its existing dependents. The
supervisor rejects a substitution when no position satisfies both constraints.

Las leases son un contrato cooperativo de API, no una revocación fuerte. El
consumidor debe mantener la lease durante toda la operación y no debe conservar
una referencia interna al proveedor después de liberar la lease. El método
heredado `resolve() -> Arc<T>` sigue disponible, pero no ofrece esta garantía.
El kernel no garantiza revocar `Arc`s retenidos ni trabajo detached.

Esta decisión no añade comportamiento específico de capacidades a
`huginn-core`, no cambia `huginn-plugin-api::v1` y no promete revocar referencias
arbitrarias a proveedores. Refina el requisito previo de que los dependientes
dejen de aceptar trabajo antes de retirar un proveedor requerido.

## Decisión posterior — slots y rebind de loop en producción (2026-10-29)

El kernel mantiene una tabla global de slots por `ServiceId`. Un
`ServiceSlotHandle<T>` retiene la tabla y el ID; cada `acquire()` resuelve el
valor y gate vigentes e incrementa su lease bajo la misma sincronización. El
commit reemplaza todos los slots de la generación en una sección crítica. Un
abort precommit restaura el provider anterior, reabre los gates y reanuda los
dependientes cuyo drain se intentó. El timeout no cancela operaciones activas.

En producción, `huginn_application::composition::build` y los builders nativos
de `huginn_runtime::composition` publican el adapter privado `LeasedAgentLoop`
mediante la factory `leased_loop_plugin` y `LeasedLoopPlugin`. `run` y `compact`
obtienen admisión y leases de model y session antes de producir efectos, y las
mantienen hasta completar la operación. Todos los `save` de una operación usan
el mismo store. El drain cierra la admisión y espera operaciones activas;
`resume` la reabre tras abort o commit. Si falla la segunda lease, RAII libera
la primera sin mutar session ni producir efectos. El tipo concreto de plugin
sigue privado.

La compatibilidad pública prevalece sobre la conversión del loop legacy:
`StandardLoop` conserva sus seis campos públicos, firmas y struct literals;
`LoopPlugin(pub LoopConfig)` conserva su constructor. El uso directo de esos
tipos publica el loop estático y no obtiene garantía de rebind. La garantía de
producción pertenece a los builders que registran `LeasedLoopPlugin` mediante
`leased_loop_plugin`.

La readiness de doctor, CLI y TUI adquiere el slot actual de `ModelService` en
cada comprobación y falla cerrada si no se puede adquirir. No conserva un
`Arc<ModelProvider>` ni una bool de readiness capturada. Esta lectura dinámica
no amplía la garantía cooperativa de las leases.

`SqliteSessions::lease`, `load` y `list` siguen fuera de `SessionsService` y
directos al adapter `SqliteSessions`. La sustitución no transfiere locks ni
migra estado al store nuevo. No se añade migración de base de datos ni cambio
de esquema.

Hot reload, dynamic native libraries, multiple scopes, provider fan-out and a
public plugin SDK are outside the MVP.

## Alternatives considered

- Complete plugin framework first: delays an end-to-end useful product.
- Simple static wiring followed by a later kernel: defers validation of the main thesis.
- Putting the agent loop in the kernel: makes experimentation require core changes.

## Consequences

- Kernel behavior needs meaningful lifecycle and failure-injection tests.
- Native composition is sufficient to establish plugin semantics.
- Cleanup guarantees apply to registrations and supervised runtime work; they do not
  automatically reverse filesystem writes or other completed external side effects.
- Native code is trusted and does not obtain a sandbox from a service interface.

## Follow-up and evidence

Acceptance must demonstrate blocked dependencies, activation rollback, consumer-before-provider
shutdown, cleanup without orphan registrations or tasks, and replacement of a capability.

The exact Rust trait signatures and crate layout remain implementation details.
This preserves the accepted research principles R-003, R-004, R-006, R-007 and
R-011 through R-018 while narrowing their first delivery.
