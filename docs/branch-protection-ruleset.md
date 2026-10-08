# Branch Protection Ruleset - main

Este documento describe la configuración de protección de la rama `main` para asegurar la calidad del código antes del merge.

## Reglas Activas

> Estado de migración DALLAY-628: el ruleset #17001295 NO se modifica con el cambio de código. Añadir `CI / Required` como required check solo después de validar el gate en ejecuciones reales y probar un fallo bloqueante desde una identidad sin bypass. Mantener intactos los checks actuales durante toda la transición.

### 1. Protección Básica de Rama
- ✅ **Deletion**: No se puede eliminar la rama `main`
- ✅ **Non-fast-forward**: No se permiten force pushes
- ✅ **Required linear history**: Historial lineal obligatorio (no merge commits)

### 2. Pull Request Requirements
- **Approvals**: 1 aprobación requerida
- **Dismiss stale reviews**: Las reviews se descartan al pushear cambios
- **Last push approval**: Se requiere aprobación después del último push
- **Thread resolution**: Todos los comentarios deben estar resueltos

### 3. Required Status Checks (Obligatorios para Merge)

#### Plan de transición — ruleset #17001295 (pendiente de validación)
1. No retirar ni reemplazar ninguno de los checks ya obligatorios.
2. Ejecutar PRs representativas (docs, frontend, backend y workflows) hasta confirmar `CI / Required` verde, y una PR controlada con check requerido fallido que deje el gate rojo.
3. Probar que esa PR no puede fusionarse con una identidad que no esté en bypass.
4. Solo entonces añadir `CI / Required` a los required checks; mantener los checks existentes al menos durante la transición y retirarlos únicamente en un cambio posterior con evidencia.
5. Revisar bypass `OrganizationAdmin` y `Integration` ID `915548` (confirmar identidad antes de cambiar); ambos aparecen con `bypass_mode: always`. No eliminarlos hasta resolver el acceso de revisión humana.
6. El ruleset requiere una aprobación humana y aprobación del último push. Para un solo mantenedor, mantener por ahora la política actual y documentar el bloqueo operativo; alternativa a acordar antes de eliminar bypass: quitar la aprobación obligatoria o designar un reviewer independiente. Nunca simular la aprobación mediante bypass administrativo.

#### Fast Checks (Linting & Formatting)
- ✅ **Format** - Rust formatting con `cargo fmt`
- ✅ **Markdown** - Linting de archivos `.md`
- ✅ **Clippy** - Linter de Rust (deny warnings)
- ✅ **Check** - `cargo check` workspace

#### Tests (Calidad del Código)
- ✅ **Test (Rust)** - Tests completos del workspace Rust
- ✅ **Test (Frontend)** - Tests Vitest del dashboard Vue.js

#### Security (Merge Blockers)
- ✅ **Security / Trivy** - Escaneo de vulnerabilidades filesystem + deps
- ✅ **Security / Gitleaks** - Detección de secretos en commits
- ✅ **Security / Semgrep** - SAST scan (Rust, Docker, GitHub Actions)
- ✅ **Audit** - `cargo audit` para vulnerabilidades en dependencias

### 4. Checks Opcionales (No Bloquean Merge)

Los siguientes checks corren pero **NO** bloquean el merge:

- **Doc** - Documentación con `cargo doc`
- **Test (E2E)** - Tests E2E Playwright (pesados, solo después de tests unitarios)
- **Coverage (Rust)** - Reporte de cobertura Codecov
- **Coverage (Frontend)** - Cobertura frontend Codecov
- **SonarCloud** - Análisis de calidad (informativo)
- **Build (cross-compile)** - Validación de compilación multiplataforma (solo en push a main)
- **Test (macOS/Windows)** - Tests en otras plataformas (solo en push a main)

## Estrategia de Checks

### Path-Based Execution
Los checks se ejecutan según los archivos modificados:

```yaml
docs-only changes (*.md, docs/**):
  - Format ✅
  - Markdown ✅
  - Security scans ✅
  Time: ~3-5 min

frontend-only changes (apps/rook/dashboard/**):
  - Format ✅
  - Markdown ✅
  - Test (Frontend) ✅
  - Security scans ✅
  - Test (E2E) ⚠️
  Time: ~25 min

backend changes (crates/**, apps/rook/src/**):
  - Format ✅
  - Markdown ✅
  - Clippy ✅
  - Check ✅
  - Test (Rust) ✅
  - Audit ✅
  - Security scans ✅
  - Doc (optional)
  - Coverage (optional)
  Time: ~28-32 min
```

### Fail-Fast Strategy
1. **Fast checks primero** (fmt, markdown) - 1-2 min
2. **Linting paralelo** (clippy, check, security) - 10 min
3. **Tests unitarios** (rust, frontend) - 8 min
4. **Tests E2E** (solo si unitarios pasan) - 15 min

## Bypass Actors

Pueden saltarse las reglas:
- **Organization Admins** - bypass mode: always
- **Release Please Bot** (integration_id: 915548) - bypass mode: always

## Aplicar la Ruleset

```bash
# Ver ruleset actual
gh api repos/dallay/cortex/rulesets/17001295 | jq .

# Actualizar ruleset (requiere permisos de admin)
gh api \
  --method PUT \
  repos/dallay/cortex/rulesets/17001295 \
  --input docs/main-ruleset.json
```

## Notas Importantes

1. **integration_id: 15368** es GitHub Actions
2. **strict_required_status_checks_policy: true** significa que la rama debe estar actualizada con main antes de mergear
3. Los nombres de contexts (`Format`, `Clippy`, etc.) deben coincidir **exactamente** con los `name:` en el workflow CI
4. E2E tests NO bloquean porque son pesados y pueden ser flaky ocasionalmente
5. Cross-compilation builds NO bloquean porque solo validan targets adicionales

## Verificación

Para verificar que un PR cumple con la ruleset:

```bash
# Ver checks de un PR
gh pr checks 116

# Ver status detallado
gh pr view 116 --json statusCheckRollup
```

## Filosofía

**Bloquean merge:**
- Formato correcto (fmt, markdown)
- Compilación exitosa (clippy, check)
- Tests unitarios pasan (rust, frontend)
- Sin vulnerabilidades críticas (trivy, gitleaks, semgrep, audit)

**NO bloquean merge:**
- E2E tests (pesados, pueden ser flaky)
- Coverage (informativo)
- SonarCloud (informativo)
- Docs generation (no crítico para merge)
- Multi-platform builds (validación adicional)

Esta estrategia mantiene la calidad alta mientras permite iteración rápida.
