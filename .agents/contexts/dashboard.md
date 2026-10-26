---
applyTo: "apps/rook/dashboard/**"
---

# Rook dashboard — Interface instructions

These instructions extend `.agents/AGENTS.md` and `.agents/contexts/rook.md`.
Both contracts remain mandatory. Paths below are relative to
`apps/rook/dashboard` unless explicitly stated otherwise.

## UI conventions

- Use the existing Vue, TypeScript and Tailwind conventions. Reuse
  `src/components/ui` and tokens in `src/style.css`; do not duplicate components
  or hardcode colors.
- All visible text uses vue-i18n. Update both `src/locales/en.json` and
  `src/locales/es.json` in the same change.
- Every view handles loading, empty, error and denied states; reuse
  `LoadingState`, `EmptyState` and `ErrorBanner`.
- Preserve keyboard navigation, accessible labels, visible focus and useful
  recovery feedback. Verify interactions, not just successful rendering.
- Respect Rook's management cookie and CSRF contracts. UI visibility never
  replaces backend authorization.

## Tests and commands (from repository root)

- Follow Red → Green → Refactor for behavior changes. Colocate Vitest tests as
  `*.spec.ts`; test observable interactions and failure states.
- Run `pnpm --filter @dallay/rook-dashboard exec vitest run [file]`,
  `pnpm --filter @dallay/rook-dashboard run typecheck`, and
  `pnpm --filter @dallay/rook-dashboard exec biome ci .` as applicable.
- Verify affected critical flows with Playwright in `apps/rook/dashboard/e2e/`
  or `dev/e2e/`. Read `dev/README.md` for E2E prerequisites.
- Consult `apps/rook/dashboard/README.md` for setup. Use the root pnpm lockfile;
  never create a dashboard lockfile or workspace file.
