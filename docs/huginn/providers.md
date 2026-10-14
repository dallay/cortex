# Providers: contratos headless de Huginn

Esta entrega implementa P0.1 de [ADR-0014](adr/0014-builtin-providers-and-turn-scoped-model-routing.md)
y [#350](https://github.com/dallay/cortex/issues/350). No implementa todavía
`/login`, formularios, secretos persistentes ni adaptadores reales.

## Composición

`ProvidersPlugin` publica `agent:provider-catalog@1`, `agent:providers@1` y el
único `agent:model@1`. Los adaptadores requieren el catálogo y se registran
mediante `ProviderCatalog::register(ctx, adapter)` durante activación. El host
conserva la propiedad de registros y tareas; el plugin no controla el terminal.

Los registros se reservan inmediatamente para detectar colisiones, pero son
invisibles hasta que `Activation::is_active()` confirma publicación. Un fallo
de activación o discrepancia con el manifiesto libera la reserva. Teardown y
drop revocan handles retenidos, sin sustituirlos por una generación nueva.
Los identificadores también distinguen catálogos de supervisores diferentes.

Los builders existentes y `ModelPlugin` conservan sus firmas: delegan en
Providers y envuelven el proveedor configurado en una conexión efímera legacy.
Un `UnconfiguredModelProvider` no se presenta como conexión lista. No se añade
un segundo supervisor ni se cambia la configuración CLI/entorno existente.

## Selección y autorización

`ProviderAction` ofrece diagnóstico, conexión efímera, selección, revocación y
reemplazo. Son operaciones headless asíncronas con `CancellationToken`; no
autentican ni persisten credenciales. `ProviderActionState` describe estados
que un host futuro podrá mostrar al ejecutar las acciones, sin bloquear un
callback UI. Las acciones se serializan antes de mutar el estado; cancelación
previa o generación obsoleta no puede modificarlo.

`ModelProvider::pin_turn` es un seam aditivo: los proveedores estáticos no
necesitan implementarlo. El router fija conexión, revisión, adaptador y modelo
antes de esperar a una fábrica. `StandardLoop` conserva esa fijación para
respuesta, continuación tras herramientas y compactación automática. La
compactación manual fija su propia operación. Sin selección, el diagnóstico
funciona y la inferencia falla con `ProviderError::Unconfigured`.

Cambiar la selección afecta al turno siguiente. Revocar o reemplazar la
conexión invalida las inferencias siguientes, incluso en el turno actual.
Eliminar el adaptador o el núcleo invalida sus generaciones. No hay fallback
automático ante errores y una selección revocada permanece visible como tal
hasta que se elija explícitamente otra conexión.

La admisión de inferencia se serializa con revocación y reemplazo antes de
llamar al adaptador. Una solicitud admitida antes de revocar es una operación
en curso, aunque el adaptador todavía no haya sido sondeado.
Una revocación posterior a esa comprobación cancela localmente la operación
best effort, tanto al abrir como al consumir streaming. Abandonar el stream
cancela su token. Los productores reciben cancelación incluso si el consumidor
conserva el stream sin leerlo. Los watchers de generación pertenecen al kernel;
no se crean tareas desprendidas y no se afirma
que el servidor remoto haya cancelado, revocado o dejado de facturar.

El token entregado a `ProviderAdapter::create_model` dura solo lo que dure la
fábrica y termina también al completar con éxito. El modelo devuelto debe usar
el token nuevo de cada llamada `stream`, no retener el token de fábrica como
autorización de la conexión.

## Demostración sin red ni terminal

```sh
cargo run -p huginn-application --example providers_headless
```

Registra dos adaptadores internos de demostración, conecta ambos, fija uno,
selecciona el otro y verifica que la revocación no dispara fallback. No requiere
variables de proveedor, Ratatui, archivos de configuración ni credenciales.

## Verificación focal

```sh
cargo test -p huginn-core -p huginn-application
cargo test -p huginn-runtime --test stream_persistence --test coding_workflow
cargo test -p huginn --test cli --test config_compat
```

Las pruebas usan dobles, señales y barreras para selección durante herramientas,
compactación, fábricas y turnos concurrentes; cubren rollback, cancelación,
revocación y handles obsoletos. No certifican backends de secretos, proveedores
HTTP reales, onboarding definitivo ni aceptación humana del MVP.
