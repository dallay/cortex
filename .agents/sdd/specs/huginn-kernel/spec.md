# Especificación del kernel de Huginn

> **Domain**: `huginn-kernel`<br>
> **Change**: `huginn-service-provider-substitution`<br>
> **Issue**: [#509](https://github.com/dallay/cortex/issues/509)
> **Type**: New spec

## Purpose

Kernel genérico; model y session se prueban en aplicación.

## Requirements

### Requirement: Identidad y composición

El kernel MUST identificar servicios por nombre y major explícita, resolver determinísticamente y rechazar ambigüedad o ciclos.

#### Scenario: Identificadores y composición inválidos

- GIVEN `agent:model@2` requiere un proveedor `agent:model@1`, o hay proveedor duplicado/ciclo
- WHEN se resuelve o activa
- THEN falla explícitamente e identifica servicio o conflicto

### Requirement: Activación transaccional

El kernel MUST publicar tras activación válida, limpiar efectos parciales, asignar una generación y revocarla con sus recursos al retirar o cerrar.

#### Scenario: Activación fallida

- GIVEN un plugin que registra y falla
- WHEN termina la activación
- THEN no publica el servicio y limpia efectos

### Requirement: Ownership y dependientes

El kernel MUST exponer consultas de solo lectura de proveedor, generación y dependientes; tras commit MUST informar nuevo ownership.

#### Scenario: Consulta tras commit

- GIVEN una sustitución confirmada
- WHEN se consulta el servicio
- THEN devuelve proveedor/generación nuevos y dependientes vigentes

### Requirement: Sustitución transaccional

El kernel MUST sustituir sin reiniciar, conservar servicios previos y evitar ownership ambiguo, doble o ausente. Fallo pre-commit MUST conservar anterior y limpiar alternativa. Diagnóstico MUST identificar servicio, proveedores y resultado; ante fallo, motivo.

#### Scenario: Sustitución exitosa o fallida

- GIVEN alternativa compatible, incompatible o con fallo de activación
- WHEN se solicita sustituir
- THEN el commit publica la alternativa; si falla, conserva el anterior y elimina registros parciales
- AND el diagnóstico identifica ambos proveedores y el resultado o motivo

### Requirement: Leases cooperativas con plazo

El consumidor MUST mantener lease durante operación protegida. La sustitución MUST cerrar admisión y esperar leases con plazo finito; MUST NOT cancelar operaciones válidas. Timeout MUST abortar, conservar anterior publicado y no revocado, y reabrir admisión.

#### Scenario: Operación en vuelo o timeout

- GIVEN operación con lease anterior, incluso al vencer el plazo
- WHEN se cierra admisión o se aborta por timeout
- THEN no se cancela; el timeout conserva anterior y reabre admisión
- AND durante el cierre se rechazan leases nuevas

### Requirement: Drenaje y reanudación

`RequireFullDrain` MUST rechazar dependientes sin hook antes del cierre. `ReportUndrained` MAY registrarlos y continuar, pero MUST esperar quiescencia. Abort MUST reanudar intentados contra anterior; commit MUST reanudarlos contra nuevo.

#### Scenario: Política y resultado del drenaje

- GIVEN un dependiente sin hook o cuyo drenaje se intentó
- WHEN se sustituye con `RequireFullDrain`/`ReportUndrained`, y se aborta o confirma
- THEN la primera política rechaza; la segunda registra y espera; la reanudación usa anterior en abort y nuevo en commit

### Requirement: Rebind de consumidores de aplicación

Los builders productivos de application/runtime —`huginn_runtime::composition::build_native`, `huginn_runtime::composition::build_native_with_connections`, `huginn_runtime::composition::build_native_with_connections_and_guards` y `huginn_application::composition::build`— MUST garantizar que el mismo `LoopService` retenido por la aplicación use los providers vigentes en la siguiente operación `run` o `compact` que comience tras un commit, sin reconstruirse. Cada operación activa MUST conservar sus generaciones anteriores y sus leases hasta completarse; la sustitución MUST NOT cancelarla ni cambiar sus providers a mitad de operación. Todas las llamadas de session `save` de una operación MUST usar una única generación y un único store; la siguiente operación tras el commit MUST usar el store nuevo. El sistema MUST NOT migrar estado entre stores.

Esta garantía MUST NOT aplicarse al `LoopPlugin(pub LoopConfig)` legacy ni a composiciones personalizadas que solo capturan servicios con `resolve() -> Arc<T>`. Una composición personalizada que requiera rebind dinámico MUST optar por `PluginContext::service_slot_handle`, leases y hooks de ciclo de vida.

#### Scenario: Rebind de model

- GIVEN la aplicación retiene un `LoopService` creado por uno de los builders productivos application/runtime enumerados en este requisito, y hay un `run` o `compact` activo que usa el model de la generación anterior
- WHEN la sustitución espera a que termine la operación activa, confirma el provider nuevo y comienza otra operación en el mismo `LoopService`
- THEN la operación activa termina sin cancelación con el model anterior y la nueva usa el model nuevo sin reconstruir el `LoopService`

#### Scenario: Rebind de session

- GIVEN la aplicación retiene un `LoopService` creado por uno de los builders productivos application/runtime enumerados en este requisito, y un `run` o `compact` activo realiza varias llamadas `save` en el store A, que contiene un registro identificable ausente del store B
- WHEN la sustitución espera a que termine la operación activa, confirma el store B y comienza otra operación en el mismo `LoopService`
- THEN todas las llamadas `save` de la primera operación usan la misma generación y el store A, y la operación termina sin cancelación con el provider anterior
- AND la siguiente operación usa el store B y el registro de A no aparece en B por efecto de la sustitución

#### Scenario: Composición legacy o personalizada

- GIVEN `LoopPlugin(pub LoopConfig)` o una composición personalizada conserva un `Arc<T>` obtenido con `resolve()`
- WHEN se confirma una sustitución y comienza una operación posterior
- THEN el `Arc<T>` conservado sigue siendo la referencia capturada y no se garantiza rebind dinámico; una composición personalizada que lo requiera usa `PluginContext::service_slot_handle`, leases y hooks de ciclo de vida

### Requirement: Genericidad y compatibilidad

El kernel MUST NOT conocer `ModelProvider` ni `SessionStore`. `resolve() -> Arc<T>` queda fuera de garantía: MUST NOT afirmarse revocación arbitraria. `huginn-plugin-api::v1` MUST seguir compatible.

#### Scenario: Referencias y API existentes

- GIVEN consumidor que conserva un `Arc<T>` de `resolve()` y consumidores de `huginn-plugin-api::v1`
- WHEN se sustituye el proveedor
- THEN no se garantiza revocar el `Arc` y los contratos existentes siguen válidos
