# Security Policy

Cortex is a public monorepo containing Rook, Huginn, and shared or experimental
components. Security reports are welcome for the code in this repository, but
receiving a report is not a promise to maintain every build or version.

## Supported Versions

**Current pre-GA position (verified 23 November 2026):** No public GitHub
Releases and no release tags are published for Rook or Huginn. The
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
2. Select **Report a vulnerability** and submit the report privately. Include
   the affected product, commit or build reference, impact, reproduction steps,
   and a minimal proof of concept, without including live secrets.
3. If **Report a vulnerability** is **not** available, open a
   [public issue](https://github.com/dallay/cortex/issues/new) **only to
   request a private reporting channel**. Do not describe the vulnerability
   publicly. The maintainer set should arrange a private contact method before
   asking for technical details.

**Private reporting status:** Private vulnerability reporting is **enabled**
for this repository, verified on 23 November 2026 through the repository API.
A visible Security Advisories URL does not prove the setting is on, so a
repository administrator must re-confirm it after any change of repository
ownership or administrator, and must test the external reporter flow with a
report that never leaves the advisory draft.

### Triage Ownership

Cortex has no dedicated security team and no security-specific mailbox. Security
triage is owned by the repository maintainers: the repository owner (`dallay`)
and the maintainers with merge and release access, currently Yuniel Acosta
(`@yuniel-acosta`). [`CONTRIBUTING.md`](CONTRIBUTING.md) requires at least one
maintainer approval before a merge, and the same maintainer set authorizes
releases through [the release contract](docs/releasing.md).

Every commitment below is made by that maintainer set. No other party is
authorized to promise a fix, a disclosure date, or a backport on behalf of
Cortex.

### Response and Disclosure

The maintainer set reviews actionable reports and coordinates next steps with
the reporter when a private channel is available. Prioritization depends on
impact, exploitability, affected surfaces, maintainer capacity, and release
readiness. **There is currently no guaranteed acknowledgment, fix, or disclosure
SLA, and no response-time commitment of any kind is published.**

The triage process for a credible private report is:

1. The maintainer set acknowledges the report privately and confirms the
   affected product, version or commit, impact, and reproducibility.
2. The maintainer set assesses severity, records a triage owner, and rotates
   exposed secrets when applicable.
3. The maintainer set reproduces the issue, implements the fix with a
   regression test, and verifies remediation with focused tests and the
   security scans listed below.
4. The maintainer set decides whether to release a fix through an authorized
   distribution channel, then coordinates public disclosure and a GitHub
   Security Advisory with the reporter.

Publishing a patch or GitHub Security Advisory requires an actual validated
fix and the relevant product's release authorization; neither is automatic.
Never disclose a reporter's identity or reproduction details without
permission. Deadlines are never announced publicly until the maintainer set
owns a fix for them.

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
is successful or that every GitHub security setting is enabled. Each row states
whether the control **blocks** on a finding or only **reports** one.

| Control | Current scope and enforcement |
|---------|-------------------------------|
| `cargo audit` | Job `audit` in `ci.yml` checks Rust dependencies when backend or GitHub files change. **Blocking**: the job fails on a vulnerability, with one documented exception — `RUSTSEC-2026-0594` (`cookie < 0.18.3`) is ignored until the workspace publish-age cooldown allows `Cargo.lock` to be re-resolved. |
| Semgrep | Job `semgrep-pr` scans Rust, Dockerfile, and GitHub Actions rules at `--severity ERROR` when backend, Docker, or GitHub files change. **Blocking**: an explicit step fails the job on unsuppressed findings. This is not a general TypeScript/Vue SAST guarantee. |
| Gitleaks | Job `gitleaks-pr` scans the pull-request commit range, or the full history on a push, with `--exit-code 1`. **Blocking**. The separate history scans in `security-deep.yml` are reporting-only. |
| Trivy | Job `trivy-fs` scans filesystem dependencies and misconfigurations at `HIGH,CRITICAL` severity with `ignore-unfixed: true` when backend, frontend, Docker, or GitHub files change. **Reporting-only**: the action runs with `exit-code: '0'`, so findings are published as SARIF and do not fail the job. |
| OSV-Scanner | A separate workflow ([`osv-scanner.yml`](.github/workflows/osv-scanner.yml)) scans `Cargo.lock`, `pnpm-lock.yaml`, and other lockfiles on pull requests, pushes, merge queues, and a weekly Tuesday 03:17 UTC schedule. Its pull-request job is designed to fail on newly introduced vulnerabilities, but it is **not** part of the `CI / Required` aggregate in `ci.yml`; whether it blocks a merge depends on repository branch-protection settings. |
| GitGuardian | A separate reusable workflow runs for push and eligible PRs, but excludes fork PRs and Dependabot PRs; effectiveness depends on the configured secret and on the shared workflow `dallay/common-actions`. |
| SonarCloud | Job `sonar` fails when `SONAR_TOKEN` is missing, so the analysis is a **blocking** step for pull requests and for backend, frontend, or GitHub changes. The quality-gate wait is deliberately disabled in CI, so the project quality gate does **not** block a merge and SonarCloud is not evidence of a completed security audit. |
| Renovate | Dependency PRs, vulnerability-update PRs, and lockfile maintenance; review and merge are separate human or repository-policy decisions. It is not a scanner and not a release gate. |

Source of truth: [`ci.yml`](.github/workflows/ci.yml),
[`gitguardian-scan.yml`](.github/workflows/gitguardian-scan.yml),
[`osv-scanner.yml`](.github/workflows/osv-scanner.yml),
[`security-deep.yml`](.github/workflows/security-deep.yml), and
[`renovate.json`](renovate.json).

The `CI / Required` job in `ci.yml` is the aggregate merge gate. It requires
`audit`, `trivy-fs`, `gitleaks-pr`, `semgrep-pr`, and `sonar` to succeed when
their path filters select the job, and it fails when a selected job does not
succeed. `trivy-fs` and `sonar` satisfy that requirement without turning their
findings into a blocked merge.

The `security-deep.yml` workflow is scheduled **Monday–Friday at 03:17 UTC**
(not every night of the week). Its full-history Gitleaks, broader Semgrep,
and Trivy scans publish reports/SARIF but do **not** block merges. The current
CI workflow does **not** define a dedicated `pnpm audit` merge gate;
frontend dependency risk should not be described as covered by such a job.

The [release workflow](.github/workflows/release.yml) defines tagged source
verification and product-specific builds. Its npm and container publication
steps require the `RELEASE_PUBLICATION_ENABLED` repository variable; the
Release Please release-creation step does not. These are intended controls,
**not proof of published artifacts or reproducible builds**.
Follow [`docs/releasing.md`](docs/releasing.md) and verify the release
preflight before making distribution security claims.

## Incident Handling

The triage process above is the incident-handling process for the Cortex
maintainer set. It applies to every product in this repository, and to
credential exposure, dependency compromise, and CI control failures in
addition to code-level findings.

No SOC 2, ISO 27001, or other formal security certification is claimed.

## Related Resources

- [Cortex Security Advisories](https://github.com/dallay/cortex/security/advisories)
- [GitHub code scanning](https://github.com/dallay/cortex/security/code-scanning)
- [Dependency management](renovate.json)
- [Development and test guide](docs/development.md)
- [Release and distribution contract](docs/releasing.md)
