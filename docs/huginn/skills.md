# Catálogo de Agent Skills

`huginn skills list` inspecciona metadatos locales sin configurar un modelo,
abrir una sesión SQLite, iniciar MCP ni solicitar entrada interactiva.

```sh
huginn --workspace ./packages/app skills list
huginn --workspace ./packages/app --json skills list
```

## Ubicaciones y precedencia

- Global: `$HOME/.agents/skills/<nombre>/SKILL.md`. Sin `HOME`, no se consulta
  el ámbito global.
- Proyecto: `<raíz>/.agents/skills/<nombre>/SKILL.md`. La raíz es el ancestro
  más cercano del workspace resuelto con un marcador `.git` de archivo o
  directorio. Sin marcador, se usa el workspace resuelto.

Solo se inspecciona un nivel de directorios. No se buscan colecciones anidadas
ni ámbitos adicionales por paquete. Un proyecto válido sobrescribe un global
con el mismo nombre exacto; ambos permanecen visibles con estado `selected`
o `overridden`. Las entradas inválidas no ocultan una entrada global válida.

Los resultados se ordenan por nombre ASCII, ámbito (proyecto primero) y ruta
lógica. Las rutas lógicas conservan el alias utilizado para descubrir el archivo;
las rutas canónicas indican el destino resuelto.

## Validación y límites

Cada archivo debe ser UTF-8 y comenzar con frontmatter YAML entre líneas `---`.
`name` y `description` deben ser cadenas. El nombre debe coincidir exactamente
con el directorio lógico: entre 1 y 64 caracteres ASCII en minúsculas, dígitos o
guiones, sin guiones iniciales, finales ni consecutivos. La descripción debe
contener texto no vacío y como máximo 1024 caracteres Unicode.

`compatibility`, si existe, debe contener entre 1 y 500 caracteres Unicode.
`license` y `allowed-tools` deben ser cadenas. `metadata` debe ser un mapa de
cadenas a cadenas. Los campos adicionales no se interpretan como políticas.

Huginn limita cada archivo a 65.536 bytes y cada raíz a 256 hijos directos,
incluidos los archivos que no sean directorios de skills. Una raíz con más hijos
se rechaza completa; la otra raíz se procesa de forma independiente. Estos
límites son política de Huginn, no requisitos de la especificación Agent Skills.

Los enlaces globales pueden apuntar fuera de su raíz, por ejemplo a un repositorio
de dotfiles. Tanto el directorio como el archivo resueltos de una skill de
proyecto deben permanecer dentro del repositorio canónico. Los destinos rotos,
cíclicos, ilegibles o no regulares producen diagnósticos.

## Contrato JSON

La salida es un único objeto, sin mensajes adicionales en stdout:

```json
{
  "fingerprint": "<SHA-256 hexadecimal de 64 caracteres>",
  "skills": [
    {
      "name": "code-review",
      "description": "Revisar cambios locales",
      "source_scope": "project",
      "logical_path": "/repo/.agents/skills/code-review/SKILL.md",
      "canonical_path": "/repo/.agents/skills/code-review/SKILL.md",
      "content_fingerprint": "<SHA-256 del archivo completo>",
      "trust": "untrusted",
      "selected": true,
      "status": "selected"
    }
  ],
  "diagnostics": [
    {
      "code": "name_mismatch",
      "scope": "global",
      "path": "/home/user/.agents/skills/alias/SKILL.md",
      "message": "name must exactly match the logical directory basename",
      "severity": "error"
    }
  ]
}
```

Las rutas JSON y humanas convierten bytes de ruta no UTF-8 mediante sustitución
por U+FFFD, sin abortar el comando. Las rutas nativas permanecen intactas en el
catálogo tipado y en el cálculo de huellas; la cadena mostrada con pérdida no
debe usarse como identidad para activar una skill.

Las raíces ausentes no son errores. Una entrada inválida genera un diagnóstico
y no impide mostrar entradas válidas. Los códigos estables son `root_unreadable`,
`path_resolution`, `root_limit`, `project_escape`, `not_directory`, `not_regular`,
`file_unreadable`, `file_limit`, `source_changed`, `invalid_utf8`,
`frontmatter_missing`, `invalid_yaml`, `metadata_type`, `invalid_name`,
`name_mismatch`, `invalid_description` e `invalid_compatibility`.
`project_escape` usa severidad `security`; los demás usan `error`.

## Frescura y confianza

No hay caché: cada consulta reconstruye el catálogo. El puerto
`SkillCatalogQuery::refresh` vuelve a leer y calcular SHA-256, incluso si una
edición conserva el tamaño y la fecha de modificación. La huella del catálogo
usa las entradas ordenadas, su ámbito, nombre, rutas y digest del archivo completo.
Las entradas sobrescritas también participan; los diagnósticos no participan.
Las rutas forman parte de la identidad: mover un archivo cambia la huella.

El adaptador comprueba metadatos y destinos antes y después de leer. Si detecta
cambios, rechaza la entrada o la raíz afectada. Estas comprobaciones no son un
sandbox ni garantizan aislamiento frente a un proceso local que modifique rutas
de forma concurrente. La activación vuelve a validar el origen.

Toda skill se etiqueta `untrusted`, incluida la global. La salida humana escapa
controles de terminal y controles bidireccionales; JSON conserva los datos
mediante serialización. `description`, `allowed-tools` y los demás metadatos nunca
conceden permisos. `huginn skills list` solo inspecciona el catálogo y no carga
instrucciones. `AGENTS.md` y `.agents/rules` mantienen su comportamiento anterior.

## Carga bajo demanda

La composición nativa anuncia al modelo metadatos globales como datos de tarea
`untrusted`, con identidad exacta, ámbito y ruta canónica. El anuncio tiene un
límite de 16 KiB de datos JSON; el anuncio indica cuántas entradas se omiten.
Las identidades seleccionadas por el host tienen prioridad. Si sus metadatos
no caben, el turno falla explícitamente. No incluye
cuerpos ni descripciones de proyecto sin selección explícita del usuario.

La herramienta `load_skill` acepta la identidad exacta anunciada, no una ruta
arbitraria ni un nombre ambiguo. Reconstruye el catálogo y comprueba otra vez
ámbito, rutas y huella del archivo antes de entregar `SKILL.md`. Si el origen
cambia, desaparece o deja de ser válido, la carga falla y solicita actualizar el
catálogo. El resultado JSON completo tiene un límite de 16 KiB; una skill que
supere ese límite falla sin truncar instrucciones. El contexto de la sesión puede
imponer un límite menor y conserva sus errores y reglas de compactación normales.

`StandardLoop::run_with_skills` permite al host seleccionar identidades de
proyecto para un solo turno. Esta selección no concede confianza persistente.
La herramienta no ejecuta scripts, carga referencias automáticamente ni modifica `HostPolicy`.
Las llamadas posteriores a herramientas siguen requiriendo sus aprobaciones.

La sesión guarda únicamente la huella versionada del catálogo, sin duplicar
cuerpos en sus metadatos. Los resultados normales de herramientas permanecen en
el historial original. Al iniciar otro turno, una huella distinta produce una
advertencia con las huellas anterior y actual. No se repiten cargas ni efectos
anteriores y no se reinyectan cuerpos fuera del historial normal.

## Invocación explícita

Usa `/skill:<nombre> [tarea]` en la TUI, modo línea, `run` o una sesión reanudada:

```sh
huginn --workspace . run '/skill:code-review Revisa "src/lib.rs" sin editar'
huginn --workspace . --json run '/skill:code-review'
huginn resume <uuid> --prompt '/skill:code-review Revisa los cambios nuevos'
```

La TUI muestra los nombres seleccionados y sus descripciones en el autocompletado
existente. Tab inserta el comando; Enter lo envía. Es un comando del host, no una
extensión de la gramática de comandos de plugins. El catálogo de autocompletado
es una instantánea de inicio; cada invocación consulta de nuevo el catálogo.

El host admite un máximo de 65.536 bytes por invocación. El nombre sigue las reglas
del catálogo. No admite rutas, scripts ni plugins remotos. Tras el nombre, consume
un carácter de espacio en blanco como separador y conserva todos los bytes restantes
de la tarea, incluidas comillas, barras inversas y espacios adicionales o finales.
No realiza análisis de shell ni expansión de argumentos. La tarea es opcional.

Antes de iniciar el turno o consultar el modelo, el host resuelve exactamente una
entrada `selected` y carga su identidad mediante `SkillLoader`. La precedencia de
proyecto sobre global no cambia. Un nombre inválido, desconocido, ambiguo, una
carga no disponible o una cancelación durante la carga rechaza la solicitud sin
inferencia, efectos ni turno de asistente artificial. Los errores JSON no añaden
texto humano a stdout. Las plantillas de prompts no pueden activar skills de forma
indirecta; introduce el comando de skill directamente.

La tarea y el resultado de carga se guardan en un mensaje de usuario como un objeto
JSON separado en `task` y `skill`. El cuerpo conserva `untrusted`, identidad, ámbito
y ruta canónica; nunca se convierte en un mensaje de sistema. Ni el cuerpo ni
`allowed-tools` conceden permisos: `HostPolicy` mantiene las aprobaciones normales.
Reanudar conserva el historial, pero una nueva invocación vuelve a validar y cargar
el origen actual. No reproduce cargas ni efectos anteriores.
