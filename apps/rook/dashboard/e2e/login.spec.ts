import { test, expect } from '@playwright/test'

/**
 * Login page e2e tests.
 *
 * These tests cover the Vite proxy bypass fix: browser navigation to /login
 * must be served the Vue SPA (index.html), not the raw CSRF JSON from the
 * backend. Vue Router then renders LoginView.
 *
 * Backend dependency: tests that stub the bootstrap API work offline.
 * Tests that require a live backend are skipped when SKIP_BACKEND is set.
 */

const skipBackend = !!process.env.SKIP_BACKEND

test.describe('Login page — proxy bypass', () => {
  test('navigating to /login serves the Vue SPA (not raw JSON)', async ({ page }) => {
    await page.goto('/login')

    // If the proxy bypass were missing, the page body would be raw JSON like
    // {"csrf_token":"..."} — check that we got HTML with a <div id="app">
    const appDiv = page.locator('#app')
    await expect(appDiv).toBeAttached({ timeout: 10_000 })
  })

  test('/login page renders without a JSON body', async ({ page }) => {
    await page.goto('/login')

    // Raw JSON would be visible as plain text in <body>; make sure it is not
    const bodyText = await page.locator('body').innerText()
    expect(bodyText).not.toMatch(/^\s*\{.*csrf_token.*\}\s*$/)
  })
})

test.describe('Login page — rendering', () => {
  test.use({ storageState: { cookies: [], origins: [] } })
  test('shows a login form on /login', async ({ page }) => {
    await page.goto('/login')
    await page.waitForLoadState('networkidle')

    // Either the login form or the setup form must be visible
    const hasLoginForm = await page.locator('input[type="password"]').isVisible().catch(() => false)
    const hasUsernameInput = await page.locator('input[type="text"], input[id*="user"], input[id*="email"]').isVisible().catch(() => false)

    expect(hasLoginForm || hasUsernameInput).toBe(true)
  })

  test('does not redirect /login to another route', async ({ page }) => {
    await page.goto('/login')
    await page.waitForLoadState('networkidle')

    // Must stay on /login (not bounce to / or /dashboard)
    await expect(page).toHaveURL(/\/login/)
  })
})

;(skipBackend ? test.describe.skip : test.describe)('Login page — route guard', () => {
  test.use({ storageState: { cookies: [], origins: [] } })
  test('unauthenticated visit to / redirects to /login', async ({ page }) => {
    // Clear cookies to ensure unauthenticated state
    await page.context().clearCookies()
    await page.goto('/')
    await page.waitForURL(/\/login/, { timeout: 10_000 })
    await expect(page).toHaveURL(/\/login/)
  })

  test('unauthenticated visit to /api-keys redirects to /login', async ({ page }) => {
    await page.context().clearCookies()
    await page.goto('/api-keys')
    await page.waitForURL(/\/login/, { timeout: 15_000 })
    await expect(page).toHaveURL(/\/login/)
  })
})
