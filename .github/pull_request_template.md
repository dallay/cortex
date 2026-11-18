## Description

Brief description of the changes in this pull request.

## Related Issue

Fixes # (issue number)

## Type of Change

- [ ] 🐛 Bug fix (non-breaking change which fixes an issue)
- [ ] ✨ New feature (non-breaking change which adds functionality)
- [ ] 💥 Breaking change (fix or feature that would cause existing functionality to not work as expected)
- [ ] 📝 Documentation change
- [ ] 🔧 Refactor (no functional change)
- [ ] 🏗️ Chore (maintenance, dependencies, etc.)

## Accessibility Checklist

_For user interface changes only (check N/A if not applicable):_

- [ ] Keyboard navigation works end-to-end
- [ ] Focus states are visible and logical
- [ ] Forms have labels and errors are announced to assistive technology
- [ ] Color is not the only method to convey meaning
- [ ] Screen reader behavior was checked (VoiceOver, NVDA, or TalkBack)
- [ ] No accessibility linting errors (`pnpm --filter @dallay/rook-dashboard exec biome ci .` passes)
- [ ] N/A — This PR does not touch or affect user interfaces

## Quality Checklist

- [ ] Tests pass (`cargo test` and/or `pnpm --filter @dallay/rook-dashboard exec vitest run`)
- [ ] Documentation is updated if this changes user-facing behavior
- [ ] Linting passes (`cargo clippy --all -- -D warnings` and/or `pnpm --filter @dallay/rook-dashboard exec biome ci .`)
- [ ] `just ci-local` passes locally
- [ ] Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/)

## Screenshots (if applicable)

For UI changes, include screenshots or recordings to show the before and after.