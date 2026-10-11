# Contrato de releases y distribución de Cortex

Este documento define cómo se liberan los productos de Cortex. La regla
principal es separar tres conceptos:

1. **Producto:** Rook y, posteriormente, Huginn.
2. **Arquitectura:** crates internos organizados por responsabilidades y
   boundaries.
3. **Distribución:** canales mediante los cuales se instala cada producto.

No tienen que compartir el mismo ciclo de vida.

## Identidad de productos

Cortex es el repositorio y el ecosistema. Los productos actuales son:

| Producto | Binario | Código principal | Tag | Estado público |
| --- | --- | --- | --- | --- |
| Rook | `rook` | `apps/rook` | `rook@vX.Y.Z` | Pipeline activo; distribución pública condicionada a preflight |
| Huginn | `huginn` | `apps/huginn` | `huginn@vX.Y.Z` | Release Please activo; distribución pública espera aceptación y preflight |

No existe un release de producto `cortex@vX.Y.Z`. Los tags históricos de Rook
sin prefijo se conservan y nunca se reescriben. Los releases nuevos usan
`rook@vX.Y.Z`.

## Release Please

El repositorio usa manifest mode:

- `.release-please-manifest.json` y `release-please-config.json` contienen los
  dos productos activos de versionado independiente: Rook y Huginn.
- La activación de Huginn en Release Please permite release PRs y tags de
  producto; la revisión y merge de esas PRs queda bajo control humano.
- Los jobs de build, GitHub assets y distribución siguen condicionados al
  output de Rook. La presencia de Huginn en Release Please no lo añade a esos
  canales.
- `separate-pull-requests: true` mantiene separados los release PRs.
- `always-update: true` sincroniza la metadata del producto.
- `include-component-in-tag: true` y `tag-separator: "@"` hacen explícitos los
  tags.
- El plugin `cargo-workspace` puede actualizar dependencias Rust reales, pero
  la co-localización en el repositorio no debe activar un producto ajeno.

El workflow de producción consume únicamente outputs path-scoped:
`apps/rook--release_created`, `apps/rook--version`,
`apps/rook--tag_name` y `apps/rook--sha`. El workflow consume también outputs path-scoped de Huginn (`apps/huginn--release_created`, `apps/huginn--version`, `apps/huginn--tag_name` y `apps/huginn--sha`), pero por ahora no hay jobs de distribución conectados a esos outputs. Los root outputs están prohibidos.

## Regla de build inmutable

Cada job de distribución hace checkout del tag del producto y verifica que
`HEAD` coincida con el SHA entregado por Release Please. Nunca se construye
desde `main` mutable.

Cada archive nativo contiene el binario del producto, usa el nombre del
producto, SemVer y Rust target, y tiene un sidecar SHA-256. La salida de
`<binary> --version` debe coincidir con el SemVer del release sin la `v`.

## Canales de distribución de Rook GA

### GitHub Releases — obligatorio

El release de Rook recibe únicamente sus archives y checksums. Los nombres
usan `rook-<version>-<target>`. Los artefactos se generan desde el tag y SHA
exactos.

### npm — obligatorio

El paquete base es `@dallay/rook`. Los paquetes de plataforma activos son:

```text
@dallay/rook-darwin-arm64
@dallay/rook-darwin-x64
@dallay/rook-linux-x64
@dallay/rook-windows-x64
```

Los paquetes de plataforma se publican primero. Después se publica el wrapper
base con `optionalDependencies` de la misma versión exacta. Los nombres no
contienen backticks ni otros caracteres accidentales.

Solo se incluyen plataformas con evidencia de build y smoke test de Rook.
Huginn debe definir su propia matriz; no hereda automáticamente la de Rook.

### Docker Hub y GHCR — obligatorios

Las imágenes son product-scoped:

```text
dallay/rook:<version>
ghcr.io/dallay/rook:<version>
```

La matriz inicial habilitada es `linux/amd64`. `linux/arm64` se incorporará
solo cuando exista evidencia específica de build y smoke test. Tags móviles
como `latest` son locales al producto.

El contenedor de Rook es un gateway HTTP y puede tener healthcheck. El futuro
contenedor de Huginn será un contrato de CLI/agent y no copiará el startup ni
el healthcheck de Rook.

### Cargo/crates.io — fase posterior, no bloquea Rook GA

`cargo install rook` desde crates.io no es requisito del primer lanzamiento.
El canal Rust soportado en Rook GA es:

```bash
cargo install \
  --git https://github.com/dallay/cortex.git \
  --tag rook@vX.Y.Z \
  --locked \
  -p rook
```

Los crates internos permanecen privados/no publicados en esta fase. No se
reestructura Clean Architecture para satisfacer únicamente una restricción de
packaging de Cargo.

La publicación en crates.io queda como una fase posterior. Cuando se active:

1. se verifica ownership de los nombres, especialmente los nombres genéricos;
2. se calcula el cierre transitivo real de dependencias de Rook;
3. se renombran crates si hace falta un namespace reconocible, por ejemplo
   `cortex-shared-kernel`;
4. se añaden versiones a las dependencias path usando path + version;
5. se publican solo los crates necesarios, en orden topológico;
6. se publica `rook` después de que el cierre esté disponible.

Los crates publicados no serán productos comerciales ni tendrán releases de
producto propios, pero sí serán unidades técnicas de versionado: una versión
publicada es inmutable y una corrección posterior requiere otra versión.

## Dry-run y gates

`workflow_dispatch` con `dry_run: true` es read-only. Valida metadata activa,
fixtures de independencia, YAML, JSON y paquetes npm. No ejecuta el action
Release Please y no abre release PRs, tags, GitHub Releases, publicaciones de
registries ni pushes de imágenes. El action normal lee
`release-please-config.json` y `.release-please-manifest.json`, donde están
configurados ambos productos.

Antes de habilitar los canales públicos de Rook se debe registrar evidencia de:

1. ownership de npm, Docker Hub y GHCR;
2. GitHub App, npm trusted publishing/provenance y credenciales Docker;
3. targets soportados y sus smoke tests;
4. instalación limpia, checksum y `<binary> --version`;
5. trazabilidad entre artefacto, tag y SHA.

Cargo ownership y publicación crates.io quedan fuera del gate de Rook GA.

Huginn requiere además el rename en `main`, MVP acceptance, evidencia de
Ratatui/PTY y contratos públicos estables `HUGINN_*`. Huginn ya está incluido
en el manifest activo de Release Please, pero no tendrá jobs públicos de
distribución hasta que se cumplan estos gates.

## Reintentos y releases parciales

Las versiones de registries y assets de GitHub son inmutables. Un reintento:

1. reutiliza el mismo tag y SHA;
2. comprueba qué outputs ya existen;
3. publica únicamente los faltantes;
4. vuelve a verificar checksums y `--version`;
5. nunca incrementa la versión solo para recuperar un job parcial.

Un release parcial es un fallo visible, no un éxito.

## Fixtures de independencia

Ejecutar localmente:

```bash
node scripts/check-release-config.mjs
node scripts/release-fixtures.mjs
```

Estos fixtures cubren cambios solo de Rook, solo de Huginn, cambios
simultáneos, paths fuera de los paquetes configurados y aislamiento entre
productos. Validan los prefijos de paths del config activo; la propagación de
dependencias mediante `cargo-workspace` requiere el dry-run real de Release Please; el workflow real sigue siendo la validación final. El job `workflow_dispatch` con
`dry_run: true` valida sintaxis y fixtures, pero no ejecuta Release Please. Para
probar el cálculo real sin escrituras, usar una versión de CLI fijada, un
checkout con historial y tags representativos y el config/manifest activos:

```bash
npx --yes release-please@<VERSION> release-pr \
  --repo-url=dallay/cortex \
  --config-file=release-please-config.json \
  --manifest-file=.release-please-manifest.json \
  --dry-run
```

El comando requiere contexto de repositorio Git y puede consultar GitHub; no
proporcionar credenciales de escritura. El workflow actual solo parsea los
archivos y ejecuta fixtures, no invoca este CLI.

## Agregar un producto futuro

Agregar Muninn u otro producto debe ser declarativo:

1. cerrar primero el rename y el contrato público;
2. añadir su path y versión al manifest;
3. añadir una entrada con component, changelog y metadata versionada;
4. añadir jobs que consuman únicamente sus outputs path-scoped y SHA;
5. añadir fixtures, smoke tests e identidades de canales;
6. verificar ownership y credenciales antes de habilitar writes.

No se crea una versión global de Cortex ni se copia un workflow monolítico sin
cambiar identidad, targets, contrato de contenedor y gates del producto. Huginn
ya participa en release PRs de Release Please; sus jobs de distribución se
habilitarán después de DALLAY-634 y del preflight del canal.
