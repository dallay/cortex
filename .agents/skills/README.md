# Agent Skills — Cortex

Catálogo de skills de ingeniería y de producto del monorepo. Cada skill vive en
su propio directorio con un `SKILL.md`; las guías Rust heredadas son documentos
sueltos bajo `languages-typing/rust/`.

La validación estructural es obligatoria en CI:

```bash
node scripts/validate-skills.mjs
node --test scripts/validate-skills.test.mjs
```

El validador comprueba frontmatter estructural (incluido `allowed-tools` separado
por espacios), identidad `name`/directorio, duplicados, longitud de descripción,
tamaño y enlaces locales. **No certifica** afirmaciones
técnicas, versiones ni comandos: eso se revisa contra manifiestos, código y tests.

## Descubrimiento: dos consumidores

`.agents/skills/` tiene dos usos distintos y no comparten el mismo mecanismo:

| Consumidor                                           | Qué usa                                       | Alcance               |
|------------------------------------------------------|-----------------------------------------------|-----------------------|
| Agentes de ingeniería (OpenCode, Claude Code, Codex) | Catálogo completo, sincronizado con AgentSync | Cualquier profundidad |
| Huginn (`FilesystemSkillCatalog`)                    | Solo hijos directos de `.agents/skills/`      | Un nivel              |

Consecuencias:

- `pinned-tag/SKILL.md` es **descubrible** por Huginn.
- `tui/ratatui-tui/SKILL.md` es una skill de ingeniería y **no** es descubrible
  por el catálogo actual. Es intencional; cambiar el contrato de descubrimiento
  requiere una decisión de producto y pruebas de profundidad, duplicados y
  enlaces simbólicos.
- Huginn valida hasta 65.536 bytes por archivo, pero `load_skill` tiene un
  presupuesto de 16 KiB para la respuesta JSON. Una skill puede ser **válida** y
  no ser **cargable**; revisa el tamaño antes de añadir material extenso.

## Skills de producto

### [pinned-tag](pinned-tag/SKILL.md)

Fija GitHub Actions a SHAs inmutables y resuelve la referencia correcta (`v4`, `v4.2.2`, tags anotados) con `scripts/resolve-action-sha.sh`.

### [tui/ratatui-tui](tui/ratatui-tui/SKILL.md)

Contratos operativos para la TUI de Huginn: propietario único del terminal,
renderizado inline con Ratatui, layout, Unicode, foco/overlays, ciclo de vida y
pruebas PTY.

## Frontend

Ruta base: `frontend-platform/`.

| Skill                                                                 | Uso                                                               |
|-----------------------------------------------------------------------|-------------------------------------------------------------------|
| [vue](frontend-platform/vue/SKILL.md)                                 | Componentes, composables, stores Pinia, formularios del dashboard |
| [shadcn-vue](frontend-platform/shadcn-vue/SKILL.md)                   | Componentes y registro de shadcn-vue                              |
| [best-practices](frontend-platform/best-practices/SKILL.md)           | Seguridad, compatibilidad, calidad y rendimiento web              |
| [accessibility](frontend-platform/accessibility/SKILL.md)             | WCAG, navegación por teclado, lectores de pantalla                |
| [core-web-vitals](frontend-platform/core-web-vitals/SKILL.md)         | LCP, CLS e INP                                                    |
| [performance](frontend-platform/performance/SKILL.md)                 | Optimización de carga y runtime                                   |
| [seo](frontend-platform/seo/SKILL.md)                                 | Metadatos, datos estructurados, sitemap                           |
| [astro](frontend-platform/astro/SKILL.md)                             | Astro, SSG e islas                                                |
| [frontend-design](frontend-platform/frontend-design/SKILL.md)         | Diseño de componentes y páginas                                   |
| [animate-text](frontend-platform/animate-text/SKILL.md)               | Efectos de texto con nombre                                       |
| [nothing-design](frontend-platform/nothing-design/SKILL.md)           | Estilo Nothing                                                    |
| [modern-web-guidance](frontend-platform/modern-web-guidance/SKILL.md) | Búsqueda de APIs y patrones web actuales                          |
| [chrome-extensions](frontend-platform/chrome-extensions/SKILL.md)     | Extensiones Manifest V3 y Chrome Web Store                        |

## Testing

Ruta base: `testing/`.

| Skill                                                                   | Uso                                            |
|-------------------------------------------------------------------------|------------------------------------------------|
| [vitest](testing/vitest/SKILL.md)                                       | Tests unitarios y de componentes; Vitest 5.0.5 |
| [playwright](testing/playwright/SKILL.md)                               | E2E del dashboard; `apps/rook/dashboard/e2e`   |
| [playwright-best-practices](testing/playwright-best-practices/SKILL.md) | Patrones avanzados de Playwright               |

## Lenguajes y tipado

| Skill                                                               | Uso                                      |
|---------------------------------------------------------------------|------------------------------------------|
| [typescript](languages-typing/typescript/SKILL.md)                  | Tipos, interfaces y genéricos            |
| [rust-anti-patterns](languages-typing/rust/rust-anti-patterns.md)   | Anti-patterns comunes                    |
| [rust-api-design](languages-typing/rust/rust-api-design.md)         | Diseño de APIs públicas                  |
| [rust-async-patterns](languages-typing/rust/rust-async-patterns.md) | Tokio y concurrencia                     |
| [rust-error-handling](languages-typing/rust/rust-error-handling.md) | `CortexError`, `thiserror`, propagación  |
| [rust-linting](languages-typing/rust/rust-linting.md)               | Lints y enforcement                      |
| [rust-documentation](languages-typing/rust/rust-documentation.md)   | Docs públicas, ejemplos y safety         |
| [rust-naming](languages-typing/rust/rust-naming.md)                 | Convenciones de nombres                  |
| [rust-memory](languages-typing/rust/rust-memory.md)                 | Asignación, clonado y zero-copy          |
| [rust-testing](languages-typing/rust/rust-testing.md)               | Tests, doc-tests y benchmarks            |
| [rust-tooling](languages-typing/rust/rust-tooling.md)               | `just`, cobertura, auditoría y debugging |

## Arquitectura del monorepo

```text
crates/
├── domain/
│   ├── shared-kernel/     # CortexError, IDs, sin dependencias de infraestructura
│   └── rook-core/         # Puertos y modelo de dominio
├── application/
│   └── rook-usecases/     # Casos de uso, routing, fallback
├── infrastructure/
│   ├── providers-*/       # Adaptadores de proveedores
│   ├── transport-axum/    # Adaptador HTTP
│   └── …                  # Cache, auditoría, persistencia
├── huginn/                # Agente local: dominio, aplicación, runtime, presentación
└── test-support/          # Utilidades compartidas de test
```

Las skills respetan esta estructura y la dirección de dependencias.

## Reglas de aplicación

1. **Domain (`shared-kernel`, `rook-core`)** — no usar `anyhow`.
    - `CortexError` y errores específicos de dominio.
    - `thiserror` para definiciones.
2. **Application (`rook-usecases`)** — errores tipados.
    - Propagar errores de proveedores.
    - No capturar y ocultar fallos.
3. **Infrastructure (`providers-*`, `transport-axum`)** — adaptar errores.
    - Convertir errores externos a `CortexError`.
    - Documentar errores de APIs externas.
4. **Binarios (`apps/rook`)** — pueden usar `anyhow` en el borde.
    - Contexto adicional, logging y respuesta HTTP.

## Fuentes

- [rust-skills](https://github.com/leonardomso/rust-skills) — colección original
- [Arquitectura Cortex](../../docs/rook/architecture.md)
- [Guía de desarrollo](../../docs/development.md)
