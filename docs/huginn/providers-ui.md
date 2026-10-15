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

## Prueba manual en macOS

La TUI requiere una terminal real. Desde el checkout del PR, en una ventana de
Terminal o iTerm:

```sh
cargo run -p huginn --
```

Deberías entrar a la conversación y ver una indicación para configurar un modelo.
Prueba primero el flujo sin credenciales externas:

1. Escribe `/login` y pulsa Enter. Recorre proveedores y autenticación; cancela
   con Esc. No hace falta guardar nada para comprobar el formulario.
2. Si tienes Ollama local y un modelo ya descargado, inicia Ollama y sigue
   `/login` con endpoint `http://127.0.0.1:11434`, permitiendo explícitamente
   HTTP solo para loopback. Usa el identificador de un modelo local instalado.
   No descargues nada como parte de esta prueba.
3. Alternativamente, si tú decides usar una cuenta de prueba, introduce la clave
   únicamente en el campo secreto enmascarado. No la pongas en argumentos CLI,
   el prompt, el archivo de configuración, capturas, logs ni comentarios. La UI
   no valida aquí la disponibilidad remota; evita claves de producción.
4. En `/providers`, revisa el estado y cambia el modelo; `/model` debe indicar que
   la selección aplica al siguiente turno. Comprueba que `/logout` confirma la
   revocación local, sin afirmar que invalida la clave en el proveedor.
5. Prueba Ctrl+C durante un formulario y pega texto incompleto si tu terminal
   soporta bracketed paste. El formulario debe cancelarse sin mostrar el contenido
   pegado ni cerrar la TUI. Esc dentro de un paste incompleto pertenece al framing;
   usa Ctrl+C para cancelar.
6. Sal con `/quit`. Reiniciar permite comprobar el estado persistido, pero no el
   modo efímero, salvo que lo selecciones explícitamente y aceptes perder la clave
   al cerrar.

La prueba local con Ollama sí toca un adaptador real, pero no valida Keychain /
Secret Service del usuario ni onboarding P0.5. Las pruebas automatizadas usan
dobles controlados; PTY captura que secretos centinela, errores y pánicos no
aparezcan en salida.

## Comprobación automatizada

```sh
cargo test -p huginn-plugin-api
cargo test -p huginn-presentation --features test-driver
cargo test -p huginn-application
just huginn-architecture
```
