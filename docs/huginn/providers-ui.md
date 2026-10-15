# UI opcional de Providers

> Estado: implementación revisada y probada con dobles. Se corrigieron la
> cancelación de `/model` durante el guardado y la cancelación segura del formulario
> durante un paste incompleto (Ctrl+C; Esc inicia el delimitador del protocolo).
> No se validaron adaptadores remotos, Keychain/Secret Service del usuario ni
> aceptación de onboarding (integración P0.5); tampoco equivale a CI completa.

`builtin.providers.ui` aporta cuatro comandos sobre el servicio headless
`agent:connections@1` y el catálogo. No entrega terminal, foco, teclado ni
renderizado a los adaptadores.

## Uso

- `/login`: elegir proveedor y autenticación; introducir endpoint, modelo y,
  cuando corresponde, la clave en un campo secreto del host.
- `/providers`: consultar estado de conexiones, seleccionar modelo, actualizar
  configuración/credenciales o desconectar.
- `/model`: elegir conexión y modelo para el **siguiente turno**.
- `/logout`: confirmar la revocación local y limpieza de credenciales.

Tab o flechas verticales cambian de campo; flechas horizontales cambian opciones.
Enter envía; Esc cancela el formulario salvo durante un paste incompleto, donde
se interpreta como parte del delimitador de paste; usa Ctrl+C para cancelar sin
esperar ese delimitador. Ctrl+C dentro del formulario cancela la acción local,
no el proceso. Los campos secretos se muestran con una máscara constante y
no pasan por el editor de conversación ni por eventos JSON.

El almacenamiento seguro no cambia automáticamente a efímero cuando falla.
El modo efímero requiere elección explícita y pierde las credenciales al cerrar
el proceso. Tras reiniciar, la conexión conserva sus metadatos pero muestra
credenciales ausentes, no disponibilidad ficticia.

## Contrato y límites

`huginn-plugin-api::v1::Workflow` describe campos y una operación asíncrona.
El callback síncrono solo devuelve esa descripción. El host captura los campos
públicos y secretos por separado en `FormInput` y ejecuta el futuro fuera del
bucle de entrada. El registro original se conserva durante toda la cadena;
no se busca por nombre una contribución de reemplazo.

El host conserva una sola acción/formulario activo. Cerrar, cancelar o revocar
la contribución descarta los buffers y cancela la tarea. Esto **no demuestra**
rollback de una escritura duradera ni cancelación, facturación o revocación
remota. Los errores requieren comprobar el estado real antes de repetir.

Cambiar de modelo no rota la autorización. Un turno iniciado conserva su
conexión/modelo; revocar o actualizar credenciales impide posteriores inferencias
de ese turno. La configuración explícita mantiene prioridad al iniciar; una
elección interactiva es un cambio deliberado para turnos posteriores.

Los modelos estáticos del catálogo se muestran como opciones. Los adaptadores
con catálogo asociado a la conexión admiten un identificador candidato: esta
UI no afirma haber descubierto ni validado modelos remotos. La integración y
aceptación con adaptadores reales corresponde a P0.5, no a estas pruebas.

## Comprobación

Las pruebas de contrato y estado usan dobles controlados. Las pruebas PTY
capturan la salida de formularios con secretos centinela, errores y pánicos.
No validan proveedores reales, Keychain/Secret Service del usuario ni onboarding.

```sh
cargo test -p huginn-plugin-api
cargo test -p huginn-presentation --features test-driver
cargo test -p huginn-application
just huginn-architecture
```
