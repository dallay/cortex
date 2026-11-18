# Contributing to Cortex

Thank you for your interest in contributing to Cortex! We welcome contributions from everyone and want to make the process as easy and transparent as possible.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Ground Rules](#ground-rules)
- [How to Contribute](#how-to-contribute)
  - [Reporting Bugs](#reporting-bugs)
  - [Suggesting Features](#suggesting-features)
  - [Reporting Accessibility Issues](#reporting-accessibility-issues)
  - [Submitting Pull Requests](#submitting-pull-requests)
- [Development Workflow](#development-workflow)
- [Code Style](#code-style)
- [Pull Request Process](#pull-request-process)
- [Accessibility](#accessibility)
- [Issue Templates](#issue-templates)
- [License](#license)

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code. Please report unacceptable behavior to the maintainers.

## Ground Rules

- **Be respectful and inclusive**. We welcome contributors of all backgrounds and skill levels.
- **Use conventional commits**. Follow [Conventional Commits](https://www.conventionalcommits.org/) for commit messages (e.g., `feat: add amazing feature`, `fix: resolve null pointer`).
- **One feature per PR**. Keep pull requests focused on a single change to make reviews faster and clearer.
- **Write tests**. New features and bug fixes should include tests whenever applicable.
- **Update documentation**. If your change affects user-facing behavior, update the relevant documentation.
- **Follow existing conventions**. Match the style, structure, and patterns of the surrounding code.

## How to Contribute

### Reporting Bugs

If you find a bug, please [open an issue](https://github.com/dallay/cortex/issues/new) using the **Bug Report** template. The template guides you through providing:

- What you expected to happen
- What actually happened
- Steps to reproduce the issue
- Your environment (OS, Rust version, etc.)

Before filing, please check existing issues to avoid duplicates.

### Suggesting Features

Have an idea for improving Cortex? [Open an issue](https://github.com/dallay/cortex/issues/new) using the **Feature Request** template. Include:

- A clear description of the problem or need
- Proposed solution or approach
- Any relevant context or examples

### Reporting Accessibility Issues

Cortex is committed to accessibility. If you encounter an accessibility barrier, please [file an accessibility report](https://github.com/dallay/cortex/issues/new?template=accessibility.yml) using our dedicated template. This helps us prioritize and address barriers for assistive technology users.

See our [Accessibility Statement](ACCESSIBILITY.md) for more information.

### Submitting Pull Requests

1. **Fork** the repository
2. **Create a feature branch** (`git checkout -b feature/amazing-feature`)
3. **Make your changes** following the guidelines below
4. **Run tests and checks** (`just ci-local`)
5. **Commit** using conventional commits (`git commit -m 'feat: add amazing feature'`)
6. **Push** to your branch (`git push origin feature/amazing-feature`)
7. **Open a Pull Request** with a clear description

Our PR template includes checklists for accessibility (for UI changes) and quality — please complete them before requesting review (mark N/A if non-UI).

## Development Workflow

1. **Set up your environment**. Run `just setup` for initial setup and consult [docs/development.md](docs/development.md) for details.
2. **Follow Rust best practices** and project conventions defined in [.agents/AGENTS.md](.agents/AGENTS.md).
3. **Write tests** for new behavior. See the [Development Guide](docs/development.md) for testing strategies.
4. **Run focused checks** during iteration:
   - Rust: `cargo test -p <package>`
   - Dashboard: `pnpm --filter @dallay/rook-dashboard exec vitest run`
5. **Run full CI locally** before submitting: `just ci-local`
6. **Update documentation** as needed.

## Code Style

- **Rust**: Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) and project conventions. Run `cargo fmt` and `cargo clippy --all -- -D warnings`.
- **TypeScript/JavaScript**: Follow the existing patterns in the dashboard. Run `pnpm exec vitest run` and type checks.
- **Markdown**: Follow the project's markdownlint configuration (`.markdownlint.json`). Run `pnpm exec markdownlint-cli2` on documentation files.
- **Commits**: Use [Conventional Commits](https://www.conventionalcommits.org/) format.

## Pull Request Process

1. **Fill out the PR template**. Include a description, related issue, type of change, and complete all checklists.
2. **Accessibility checklist**. Verify keyboard navigation, focus states, screen reader behavior, and more.
3. **Quality checklist**. Ensure tests pass, documentation is updated, and linting passes.
4. **Request review**. At least one maintainer must approve before merging.
5. **Address feedback**. Respond to review comments and make requested changes.
6. **Merge**. Maintainers will squash-merge approved PRs.

## Accessibility

Cortex aims to be accessible to all users. When contributing UI changes:

- Ensure keyboard navigation works end-to-end
- Make focus states visible and logical
- Provide labels for form elements
- Don't rely on color alone to convey meaning
- Test with a screen reader when possible

See our [Accessibility Statement](ACCESSIBILITY.md) for our commitment and guidelines. If you encounter an accessibility barrier, please [file a report](https://github.com/dallay/cortex/issues/new?template=accessibility.yml).

## Issue Templates

Cortex uses structured issue templates to help contributors provide the right information:

| Template | Purpose |
|----------|---------|
| [Bug Report](.github/ISSUE_TEMPLATE/bug.yml) | Report unexpected behavior or errors |
| [Feature Request](.github/ISSUE_TEMPLATE/feature.yml) | Suggest new features or improvements |
| [Technical Task](.github/ISSUE_TEMPLATE/technical-task.yml) | Propose technical improvements or refactors |
| [Accessibility Issue](.github/ISSUE_TEMPLATE/accessibility.yml) | Report accessibility barriers |

### Issue Classification

All issues are classified using labels:

- `product/*` — Owning product: `rook`, `huginn`, or `shared`
- `area/*` — Technical areas: `auth`, `ci`, `core`, `dashboard`, etc.
- `type/*` — Work type: `bug`, `feature`, `enhancement`, `chore`, etc.

See [docs/contributing/labeling-policy.md](docs/contributing/labeling-policy.md) for the full taxonomy.

## Policies and Guidelines

| Topic                  | Location                                                                           |
|------------------------|------------------------------------------------------------------------------------|
| Issue labeling         | [docs/contributing/labeling-policy.md](docs/contributing/labeling-policy.md)       |
| GitHub → Linear sync   | [docs/contributing/linear-sync-policy.md](docs/contributing/linear-sync-policy.md) |
| Architecture decisions | [docs/adr/README.md](docs/adr/README.md)                                           |

## License

By contributing to Cortex, you agree that your contributions will be licensed under the MIT License. See [LICENSE](LICENSE).
