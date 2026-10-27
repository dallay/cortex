# Huginn extension authoring

This guide describes the supported **in-process, statically linked** extension
path. It is an authoring contract, not a promise of dynamic loading, hot reload,
or a stable Rust ABI.

## The boundary

The kernel owns service discovery, dependency ordering, generation identity,
transactional publication, approval authority, and cleanup. A presentation host
owns the TTY, raw mode, focus, keymap, layout, and final rendering.

An extension uses `huginn-plugin-api::v1`. It does not import Ratatui,
Crossterm, `huginn-presentation`, or terminal input APIs. Contributions are
declarative and host-rendered:

- commands receive a string of arguments and return a bounded `View` or the ID
  of a host-mediated selection dialog;
- tool-result renderers select an exact tool name, with one optional `*` fallback;
- dialogs expose bounded text and choices, never an approval response;
- widgets publish text into a named host-owned status slot.

The host sanitizes returned text, applies size limits, converts callback panics
into errors, and decides whether a contribution is visible. The plugin API
exposes no terminal input, ANSI output, key handling, tool-permission grants, or
approval-policy mutation. That boundary is contractual and API-surface only:
in-process native extensions share the host process and are trusted code, not
sandboxed or isolated. Authors must not use process or OS I/O (spawning
processes, opening the TTY, writing escape sequences) to bypass it.

## Reference extension

`crates/huginn/reference-extension` is a separately authored example. It
depends on `huginn-plugin-api` and `async-trait`, not on the TUI crate. It
registers two commands, a dialog, a tool-result renderer, and a widget.

The application statically composes it by default:

```bash
cargo run -p huginn --features reference-extension -- chat
cargo run -p huginn --no-default-features -- chat
```

The second command demonstrates disabling the extension at build time without
editing TUI rendering code. A separately developed native extension currently
follows the same pattern: add it as a Cargo dependency and register its
`Plugin` from the application composition root.

## Minimal shape

```rust
use huginn_plugin_api::{
    kernel::{Manifest, Plugin, PluginContext},
    v1::{contributions_id, CommandResult, Contribution, UiService, View},
    Result,
};
use std::sync::Arc;

pub struct MyExtension;

#[async_trait::async_trait]
impl Plugin for MyExtension {
    fn manifest(&self) -> Manifest {
        Manifest {
            id: "example.my-extension".into(),
            provides: vec![],
            requires: vec![contributions_id()],
        }
    }

    async fn activate(&mut self, ctx: &mut PluginContext) -> Result<()> {
        let ui = ctx.resolve::<UiService>(&contributions_id())?.0.clone();
        ui.register(
            ctx,
            "example.my-extension.info".into(),
            Contribution::Command {
                name: "/my-info".into(),
                handler: Arc::new(|arguments| {
                    Ok(CommandResult::Notice(View::text(format!(
                        "Arguments: {arguments}"
                    ))))
                }),
            },
        )?;
        Ok(())
    }
}
```

Registration is generation-owned. The host publishes it only after activation
and manifest validation succeed. On activation failure, removal, shutdown, or
supervisor drop, every registration and owned task is revoked and cleaned up.
Retained handles therefore fail closed after teardown.

## Tool hooks

An extension that requires `agent:tool-hooks@1` can register a `ToolHook` from
`huginn_plugin_api::v1::hooks`. A hook inspects each tool call before preparation
and approval. It can let the call continue or block it with a reason; it cannot
approve anything. An `after` method may append a short note to the result the
model sees.

```rust
let hooks = ctx.resolve::<ToolHooksService>(&tool_hooks_id())?.0.clone();
hooks.register(ctx, "example.deny-shell".into(), Arc::new(DenyShell))?;
```

Hooks run in hook-id order and the first block wins. Errors, panics and
timeouts block the call. Reasons and notes are sanitized and limited to 4 KiB.
The loop limits the combined annotation block to 16 KiB of content plus a visible
truncation marker. It trims the body when needed to keep notes visible and the
combined result within 64 KiB plus the marker's 19 bytes. Each `after` hook still
receives the original bounded tool result and error status. Plain tool output
keeps its existing 64 KiB content budget plus a marker when truncated.
Registrations follow the plugin generation like other contributions. See
[ADR-0016](adr/0016-restrict-only-tool-hooks.md).

## Registro de herramientas desde plugin-api

Un autor nativo puede implementar `Tool` y registrar herramientas mediante
`huginn_plugin_api::v1::tools`, sin depender de `huginn-runtime`:

```rust
use huginn_plugin_api::v1::tools::{tool_registration_id, ToolRegistrationService};

// En manifest(): requires: vec![tool_registration_id()].
// En activate(), con una herramienta que implementa Tool:
let service = ctx.resolve::<ToolRegistrationService>(&tool_registration_id())?;
service.register(ctx, tool)?;
```

El módulo también expone `Tool`, `ToolDefinition`, `ToolCall`, `ToolContext`,
`PreparedAction` y `ApprovalRequest`. El autor puede construir
`PreparedAction { approval: Some(ApprovalRequest { id, action, preview }), payload }`
para solicitar aprobación de un efecto; la solicitud no equivale a autorización.
El servicio exige la dependencia declarada y la misma instancia
publicada por el host; retener otra instancia no sustituye esa declaración.
`ToolRegistration` es el puerto que implementan los adaptadores del host, no una
concesión de permisos al autor.

El host construye el servicio con `ToolRegistrationService::new(port)`, donde
`port` es un `Arc<dyn ToolRegistration>`, y lo publica con
`ctx.provide(tool_registration_id(), service)`. El campo del puerto es privado:
la API del autor solo permite registrar mediante `service.register(ctx, tool)`.
La implementación pública de `ToolRegistration` para `Registry` sigue siendo una
API de adaptador del host; esta encapsulación no convierte el código nativo en un
sandbox.

`agent:tool-registration@1` comparte almacenamiento con los servicios anteriores
`agent:tool-registrations@1` y `agent:tools@1`. Los nombres se reservan al registrar;
las herramientas solo son visibles tras la activación y validación del manifest.
Las colisiones producen un error. Si la activación falla, el contexto revoca las
reservas. Al retirar el plugin, cerrar el supervisor o descartarlo, sus leases
revocan las herramientas y los handles retenidos rechazan preparación y ejecución.

El registro no autoriza efectos: el host conserva la decisión de aprobación y los
hooks solo pueden restringir. El código nativo sigue siendo confiable y comparte
el proceso; esta API no añade aislamiento ni carga dinámica. La ruta antigua
`Registry::register` exige declarar su servicio `registry_id()` y usar el registro
publicado por el host; `Registry::native` e `insert` conservan su API existente.

## Conflicts and dependencies

Contribution IDs and capability keys must be unique. Commands reserve their
command name; renderers reserve their tool selector; widgets reserve their
slot; dialogs reserve their contribution ID. Reserved host commands such as
`/quit`, `/exit`, `/compact`, and `/summarize` are rejected. Duplicate providers,
unsupported `name@major` dependencies, cycles, and missing dependencies are
reported through supervisor diagnostics; a missing UI service leaves the
extension blocked and does not affect headless operation.

## Trust and future loading

Native Rust extensions are trusted code in this release. The API boundary
reduces coupling and preserves host authority; it is **not** a sandbox. There
is no automatic runtime loader, unloadable native ABI, hot reload, WASM
isolation, or process/IPC isolation in this contract.

The compatibility, discovery, lifecycle, sandbox, and versioning analysis for
WASM, IPC/process, and native-loader options is recorded in
[ADR-0013](adr/0013-authorable-plugin-contract-and-loading.md). A future loader
must be selected explicitly before claiming any of those properties.
