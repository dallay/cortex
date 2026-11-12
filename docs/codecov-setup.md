# Integración de Codecov

Este documento describe los reportes de cobertura, Bundle Analysis y Test Analytics de Cortex.
El workflow de CI está en `.github/workflows/ci.yml` y la configuración del servicio está en
`codecov.yml`.

## Cobertura de código

Codecov recibe reportes separados para backend (`backend`) y dashboard (`frontend`). El archivo
`codecov.yml` configura objetivos distintos para ambos flags, carryforward y comentarios de PR.
Los jobs `coverage` y `coverage-frontend` suben los reportes LCOV mediante el action Codecov v7.

Los flags cubren estas rutas y objetivos:

- `backend`: `apps/rook/src/**`, `apps/rook/tests/**` y `crates/**`; objetivo de proyecto del 85% y
  objetivo de patch del 85%.
- `frontend`: `apps/rook/dashboard/**`; objetivo de proyecto del 80% y objetivo de patch del 75%.
- Los flags usan carryforward para conservar el último reporte cuando un job no corre por el filtro
  de rutas.

El comentario de PR muestra resumen, diff, flags, componentes, archivos y pie. `codecov.yml` espera
un reporte con `after_n_builds: 1`; carryforward conserva el último flag omitido y una segunda subida
actualiza el comentario existente.

## Componentes por producto

`component_management` define tres componentes que reparten la cobertura por producto: `rook`,
`huginn` y `dashboard`. Los componentes son filtros virtuales definidos solo en `codecov.yml`: no
requieren subidas adicionales ni cambios en los jobs de CI. El upload Rust ya cubre todo el workspace
con `cargo llvm-cov --workspace`, así que cada componente lo recorta por ruta.

- `rook`: `apps/rook/src/**`, `apps/rook/tests/**`, `crates/domain/**`, `crates/application/**`,
  `crates/infrastructure/**` y `crates/test-support/**`.
- `huginn`: `apps/huginn/src/**`, `apps/huginn/tests/**` y `crates/huginn/**`.
- `dashboard`: `apps/rook/dashboard/**`.

Las rutas de los componentes son disjuntas, así que cada archivo pertenece a un solo componente.
El componente `huginn` cubre código que ningún flag reporta: el flag `backend` declara `crates/**`,
`apps/rook/src/**` y `apps/rook/tests/**`, por lo que `apps/huginn/src/**` y `apps/huginn/tests/**`
entran al reporte Rust pero quedan sin atribución de flag. El componente es la única vista que los
muestra.

Los estados por componente son informativos: `project` con `target: auto` y `patch` con objetivo del
80%, ambos con `informational: true`. Miden sin bloquear merges. Además, `.github/main-ruleset.json`
no exige ningún contexto de Codecov, así que un estado de componente que falle tampoco bloquea el
merge. Cuando exista historial suficiente, se pueden endurecer los objetivos por producto.

Los flags, sus carryforward y los estados de `coverage.status` no cambian: los componentes agregan
visibilidad, no sustituyen los gates.

Las subidas de cobertura usan `fail_ci_if_error: false`. Los umbrales locales de cobertura y el job
`CI / Required` conservan la responsabilidad de bloquear una regresión; una falla temporal del
servicio Codecov no cambia esos gates.

El comentario de cobertura usa `after_n_builds: 1` porque los filtros de ruta pueden omitir uno de
los jobs. Las reglas carryforward conservan el último reporte válido para el flag omitido. Si el
segundo reporte también llega, Codecov actualiza el comentario existente. La opción
`show_carryforward_flags: true` muestra una columna para identificar los flags con datos carryforward.

## Bundle Analysis

El dashboard usa `@codecov/vite-plugin` en `apps/rook/dashboard/vite.config.ts`. El plugin aparece
después de los demás plugins de Vite y publica el bundle con el nombre `rook-dashboard`.

El job `build-dashboard` activa el plugin cuando el detector marca cambios frontend. Entrega
`CODECOV_TOKEN` al paso de build en pushes y PRs humanos de la misma repo. Los PRs de Dependabot no
ejecutan Bundle Analysis porque no reciben los secretos de GitHub Actions. Los PRs de forks tampoco
reciben el secreto; `gitService: "github"` permite el modo tokenless de Codecov para el análisis del
bundle en este repositorio público. Los builds locales y los builds sin cambios frontend no suben
bundles.

`codecov.yml` define un umbral de cambio del 5% y un estado `informational`. Bundle Analysis informa
el tamaño del bundle; no bloquea la fusión.

Los PRs de forks sí pueden usar el modo tokenless para publicar el reporte del bundle.

## Test Analytics

Codecov Test Analytics acepta resultados JUnit XML. CI produce dos reportes principales:

- **Rust (Linux):** el job principal `Test (Rust)` usa `cargo-nextest` para ejecutar las pruebas
  unitarias y de integración con el perfil `ci` de `.config/nextest.toml`. El reporte se escribe en
  `target/nextest/ci/junit.xml`. Nextest no ejecuta doctests; CI los conserva con
  `cargo test --workspace --all-features --doc`.
- **Dashboard:** Vitest escribe `apps/rook/dashboard/junit.xml` con los reporters `default` y
  `junit`, para conservar detalles en la consola y generar el archivo XML.

El workflow sube ambos archivos con `codecov/codecov-action` v7 y `report_type: test_results`. Usa
los flags `backend` y `frontend` para distinguir cada suite. Cada paso de subida usa
`fail_ci_if_error: false`, así que un fallo de Codecov no reemplaza ni debilita los tests requeridos.

Las cargas de Test Analytics se ejecutan aunque una suite falle, excepto si el job fue cancelado. Se
omiten en PRs de forks y de Dependabot porque esos eventos no reciben `CODECOV_TOKEN`. Los jobs Rust
específicos de macOS/Windows, la suite auxiliar de Node y las pruebas E2E siguen ejecutándose, pero
quedan fuera de esta integración.

CI conserva cada archivo JUnit como artifact por siete días, incluso en PRs de forks y de Dependabot.
La subida del artifact es no bloqueante.

El workflow usa el action Codecov v7, que admite `report_type: test_results`. No usa
`codecov/test-results-action`, porque [ese repositorio está archivado](https://github.com/codecov/test-results-action).

## Validación local

Ejecuta estos comandos desde la raíz del repositorio:

```sh
cargo install cargo-nextest --locked --version 0.9.151
node --test .github/scripts/ci-workflow.test.cjs
cargo nextest run --workspace --all-features --profile ci
cargo nextest run -p shared-kernel --all-features --profile ci
cargo test --workspace --all-features --doc
pnpm --filter @dallay/rook-dashboard exec vitest run --reporter=default --reporter=junit --outputFile.junit=junit.xml
curl -s --data-binary "@codecov.yml" -H "Content-Type: application/yaml" https://api.codecov.io/validate
```

El último comando crea `apps/rook/dashboard/junit.xml`. La configuración de bundle se verifica en
CI, donde el plugin se activa solo para cambios frontend. El comando de `curl` valida `codecov.yml`
contra el validador oficial de Codecov y responde `Valid!` cuando el esquema es correcto.

## Diagnóstico

- **Falta el comentario de cobertura:** comprueba que al menos un job de cobertura terminó y que el
  comentario tiene `after_n_builds: 1`.
- **Falta el reporte de bundle:** comprueba que los filtros marcaron cambios frontend. Los PRs de
  forks usan el modo tokenless; los demás builds requieren el secreto `CODECOV_TOKEN`.
- **Faltan resultados de tests:** comprueba que existe el archivo JUnit de cada runner y que la
  subida usa `report_type: test_results`. Los PRs de forks no suben resultados a Test Analytics, pero
  conservan los XML como artifacts de CI.
- **Falla un estado de cobertura:** revisa el objetivo y el umbral correspondiente en `codecov.yml`.
  No cambies esos valores para compensar un reporte ausente.
- **Un componente aparece vacío:** confirma que las rutas del componente coinciden con las rutas que
  reporta el upload. Un componente solo muestra código con cobertura en un reporte recibido.

## Verificación en Codecov

Después de una ejecución de CI:

1. Revisa los logs de `Build Dashboard` para confirmar que el plugin publicó el bundle.
2. Revisa los logs de los dos pasos `Upload ... test results to Codecov` para confirmar que los
   archivos JUnit se aceptaron.
3. En Codecov, abre la vista de bundles para consultar tamaño total, tamaño comprimido y cambios por
   commit o PR.
4. En Codecov, abre la pestaña **Tests** para revisar duración, fallos y tendencias de las pruebas.

Si falta un reporte, confirma primero que el job de tests generó el archivo JUnit esperado y que la
subida de Test Analytics tuvo acceso a `CODECOV_TOKEN`. Los PRs de forks publican Bundle Analysis sin
token, pero no publican resultados de Test Analytics.

Para consultar los contratos externos, revisa la [guía de Bundle Analysis de Codecov], la
[guía de Test Analytics de Codecov] y la [referencia del action Codecov].

[guía de Bundle Analysis de Codecov]: https://docs.codecov.com/docs/bundle-analysis
[guía de Test Analytics de Codecov]: https://docs.codecov.com/docs/test-analytics
[referencia del action Codecov]: https://github.com/codecov/codecov-action
