# Security Policy

Cortex is a public monorepo containing Rook, Huginn, and shared or experimental
components. Security reports are welcome for the code in this repository, but
receiving a report is not a promise to maintain every build or version.

## Supported Versions

**Current pre-GA position (18 November 2026):** No public GitHub Releases are
published for Rook or Huginn. The
[Release Please manifest](.release-please-manifest.json) tracks `0.0.1` for
each product; a manifest version is not evidence of a released, supported
distribution.

| Product | Public release status | Security maintenance commitment |
|---------|-----------------------|---------------------------------|
| Rook | No published GitHub Release | No supported released version; reports about current development code are welcome |
| Huginn | No published GitHub Release | No supported released version; reports about current development code are welcome |

Cortex is an ecosystem and repository, **not** a separately versioned product.
Other components, including Muninn and internal crates, are within the scope of
security reports but have no independently announced support window. We do not
currently promise fixes, backports, or a security update schedule for particular
commits, pre-release builds, or historical versions.

Each product will define its own supported versions and maintenance window
before an announced public support commitment. Until then, follow the
[release and distribution contract](docs/releasing.md) for release gates and
the [GitHub Releases page](https://github.com/dallay/cortex/releases) for
published artifacts. Do not infer a support guarantee from a manifest entry,
tag, CI build, or source installation.

## Reporting a Vulnerability

**Do not publish exploit details, credentials, reproduction payloads, or
sensitive logs in GitHub Issues, pull requests, or Discussions.**

1. Visit [Cortex Security Advisories](https://github.com/dallay/cortex/security/advisories).
2. If **Report a vulnerability** is available, submit the report privately.
   Include the affected product, commit or build reference, impact, reproduction
   steps, and a minimal proof of concept, without including live secrets.
3. If the private-reporting option is **not** available, open a
   [public issue](https://github.com/dallay/cortex/issues/new) **only to
   request a private reporting channel**. Do not describe the vulnerability
   publicly. Maintainers should arrange a private contact method before asking
   for technical details.

**Maintainer configuration check:** The presence of a Security Advisories URL
does **not** prove that private vulnerability reporting is enabled. A repository
administrator must confirm the **Private vulnerability reporting** setting and
test the external reporter flow. This check is still required; no functioning
private intake channel is asserted by this document.

### Response and Disclosure

Maintainers will review actionable reports and coordinate next steps with the
reporter when a private channel is available. Prioritization depends on impact,
exploitability, affected surfaces, maintainer capacity, and release readiness.
**There is currently no guaranteed acknowledgment, fix, or disclosure SLA.**

Where appropriate, maintainers may prepare a fix privately, communicate
mitigations, and coordinate an advisory and disclosure with the reporter.
Publishing a patch or GitHub Security Advisory requires an actual validated
fix and the relevant product's release authorization; neither is automatic.
Never disclose a reporter's identity or reproduction details without
permission. Maintainers should record ownership, escalation, and feasible
response targets before publicly promising specific deadlines.

## Scope

Security findings are in scope when they demonstrate a real trust-boundary
failure in repository-maintained components, including:

- **Rook:** HTTP API and authentication, provider routing, API keys, admin
  sessions, request logging, and the Vue management dashboard in
  [`apps/rook/`](apps/rook/).
- **Huginn:** local agent tools, command execution, filesystem access, MCP
  integrations, permissions, and persisted sessions in
  [`apps/huginn/`](apps/huginn/) and [`crates/huginn/`](crates/huginn/).
- **Muninn and shared code:** memory, data processing, and common components
  under [`apps/muninn/`](apps/muninn/) and [`crates/`](crates/).
- **Supply chain:** [`third_party/`](third_party/), dependencies,
  Dockerfiles, development images, and
  [GitHub Actions workflows](.github/workflows/).

Examples include credential disclosure, authentication or authorization bypass,
unintended code execution, unsafe path handling, privilege escalation, and
prompt injection **when it crosses an implemented tool or permission boundary**.

Reports without a demonstrated security impact, purely hypothetical prompt
injections, social engineering, and denial-of-service against unrelated
third-party infrastructure are generally outside this repository's security
triage process. Report ordinary bugs via the
[issue templates](.github/ISSUE_TEMPLATE/). Do not use a public bug report for
an exploitable vulnerability.

## Trust Boundaries and Contributor Guidance

### Secrets and Dependencies

- Never commit API keys, tokens, passwords, credentials, or sensitive session
  data. Use scoped environment variables or approved secret storage.
- Treat accidentally committed credentials as compromised; revoke and rotate
  them. Removing the line from a later commit does not revoke the credential.
- Review dependency updates and lockfile changes in
  [`Cargo.lock`](Cargo.lock) and [`pnpm-lock.yaml`](pnpm-lock.yaml).
  [Renovate](renovate.json) opens dependency and vulnerability-update PRs; it is
  **not** an approval or release gate.
- The Cargo three-day publish-age cooldown is configured in
  [`.cargo/config.toml`](.cargo/config.toml). It reduces early adoption of
  suspicious releases but is **not** a substitute for audits or review; it
  does not protect every installation command. See the
  [development guide](docs/development.md#dependency-cooldown).

### Rook: HTTP, Authentication, and Configuration

- Apply authorization and validation at Rook's backend/API boundaries; a
  client-side route or dashboard control is not a security boundary.
- **Persistent client API-key authentication is disabled by default** in the
  documented configuration; deployments must explicitly assess and enable
  suitable controls before exposing the API. See
  [Rook authentication configuration](docs/rook/configuration.md#authapi_keys).
- Protect `API_KEY_HASH_SECRET`, admin credentials, upstream provider
  secrets, SQLite databases, and logs. Never log a raw bearer token.
- Validate provider destinations, request input, API key restrictions, and
  untrusted external response data before routing, storing, or executing
  anything. Use least-privilege credentials and review any public exposure.

### Huginn: Tools, Model Output, and Local Authority

- Treat model-generated commands, workspace files, prompts, skill content, and
  MCP server output as **untrusted input**, not user authorization.
- Require the documented permission decisions for tool invocations. Never
  bypass approval or broaden filesystem/network access because an LLM or a
  retrieved document requests it.
- The documented macOS Seatbelt profile applies to **`run_shell` and its
  child processes only**. Native tools and MCP servers are outside that
  sandbox, and Linux/Windows do not have equivalent isolation from this
  feature. Do not claim the entire agent is sandboxed. See
  [Huginn shell isolation](docs/huginn/README.md#shell-command-isolation).
- Respect workspace boundaries, audit tool inputs/outputs, and handle local
  SQLite session data as potentially sensitive. The session store does not
  provide built-in encryption.

## Security Controls: Verified Configuration vs. Guarantees

The table reflects the **configured workflows**, not a claim that every run
is successful or that every GitHub security setting is enabled.

| Control | Current scope and enforcement |
|---------|-------------------------------|
| `cargo audit` | `audit` CI job checks Rust dependencies; runs for backend or GitHub changes and participates in the required CI result |
| Trivy | `trivy-fs` scans filesystem dependencies and misconfigurations for HIGH/CRITICAL findings, ignoring unfixed vulnerabilities; runs when relevant paths change |
| Gitleaks | `gitleaks-pr` fails on secrets in the PR commit range (or history on push); separate history scans are reporting-only |
| Semgrep | `semgrep-pr` scans Rust, Dockerfile, and GitHub Actions rules for ERROR-severity findings on relevant changes; not a general TypeScript/Vue SAST guarantee |
| GitGuardian | A separate reusable workflow runs for push and eligible PRs, but excludes fork PRs and Dependabot PRs; effectiveness depends on the configured secret and shared workflow |
| SonarCloud | Conditional integration; requires the relevant secret/service configuration and is not evidence of a completed security audit |
| Renovate | Dependency PRs, OSV alerts, and lockfile maintenance; review and merge are separate human or repository-policy decisions |

Source of truth: [`ci.yml`](.github/workflows/ci.yml),
[`gitguardian-scan.yml`](.github/workflows/gitguardian-scan.yml),
[`security-deep.yml`](.github/workflows/security-deep.yml), and
[`renovate.json`](renovate.json).

The `security-deep.yml` workflow is scheduled **Monday–Friday at 03:17 UTC**
(not every night of the week). Its full-history Gitleaks, broader Semgrep,
and Trivy scans publish reports/SARIF but do **not** block merges. The current
CI workflow does **not** define a dedicated `pnpm audit` merge gate;
frontend dependency risk should not be described as covered by such a job.

The [release workflow](.github/workflows/release.yml) defines tagged source
verification, product-specific builds and channel gates. These are intended
controls, **not proof of published artifacts or reproducible builds**.
Follow [`docs/releasing.md`](docs/releasing.md) and verify the release
preflight before making distribution security claims.

## Incident Handling

For a credible private report, the maintainers should:

1. Establish a private communication channel and confirm the affected product,
   version or commit, impact, and reproducibility.
2. Assess severity and immediate mitigations without sharing exploit details
   publicly. Rotate exposed secrets if applicable.
3. Assign an owner, reproduce and fix the issue, and verify the remediation
   using focused tests and relevant security scans.
4. Decide whether to release a fix for an authorized distribution channel,
   then coordinate public disclosure and a GitHub Security Advisory if
   appropriate.

No SOC 2, ISO 27001, or other formal security certification is claimed.

## Related Resources

- [Cortex Security Advisories](https://github.com/dallay/cortex/security/advisories)
- [GitHub code scanning](https://github.com/dallay/cortex/security/code-scanning)
- [Dependency management](renovate.json)
- [Development and test guide](docs/development.md)
- [Release and distribution contract](docs/releasing.md)
