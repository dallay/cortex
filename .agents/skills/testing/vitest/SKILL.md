---
name: vitest
description: Vitest unit testing for the Rook dashboard and other Vite packages. Use when writing tests, mocking, configuring coverage, filtering tests, or upgrading across Vitest majors.
metadata:
  author: cortex
  version: "2.0"
---

# Vitest

Vitest runs on the same Vite pipeline as the application, so tests share plugins,
aliases and transforms. This skill targets **Vitest 5.0.5**, the version pinned in
`apps/rook/dashboard/package.json`. Check that manifest before assuming an older API.

## In this repository

Dashboard tests are colocated `*.spec.ts` files under `apps/rook/dashboard/src`
(44 files at the time of writing). They run in `jsdom`, never in a browser.

```text
apps/rook/dashboard/
├── vitest.config.ts   # merged with vite.config.ts; jsdom, excludes e2e/**
├── vite.config.ts     # plugins and the `@` → ./src alias
└── src/**/*.spec.ts   # colocated unit/component tests
```

`test.environment` is `jsdom` and coverage uses `v8` with enforced thresholds (statements 83, branches 72, functions 80, lines 84). `e2e/**` is excluded because
Playwright owns it.

Commands, from the repository root:

```bash
pnpm --filter @dallay/rook-dashboard exec vitest run                 # full suite
pnpm --filter @dallay/rook-dashboard exec vitest run src/lib/api.spec.ts
pnpm --filter @dallay/rook-dashboard exec vitest run -t "login"      # name filter
pnpm --filter @dallay/rook-dashboard exec vitest --coverage          # watch/coverage
pnpm --filter @dallay/rook-dashboard run typecheck                   # vue-tsc --noEmit
pnpm --filter @dallay/rook-dashboard run lint                        # biome check .
```

`just ci-test-vitest` runs typecheck, `biome ci` and the full suite. Prefer the
focused command while iterating and the recipe before opening a PR.

## Vitest 5 behavior that changes tests

| Change                                                        | Consequence                                                                                                                                                               |
|---------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `clearMocks` defaults to `true`                               | Call history is cleared before every test. Mocks configured once in `beforeAll` or at module scope lose their recorded calls; set `clearMocks: false` only with a reason. |
| Unawaited async assertions fail                               | `expect(promise).resolves...` must be awaited. The test fails instead of warning.                                                                                         |
| Hoisted calls must be top level                               | `vi.mock`, `vi.unmock` and `vi.hoisted` inside a function, block or `describe` throw. Put them at module top level; use `vi.doMock` for conditional mocking.              |
| `test.sequential` and `describe.sequential` removed           | Opt out with `test("name", { concurrent: false }, fn)` or `describe("name", { concurrent: false }, fn)`.                                                                  |
| `-t` matches the full `' > '` name                            | `-t "math adds"` no longer matches `describe("math")` + `test("adds")`; use `-t adds` or `-t "math.*adds"`.                                                               |
| No parent-directory config lookup                             | Run Vitest from the package root or pass `--config`.                                                                                                                      |
| `toThrow("")` matches any message                             | Assert an empty message with `/^$/`.                                                                                                                                      |
| Coverage `include`/`exclude` are root-relative, no `contains` | Re-check patterns after upgrading; a bare directory pattern matches its contents.                                                                                         |
| `@vitest/expect` no longer shares state                       | Import assertions and `expect.extend` from `vitest`.                                                                                                                      |

## Core

| Topic         | Description                                                     | Reference                                    |
|---------------|-----------------------------------------------------------------|----------------------------------------------|
| Configuration | Vitest and Vite config integration, defineConfig usage          | [core-config](references/core-config.md)     |
| CLI           | Command line interface, commands and options                    | [core-cli](references/core-cli.md)           |
| Test API      | test/it function, modifiers like skip, only, concurrent         | [core-test-api](references/core-test-api.md) |
| Describe API  | describe/suite for grouping tests and nested suites             | [core-describe](references/core-describe.md) |
| Expect API    | Assertions with toBe, toEqual, matchers and asymmetric matchers | [core-expect](references/core-expect.md)     |
| Hooks         | beforeEach, afterEach, beforeAll, afterAll, aroundEach          | [core-hooks](references/core-hooks.md)       |

## Features

| Topic        | Description                                                    | Reference                                                  |
|--------------|----------------------------------------------------------------|------------------------------------------------------------|
| Mocking      | Mock functions, modules, timers, dates with vi utilities       | [features-mocking](references/features-mocking.md)         |
| Snapshots    | Snapshot testing with toMatchSnapshot and inline snapshots     | [features-snapshots](references/features-snapshots.md)     |
| Coverage     | Code coverage with V8 or Istanbul providers                    | [features-coverage](references/features-coverage.md)       |
| Test Context | Test fixtures, context.expect, test.extend for custom fixtures | [features-context](references/features-context.md)         |
| Concurrency  | Concurrent tests, parallel execution, sharding                 | [features-concurrency](references/features-concurrency.md) |
| Filtering    | Filter tests by name, file patterns, tags                      | [features-filtering](references/features-filtering.md)     |

## Advanced

| Topic        | Description                                             | Reference                                                    |
|--------------|---------------------------------------------------------|--------------------------------------------------------------|
| Vi Utilities | vi helper: mock, spyOn, fake timers, hoisted, waitFor   | [advanced-vi](references/advanced-vi.md)                     |
| Environments | Test environments: node, jsdom, happy-dom, custom       | [advanced-environments](references/advanced-environments.md) |
| Type Testing | Type-level testing with expectTypeOf and assertType     | [advanced-type-testing](references/advanced-type-testing.md) |
| Projects     | Multi-project workspaces, different configs per project | [advanced-projects](references/advanced-projects.md)         |

## Patterns

### Mock a module at top level

```ts
import { describe, expect, it, vi } from "vitest";
import { useProviders } from "@/composables/useProviders";

vi.mock("@/composables/useProviders", () => ({
	useProviders: vi.fn(),
}));
```

### Reset spies when a test replaces globals

`clearMocks` clears history but keeps implementations. A `vi.spyOn(...).mockReturnValue`
still needs explicit restoration:

```ts
afterEach(() => {
	vi.restoreAllMocks();
});
```

### Component tests

Use `@vue/test-utils` (`2.5.1`) with `jsdom`; no browser-mode runner is installed.

```ts
import { mount } from "@vue/test-utils";
import { expect, it } from "vitest";
import StatusBadge from "@/components/StatusBadge.vue";

it("renders the current status", () => {
	const wrapper = mount(StatusBadge, { props: { status: "active" } });
	expect(wrapper.text()).toBe("active");
});
```

- Assert user-visible output and emitted events, not internal refs.
- Prefer `findByRole`/text queries over CSS selectors where the wrapper allows it.
- Cover loading, empty, error and success states for data-driven components.

### Deterministic time

```ts
beforeEach(() => {
	vi.useFakeTimers();
	vi.setSystemTime(new Date("2026-01-01T00:00:00Z"));
});

afterEach(() => {
	vi.useRealTimers();
});
```

## Anti-patterns

- `test.sequential`, `describe.sequential`, or `{ sequential: true }` — removed in v5.
- Unawaited `.resolves` / `.rejects` / `toMatchFileSnapshot`.
- `vi.mock` inside `describe`, `test` or a helper function.
- Asserting on private state instead of props, emits and rendered output.
- Snapshotting large DOM trees; assert the behavior instead.
- Sleeping with real timers when `vi.useFakeTimers()` exists.
- Lowering coverage thresholds to make a change pass.

## Resources

- [Vitest 5 migration](https://vitest.dev/guide/migration/)
- [Configuration](https://vitest.dev/config/)
- [Mocking modules](https://vitest.dev/guide/mocking/modules)
- [Test context](https://vitest.dev/guide/test-context)
