---
name: playwright
description: >
  Plan, write, run, and fix Playwright E2E tests for the Rook dashboard.
  Trigger: when working with apps/rook/dashboard/e2e, playwright.config.ts,
  browser tests, or E2E failures.
allowed-tools: Read, Edit, Write, Glob, Grep, Bash
metadata:
  author: cortex
  version: "2.0"
---

# Playwright E2E — Rook dashboard

End-to-end coverage lives next to the dashboard, not in a shared testing package.
There is no `packages/testing-e2e`, `apps/portfolio` or `apps/blog` in Cortex.

## Where things are

```text
apps/rook/dashboard/
├── playwright.config.ts      # testDir: e2e, baseURL, projects, webServer
└── e2e/
    ├── global-setup.ts       # logs in once and saves storageState
    ├── login.spec.ts
    ├── api-keys.spec.ts
    ├── providers.spec.ts
    ├── vue.spec.ts
    ├── tsconfig.json
    └── .auth/admin.json      # generated; never commit credentials
dev/e2e/run-api-keys-e2e.sh   # Docker-backed full suite
```

Playwright version: `1.64.1` (`apps/rook/dashboard/package.json`). Do not assume
a different major without checking the manifest.

## Commands

Run from `apps/rook/dashboard` unless noted:

```bash
pnpm exec playwright test                    # all specs, local dev server
pnpm exec playwright test e2e/login.spec.ts  # one file
pnpm exec playwright test -g "login"         # one title
pnpm exec playwright test --headed           # visible browser (local)
pnpm exec playwright test --debug            # inspector
pnpm exec playwright test --ui               # UI mode
pnpm exec playwright test --project=chromium
pnpm exec playwright show-report
```

Docker-backed suite (backend + dashboard, from the repository root):

```bash
just test-e2e-build   # build the image once
just test-e2e         # run the suite
just test-e2e-cleanup # remove containers and volumes
```

`just test-e2e` runs `dev/e2e/run-api-keys-e2e.sh --test`, which starts the
backend and the Vite dev server and then calls `pnpm playwright test e2e/`.
Python, Docker and the image are prerequisites; see `dev/README.md`.

## Configuration (actual values)

| Setting | Value |
|---|---|
| `testDir` | `./e2e` |
| Base URL (CI) | `http://localhost:4173/dashboard/` |
| Base URL (local) | `http://localhost:4747/dashboard/` |
| Test timeout | 30 s |
| Expect timeout | 5 s |
| Action timeout | 0 (no per-action limit) |
| Workers | `PLAYWRIGHT_WORKERS`, default `1` |
| Retries | not configured (0) |
| Reporter | `html` |
| Trace | `on-first-retry` |
| Projects | chromium, firefox, webkit |
| Auth state | `e2e/.auth/admin.json` |
| Web server (CI) | `pnpm run build && pnpm run preview`, port 4173 |
| Web server (local) | `pnpm run dev`, port 4747 |

Workers default to 1 because the backend rate-limits login, CSRF and API calls.
Raise `PLAYWRIGHT_WORKERS` only with evidence that the limits tolerate it.

## Authentication

`global-setup.ts` obtains a CSRF token, logs in as `admin` against
`API_BASE_URL` (default `http://127.0.0.1:8081`), and saves the authenticated
storage state. Tests then start logged in.

- To test anonymous flows, override the state inside the file:
  `test.use({ storageState: { cookies: [], origins: [] } });`
- Set `SKIP_BACKEND=1` to skip specs that need the real backend, as
  `login.spec.ts` does.
- `ADMIN_PASSWORD` defaults to the dev password. Never commit real credentials.
- CSRF is a double-submit cookie: state-changing requests need `X-CSRF-Token`
  matching the `csrf_token` cookie. Reuse the helpers in the specs; do not
  invent a second login path.

## Writing tests

```ts
import { expect, test } from "@playwright/test";

test.describe("API keys", () => {
	test("creates a key and shows it once", async ({ page }) => {
		await page.goto("api-keys");
		await page.getByRole("button", { name: /create/i }).click();
		await page.getByLabel(/label/i).fill("e2e-key");
		await page.getByRole("button", { name: /save|create/i }).click();

		await expect(page.getByText("e2e-key")).toBeVisible();
		await expect(page.getByRole("dialog")).toBeHidden();
	});
});
```

- One behavior per test; tests must pass in any order and in parallel-safe form.
- Use role, label, placeholder and text locators before CSS or test ids.
- Use web-first assertions (`toBeVisible`, `toHaveText`, `toHaveURL`) instead of
  reading state manually.
- Assert user-visible outcomes, not implementation details; control requests
  through the UI.
- When a request is unavoidable in a test, wait for the response explicitly:
  `await page.waitForResponse("**/api/providers")`.
- Mock only external or failure paths. Prefer the real backend for CRUD flows
  when the Docker suite provides it. Example:

```ts
await page.route("**/api/providers", (route) =>
	route.fulfill({ status: 500, body: "boom" }),
);
```

## Handling failures

- Reproduce first: run the single spec, then `--debug` or `--trace on`.
- Read the trace and the HTML report before changing selectors.
- Fix the smallest cause: stale locator, missing wait, changed copy, real bug.
- **Never hide a regression.** If the application is broken, keep the failing
  test visible. Use `test.fail()` only for a known, documented defect that must
  remain executed; `test.fixme()` skips the test entirely.
- `test.fixme()` requires an explicit reason and a linked issue, and needs
  reviewer approval. It is not the default response to a red suite.
- Do not add `waitForTimeout`, retries or `force: true` to make a test pass.
- Do not ask the user mid-run; make the smallest defensible fix and report it.

## Anti-patterns

- `page.waitForLoadState("networkidle")` or arbitrary sleeps.
- `page.waitForTimeout()`.
- `force: true` to bypass actionability checks.
- CSS selectors when a role or label is available.
- Shared mutable state between specs; each test creates what it needs.
- Snapshots of raw HTML or screenshots as the primary assertion.
- Asserting on backend logs instead of the UI.
- Copying commands from another repository (`pnpm --filter=portfolio`,
  `packages/testing-e2e`, `npm run ...`).

## Accessibility

`@axe-core/playwright` is not installed. Do not import it unless the dependency
is added deliberately in the same change. Until then:

- Assert accessible names and roles, which fails when labels disappear.
- Cover keyboard paths for dialogs, menus and forms in the affected spec.
- Treat a missing label or unreachable control as a product bug, not a test bug.

## Resources

- [Playwright Test](https://playwright.dev/docs/intro)
- [Best practices](https://playwright.dev/docs/best-practices)
- [Locators](https://playwright.dev/docs/locators)
- [Test annotations](https://playwright.dev/docs/test-annotations)
- [Trace viewer](https://playwright.dev/docs/trace-viewer)
