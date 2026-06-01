# Security Policy

## Supported Versions

We actively maintain security updates for the following versions. We recommend always using the latest stable release.

| Version    | Supported          |
| --------- | ------------------ |
| `>= 2.x`  | :white_check_mark: |
| `1.x`     | :white_check_mark: |
| `< 1.x`   | :x:                |

### Release Cadence

- **Major versions** (`2.x`, `3.x`): Supported for 12 months after the next major is released.
- **Minor versions** (`2.1.x`): Supported while the minor is current or previous.
- **Patch versions** (`2.1.1`): Supported within the same minor as the latest patch.

Subscribe to [GitHub Releases](https://github.com/dallay/cortex/releases) for alerts.

---

## Reporting a Vulnerability

**Please do not report security vulnerabilities through public GitHub Issues.**

Send a report to **privately via GitHub Security Advisories**:

1. Navigate to [the repository's Security tab](https://github.com/dallay/cortex/security/advisories).
2. Click **Report a vulnerability**.
3. Fill out the advisory form — we respond within **48 hours** with an acknowledgment.
4. Provide as much detail as possible: reproduction steps, affected versions, any potential fixes.

### What to Expect After Reporting

| Timeline | What Happens |
| -------- | ------------ |
| **< 48 hours** | Initial acknowledgment from the maintainers. |
| **< 7 days** | Preliminary severity assessment (Critical / High / Medium / Low). |
| **< 30 days** | If accepted: a fix is prepared in a private branch. Patch releasetimeline communicated. |
| **< 60 days** | Public disclosure on a mutually agreed date.COORDinator will reach out if extended timeline is needed. |

### Scope

The policy covers vulnerabilities in the cortex monorepo, including:
- Core packages (`rook` CLI, Rust backend)
- Frontend apps (`apps/`)
- Infrastructure-as-Code configurations
- GitHub Actions workflows

**In-scope**: Remote code execution, privilege escalation, data exfiltration, authentication bypass, dependency chain compromise.

**Out-of-scope**: Social engineering, denial-of-service against third-party infrastructure, pre-disclosure findings from automated scanners.

---

## Security Best Practices for Contributors

### Secrets Management

- **Never commit secrets, credentials, or tokens** to the repository. Use environment variables or GitHub Secrets.
- If a secret is accidentally committed, assume it is compromised and rotate it immediately.
- Use `.gitignore`, `.env.example`, and `git-secrets` or similar tooling.

### Dependency Management

- All Rust dependencies are audited via `cargo audit` in CI (`ci.yml#audit`).
- Frontend dependencies are audited via `pnpm audit` where applicable.
- **Do not** add dependencies with known high/critical vulnerabilities.
- Keep lock files (`Cargo.lock`, `pnpm-lock.yaml`) up to date and committed.

### Input Validation

- Validate and sanitize ALL user input at trust boundaries, especially in:
  - CLI argument parsing (`rook` package)
  - File path handling (path traversal attacks)
  - HTTP API handlers (`apps/*/api`)
  - AI model prompt injection surfaces

### Authentication / Authorization

- Use Vercel Middleware/Routing Middleware for auth at the edge.
- Never roll custom auth — use established patterns (Clerk, Auth.js, etc.).
- Apply least-privilege scoping on all secrets and API keys.

### Security-Sensitive Code Areas

The following packages/configs receive elevated security scrutiny:

| Package / Config | Reason |
| ---------------- | ------ |
| `crates/rook/` | CLI with file system and git access |
| `apps/rook/dashboard/` | User-facing web app with auth |
| `.github/workflows/` | CI/CD with secrets access |
| `infra/` | Cloud infrastructure definitions |

---

## Dependency Security

### Automated Scanning

The project uses multiple layers of automated vulnerability scanning:

| Tool | Scope | Schedule | Fail-Gate |
| ---- | ----- | -------- | --------- |
| **cargo audit** | Rust dependencies | Every PR/commit | :white_check_mark: Yes |
| **Dependabot** | `Cargo.lock`, `pnpm-lock.yaml` | On lockfile changes | :white_check_mark: Yes (auto-merge for patch/security) |
| **Gitleaks** | Repository history + on-push | Nightly (scheduled) | :x: Reporting only |
| **Semgrep** | Rust, Docker, GitHub Actions, secrets | Nightly (scheduled) | :x: Reporting only |
| **Trivy** | Filesystem, dependencies, IaC | Nightly (scheduled) | :x: Reporting only |
| **SonarCloud** | Code quality + security hotspots | On PR (if token set) | Conditional |

### Keeping Dependencies Updated

- **GitHub Dependabot** creates PRs for outdated dependencies automatically.
- Security updates are merged quickly; feature/minor updates follow regular review cadence.
- We enable **automated security updates** for critical CVEs via Dependabot.

---

## Incident Response

When a vulnerability is reported or discovered:

1. **Triage** — The maintainer team assesses severity within 48 hours.
2. **Private fix** — A fix is developed in an private fork/branch.
3. **Coordinated disclosure** — A patch is prepared with a target disclosure date.
4. **Patch release** — A patch version (`x.y.z`) is tagged and released.
5. **Public disclosure** — A GitHub Security Advisory is published with the full write-up.

### Severity Classification

| Level | Definition | Response Time |
| ----- | ---------- | ------------- |
| **Critical** | Remote code execution,彻底绕过认证 | < 24 hours for initial mitigation |
| **High** | Data exfiltration, privilege escalation | < 7 days for patch |
| **Medium** | Information disclosure, DoS | < 30 days for patch |
| **Low** | Minor impact, hard to exploit | Next release cycle |

---

## Compliance & Standards

This project follows:

- **Secure by design** principles: minimal dependency surface, defense in depth.
- **Dependency audit** before each release via `cargo audit` and `pnpm audit`.
- **Secret scanning** via Gitleaks on the repository full history.
- **Reproducible builds**: Linux, macOS, and Windows binaries are built from verified build pipelines.

No formal certifications currently (SOC2, ISO 27001, etc.).

---

## Security-Related Links

| Resource | Link |
| -------- | ---- |
| Report a vulnerability | [GitHub Security Advisories](https://github.com/dallay/cortex/security/advisories) |
| Code scanning results | [GitHub Code Scanning](https://github.com/dallay/cortex/security/code-scanning) |
| Dependabot alerts | [Dependabot Alerts](https://github.com/dallay/cortex/security/dependabot) |
| Secret scanning alerts | [GitHub Secret Scanning](https://github.com/dallay/cortex/security/secrets) |
| CI/CD workflow | [`.github/workflows/ci.yml`](.github/workflows/ci.yml) |
| Nightly security deep scan | [`.github/workflows/security-deep.yml`](.github/workflows/security-deep.yml) |
