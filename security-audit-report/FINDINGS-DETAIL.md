# Detailed Findings and Recommendations

## 1. Supply Chain & Dependencies

### Cargo Advisory Warnings (RustSec)
- **Crates affected**: `zbus_names 4.3.5`, `zvariant 5.15.1`, `zvariant_derive 5.15.1`.
- **Advisories**: RUSTSEC-2026-0371, RUSTSEC-2026-0370, RUSTSEC-2026-0372.
- **Impact**: Upstream crates were deprecated and consolidated into `zbus 6.0`. No active exploits or memory safety bugs reported.
- **Remediation**: Migrate `secret-service` / `keyring` usage to direct `zbus` 6.0 when available.

### NPM DevDependency Advisories
- **Packages affected**:
  - `zod <4.6.6` (High, GHSA-mrhx-c7xc-m3w5) via `@codecov/vite-plugin`.
  - `re2js >=2.2.0 <2.8.7` (High, GHSA-gg6h-894w-4m9c) via `markdownlint`.
  - `glob >=9.0.0 <=14.0.1` (Moderate, GHSA-3g3m-2fjr-6pw4) via `@vue/test-utils`.
  - `minimatch >=7.0.0 <11.0.1` (Moderate, GHSA-g3hv-75wg-m249) via `@vue/test-utils`.
  - `katex >=0.11.0 <0.18.2` (Low, GHSA-238p-pmpm-9mq7) via `markdownlint`.
- **Impact**: Build/test-time denial of service or validation stalls; non-production runtime exposure.
- **Remediation**: Run `pnpm update` and bump `markdownlint-cli2` and `@vue/test-utils` once upstream patches are published.

## 2. GitHub Actions Pinning

- **Observations**: Several steps in `.github/workflows/security-deep.yml` and `.github/workflows/ci.yml` lack the `# v...` comment explaining the pinned commit SHA.
- **Remediation**: Use `.agents/skills/pinned-tag/scripts/resolve-action-sha.sh` to pin and comment all action steps.

## 3. Web Protocol & Memory Safety Validation

- **Memory Safety**: No `unsafe` code found in core services; Rust's type system and ownership guarantees are fully respected.
- **SSRF Defenses**: `EndpointPolicy` validates schemes, ports, IP classifications (Loopback, LinkLocal, Private, Metadata), and verifies DNS responses at connection time.
- **CSRF & CORS**: `CsrfGuard` and `AuthzConfig` prevent cross-origin abuse and wildcard CORS on cookie-authenticated management endpoints.
