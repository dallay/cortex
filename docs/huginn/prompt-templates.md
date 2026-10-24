# Plantillas Markdown de tareas

Huginn descubre plantillas de texto de usuario sin ejecutar código de plantillas.
Estas plantillas son distintas de `AGENTS.md` y de las instrucciones de skills.
No cambian las instrucciones de sistema ni conceden permisos para herramientas.

## Directorios y precedencia

- Usuario, macOS y Linux: `~/.huginn/prompts/`.
- Usuario, Windows: `%USERPROFILE%\.huginn\prompts\`.
- Proyecto: `<project-root>/.huginn/prompts/`.

La raíz del proyecto es el ancestro más cercano con `.git`; sin repositorio,
Huginn usa el workspace. La raíz de usuario es el directorio personal del sistema.
Estas rutas no trasladan la configuración ni la base de datos actuales.

Solo se descubren archivos directos con extensión `.md` en minúsculas. El nombre
es el basename sin extensión: entre 1 y 64 caracteres ASCII alfanuméricos,
guiones o guiones bajos. La comparación distingue mayúsculas de minúsculas.
La plantilla válida del proyecto prevalece sobre una plantilla de usuario con
el mismo nombre. Un archivo inválido produce un diagnóstico y no se selecciona.
La lista de autocompletado se carga al abrir la TUI; reiníciala para descubrir
nuevos nombres. La invocación vuelve a leer y validar el archivo.

## Invocación

```text
/prompts:review src/lib.rs "thread safety"
```

La TUI ofrece autocompletado con descripción y alcance. El mismo texto funciona
en `--line-mode` y como argumento de `huginn run`:

```sh
huginn run '/prompts:review src/lib.rs "thread safety"'
```

El namespace `/prompts:` sigue el patrón histórico de Codex. No se registran
aliases como `/review`; `/login` y `/skill:` conservan su identidad.

## Formato y variables

Ejemplo de `review.md`:

```markdown
---
description: Revisar código
argument-hint: archivo "enfoque"
---
Review $1 with focus $2; context: $@
```

El frontmatter es un formato limitado de pares `clave: valor`, no YAML general.
Admite `description`, `argument-hint` y su alias `argument_hint`. Admite valores
simples y comillas exteriores, líneas vacías y comentarios completos con `#`.
Los metadatos malformados o desconocidos generan errores. No se admiten valores
multilínea en el frontmatter. El cuerpo conserva saltos de línea y Unicode.

- `$1` a `$9`: argumentos posicionales; una posición faltante genera un error.
- `$ARGUMENTS` y `$@`: todos los argumentos unidos con un espacio.
- `$NAME`: argumento nombrado, suministrado como `NAME=valor`. El nombre contiene
  letras ASCII mayúsculas, dígitos y `_`, y empieza por una letra.
- `$$`: signo `$` literal. Por ejemplo, `$$HOME` produce `$HOME` sin leer el entorno.

Las comillas simples y dobles agrupan un argumento. Un backslash escapa el
carácter siguiente salvo dentro de comillas simples. Las comillas y escapes
incompletos generan errores. Una cadena entre comillas vacías es un argumento
vacío. Las asignaciones nombradas también cuentan como argumentos posicionales
y forman parte del agregado. Una asignación duplicada usa el último valor.
Las variables nombradas y posicionales pueden combinarse en una plantilla.
`$10` y otras posiciones de varios dígitos generan un error; `$0` queda literal.
No hay valores predeterminados.

La sustitución ocurre una sola vez. Los valores insertados no se reinterpretan
como variables. No hay expresiones, ejecución de shell, lectura de secretos ni
expansión de variables de entorno. Los caracteres `$` seguidos por símbolos no
admitidos quedan literales.

## Límites y seguridad

- Máximo 256 entradas directas por directorio; si se supera, se omite ese
  directorio con un diagnóstico en lugar de seleccionar un subconjunto arbitrario.
- Máximo 64 KiB por archivo UTF-8.
- Máximo 256 KiB de argumentos y resultado expandido.
- No se admiten directorios de plantillas relocados por symlinks ni archivos
  symlink. Se comprueban las rutas al descubrir y al invocar.

El texto expandido entra como mensaje de usuario en el flujo normal. El historial
conserva ese texto, no una referencia que cambie al editar después la plantilla.
Las autorizaciones de `HostPolicy` siguen aplicándose a cualquier herramienta
que el modelo solicite. Un texto que pide omitir una autorización no la omite.
Los errores de expansión impiden enviar esa tarea al modelo. En modo JSON no se
imprimen metadatos ni diagnósticos de catálogo en stdout.
