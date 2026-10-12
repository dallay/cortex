# Contributing to Cortex

We love your input! We want to make contributing to Cortex as easy and transparent as possible, whether it's:

- Reporting a bug
- Discussing the current state of the code
- Submitting a fix
- Proposing new features
- Becoming a maintainer

## Quick Contribution Guide

Before contributing, run the one-shot setup and verify your environment:

```bash
git clone https://github.com/dallay/cortex.git
cd cortex
just setup      # Installs all dev dependencies (see README or docs/development.md)
just doctor     # Verifies toolchain
just ci-local   # Runs full CI locally
```

**Standard workflow:**

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Make your changes and add tests
4. Run `just ci-local` to verify all checks pass
5. Commit using [Conventional Commits](https://www.conventionalcommits.org/) (`feat:`, `fix:`, `docs:`, etc.)
6. Push to your fork (`git push origin feature/amazing-feature`)
7. Open a Pull Request

See the [Development Guide](docs/development.md) for detailed workflows, testing strategies, and troubleshooting.

## Development Workflow

See the [Development Guide](docs/development.md) for comprehensive setup and workflows.

**Quick reference:**
- `just setup` — one-shot bootstrap (first time only)
- `just run` — start backend on :3773
- `just run-dashboard` — start Vue dashboard on :4747
- `just dev` — watch mode (auto-check + test + clippy on save)
- `just test` — full test suite
- `just ci-local` — complete CI locally (required before PR)

**Quality gates:**
- Follow Rust best practices and project conventions
- Ensure all tests pass: `just test`
- Run `just ci-local` before submitting a PR (same checks as CI)
- Update documentation as needed
- Use [Conventional Commits](https://www.conventionalcommits.org/) format

## Code of Conduct

This project follows the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct).

## Reporting Bugs

We use GitHub issues to track public bugs. Report a bug by [opening a new issue](https://github.com/dallay/cortex/issues/new).

## License

By contributing to Cortex, you agree that your contributions will be licensed under the Cortex MIT License.
