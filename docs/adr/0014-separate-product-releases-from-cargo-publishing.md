# ADR-0014: Separar releases de producto de publicación Cargo

- **Estado:** Aceptado
- **Fecha:** 2026-10-11
- **Alcance:** Rook GA, Huginn posterior y futuros productos Cortex

## Contexto

Cortex contiene productos y crates internos con boundaries técnicos distintos.
Rook puede distribuirse como binario mediante GitHub Releases, npm y Docker sin
hacer públicos todos sus crates internos.

La publicación de `rook` en crates.io no es equivalente al release del
producto: Cargo exige que el cierre transitivo de dependencias esté disponible
en un registry. Eso convertiría crates técnicos como `shared-kernel`,
`db-migration` y `providers-core` en unidades públicas de versionado, aunque no
sean productos para usuarios finales.

## Decisión

Adoptamos **Opción C para Rook GA**:

- GitHub Releases, npm y Docker Hub/GHCR son canales obligatorios.
- `cargo install --git` desde el tag exacto es el canal Rust soportado.
- `cargo install rook` desde crates.io no bloquea el primer lanzamiento.
- Los crates internos conservan su privacidad/publicabilidad actual.
- Huginn no se publica hasta superar sus acceptance gates.
- La publicación en crates.io se conserva como una evolución explícita, no se
  descarta.

## Alternativas consideradas

### Publicar el cierre transitivo desde el principio

Es técnicamente válido y será la opción futura si crates.io demuestra ser un
canal estratégico. Requiere verificar ownership, posiblemente renombrar crates
genéricos y mantener versiones inmutables de cada dependencia.

### Reestructurar Rook para que sea un crate autocontenido

No se adopta. Cambiaría Clean Architecture y boundaries técnicos para satisfacer
una restricción de packaging, no una necesidad del dominio.

## Consecuencias

### Positivas

- Rook alcanza GA sin publicar internals prematuramente.
- Se preserva la arquitectura actual.
- Los artefactos distribuidos se pueden verificar por tag y SHA.
- La decisión de crates.io queda reversible sin imponer deuda arquitectónica.

### Negativas

- `cargo install rook` no está disponible desde crates.io en GA.
- La instalación Rust requiere Git y acceso al repositorio.
- La fase posterior deberá calcular y publicar el cierre transitivo real.

## Reglas para la fase crates.io

Cuando se active:

1. calcular solo el cierre transitivo usado por Rook;
2. verificar ownership y colisiones de nombres;
3. preferir nombres con namespace reconocible, como `cortex-shared-kernel`;
4. publicar en orden topológico;
5. recordar que cada publicación es inmutable y cada fix requiere una nueva
   versión aunque no cambien API ni grafo de dependencias.

## Separación de configuración, activación y distribución

La preparación técnica y la activación operativa de Release Please no requieren
que el producto ya esté aceptado: Release Please puede abrir release PRs, pero
el usuario revisa y decide si las mergea. Se mantienen tres fases:

1. **Configuración y validación:** preparar el config/manifest multi-producto y
   comprobar su independencia.
2. **Release PRs:** Release Please queda activo para Rook y Huginn. El usuario
   conserva el control del merge; al mergear se crean tags y GitHub Releases
   conforme al flujo normal.
3. **Distribución:** publicar binarios o paquetes en cada canal requiere el
   gate de aceptación del producto y el preflight de ese canal.

La activación de Release Please no equivale a autorizar distribución. Los jobs
actuales de build y publicación siguen conectados solo a Rook. DALLAY-630,
DALLAY-665 y DALLAY-666 se rastrean como evidencia del MVP en DALLAY-634, no
como gates independientes adicionales.

## No objetivos

- No convertir crates internos en productos independientes.
- No crear tags de producto para cada crate técnico.
- No reorganizar el dominio solo para habilitar Cargo.
- La distribución pública de Huginn requiere DALLAY-634 y el preflight del canal.
