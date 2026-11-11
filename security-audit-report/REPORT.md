# Security Audit Report — Cortex Monorepo

## Executive Summary

A comprehensive security audit of the Cortex monorepo (`Rook` AI router & proxy, `Huginn` local coding agent, shared crates, and frontend dashboard) was executed using the Cloudflare `security-audit` skill workflow.

The audit verified trust boundaries, authorization layers, memory safety invariants, agent execution guardrails, web transports, and third-party dependencies.

### Findings Breakdown
- **Critical / High Severity Vulnerabilities in Core Runtime**: **0**
- **Supply Chain / Dependency Advisories**:
  - **Cargo (Rust)**: 0 active vulnerabilities; 3 unmaintained crate warnings (`zbus_names`, `zvariant`, `zvariant_derive` migrating into `zbus 6.0`).
  - **NPM (Frontend / Dev Tools)**: 5 indirect advisories (2 High, 2 Moderate, 1 Low) in dev dependencies (`zod`, `re2js`, `glob`, `minimatch`, `katex`).
- **CI / Workflow Integrity**:
  - Multiple GitHub Actions invocations in `.github/workflows/security-deep.yml` and `.github/workflows/ci.yml` lack tag comment annotations or commit SHA pinning.
- **Web Protocol & Authentication**:
  - `CsrfGuard` double-submit cookie validation verified on all state-changing management routes.
  - Multi-tiered API key rate limiting and login rate limiting properly enforced.
  - Robust SSRF protections verified in `EndpointPolicy` against private networks, loopbacks, link-local, and cloud metadata addresses.
- **Agent Sandbox & Guardrails**:
  - `Huginn` shell lexical subset restricts command expansions, subshells, variables, and unescaped operators.
  - Path normalization and sensitive file filters protect credentials and system directories.

---

## Audit Artifacts

- **`findings.json`**: Structured findings validated with `validate-findings.cjs`.
- **`coverage-ledger.json`**: Coverage ledger validated with `validate-coverage-ledger.cjs`.
- **`FINDINGS-DETAIL.md`**: Detailed technical analysis and remediation steps.
