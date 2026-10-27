# Biblioteca restrictiva de rutas y shell

## Estado

`huginn-guards` expone una biblioteca de rutas y shell. Todavía no se registra como plugin,
no está conectada a herramientas nativas y no protege run/chat/resume por defecto.
No incorpora cargador de archivos ni integración con la CLI.

## Uso explícito

```rust
use huginn_guards::Guardrails;
use std::path::Path;

let policy = Guardrails::from_toml("block_paths=['private']")?;
policy.check_path(Path::new("/workspace"), Path::new("private/file"))?;
```

El host decide si invoca la biblioteca. Un resultado permitido no autoriza un
efecto ni sustituye `HostPolicy`, aprobación o validación del workspace.

## Configuración desde texto

`from_toml` acepta texto TOML de hasta 64 KiB con `block_paths` y `block_commands`.
Admite hasta 128 rutas de 1024 bytes, relativas, sin traversal ni controles.
Admite hasta 128 ejecutables adicionales, cada uno no vacío, de hasta 128 bytes,
con caracteres ASCII alfanuméricos, punto, guion o guion bajo. Compara sus nombres
sin distinguir mayúsculas ASCII y los combina con el catálogo incorporado.
Rechaza campos desconocidos: `allow_paths`, `enabled` y `confirm` no se aceptan
ni se ignoran. El proyecto solo añade restricciones, no desactiva el catálogo.
La biblioteca no busca, abre ni carga un archivo de configuración del proyecto.

## Catálogo y resolución

Se bloquean `.env`, `.env.*`, `.ssh`, `.gnupg`, claves privadas `id_rsa`, `id_dsa`,
`id_ecdsa`, `id_ed25519`, `credentials`, `credentials.json`, `guardrails.toml` y
sufijos `.pem`, `.key`, `.p12` y `.pfx`. Las claves públicas como `id_rsa.pub`
no se bloquean por ese nombre. Los nombres y prefijos se comparan sin distinguir
mayúsculas ASCII; no se promete normalización Unicode ni coincidencia de substrings.
Los prefijos del proyecto respetan los límites entre componentes de ruta.

`check_path` comprueba la ruta solicitada y la resuelta. Normaliza componentes y
resuelve enlaces, incluido el ancestro existente de un destino todavía nuevo.
Los enlaces colgantes o una raíz no disponible producen un error.

## Comprobación de shell

```rust
let policy = Guardrails::from_toml("block_commands=['curl']\nblock_paths=['private']")?;
policy.check_shell("git status --short")?;
policy.check_shell_in(Path::new("/workspace"), "cat public.txt")?;
```

`check_shell` analiza el subconjunto léxico y comprueba argumentos y destinos
de redirección contra rutas sensibles y prefijos de proyecto. No resuelve aliases
de archivos: `check_shell_in` añade resolución de rutas respecto del workspace,
incluidos los valores después de `=` en argumentos. Ninguno ejecuta el comando.

El parser acepta palabras, comillas simples/dobles, escapes admitidos, comentarios,
separación por nueva línea o `;`, cadenas `&&`/`||`, pipes y redirecciones simples
`<`, `>` y `>>` con destino explícito. Una descripción entre comillas no se
reinterpreta como comando. Los operadores deben tener comandos completos.

Rechaza entrada vacía o de más de 16 KiB, exceso de tokens, controles no admitidos,
comillas incompletas, expansiones, sustituciones `$()`/backticks, globbing,
continuaciones no admitidas, heredocs, background y sintaxis compuesta no soportada.
Las asignaciones iniciales, descriptores numéricos adjuntos y redirecciones ambiguas
se rechazan de forma conservadora, aunque algunas formas sean válidas en POSIX.

## Catálogo restrictivo de comandos

- Bloquea `dd`, `shred`, `wipe`, `mkfs`, `mkfs.*`, `fdisk` y `diskutil`.
- Bloquea `rm` recursivo y opciones largas no admitidas antes del separador `--`.
- Bloquea `git reset --h*`, opciones globales de Git y `git clean` sin dry-run
  inequívoco; solo admite las opciones conocidas de esa comprobación.
- Bloquea `find` con ejecución (`-exec`, `-execdir`, `-ok`, `-okdir`) o `-delete`.
- Rechaza control de flujo shell, intérpretes `sh`, `bash`, `dash`, `zsh`, `ksh`,
  `fish`, evaluadores `eval`, `source`, `.`, `alias`, `trap` y ejecución indirecta
  mediante `exec`, `command`, `builtin`, `env`, `sudo`, `doas`, `xargs`, `nohup`,
  `timeout`, `time`, `nice`, `watch`, `parallel`.
- Rechaza `cd`, `chdir`, `pushd` y `popd`: no intenta simular directorios cambiantes.

El nombre del ejecutable se toma del último componente de su ruta y se compara
sin distinguir mayúsculas ASCII. `alias` y `trap` se bloquean incluso cuando sus
payloads parecen inocuos; no se interpreta su contenido para decidir permisos.

## Límites de seguridad

No es un sandbox ni un mecanismo de autorización ni un intérprete shell completo.
No inspecciona el interior de scripts o programas arbitrarios, ni implementa una
política para MCP. Tampoco detecta todos los alias mediante hardlinks.
La sustitución concurrente entre comprobación e I/O sigue sin resolverse: no hay
confinamiento atómico. La pertenencia al workspace y los efectos siguen siendo
responsabilidad del host y de sus adaptadores, que aún no usan esta biblioteca.
