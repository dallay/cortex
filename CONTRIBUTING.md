# Contributing to Cortex

We love your input! We want to make contributing to Cortex as easy and transparent as possible.

## Quick Contribution Guide

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'feat: add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## Development Workflow

1. Follow [Rust best practices](https://rust-lang.github.io/api-guidelines/) and project conventions
2. Ensure all tests pass: `cargo test`
3. Run the dashboard checks: `pnpm --filter @dallay/rook-dashboard exec vitest run`
4. Run `just ci-local` before submitting a PR
5. Update documentation as needed

For detailed development setup, see [docs/development.md](docs/development.md).

## Policies and Guidelines

| Topic | Location |
|-------|----------|
| Issue labeling | [docs/contributing/labeling-policy.md](docs/contributing/labeling-policy.md) |
| GitHub → Linear sync | [docs/contributing/linear-sync-policy.md](docs/contributing/linear-sync-policy.md) |
| Architecture decisions | [docs/adr/README.md](docs/adr/README.md) |

### Issue Classification

All issues should be classified using the labeling policy. The system uses:
- `product/*` — Owning product: `rook`, `huginn`, or `shared`
- `area/*` — Technical areas: `auth`, `ci`, `core`, `dashboard`, etc.
- `type/*` — Work type: `bug`, `feature`, `enhancement`, `chore`, etc.

See [docs/contributing/labeling-policy.md](docs/contributing/labeling-policy.md) for full taxonomy.

## Code of Conduct

This project follows the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct).

## Reporting Bugs

We use GitHub issues to track bugs. Report a bug by [opening a new issue](https://github.com/dallay/cortex/issues/new).

## License

By contributing to Cortex, you agree that your contributions will be licensed under the Cortex MIT License.
