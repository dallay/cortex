import { test, expect } from '@playwright/test'

test('redirects unauthenticated users from / to /login', async ({ page }) => {
  await page.goto('/')
  await page.waitForURL(/\/login/, { timeout: 10_000 })
  await expect(page).toHaveURL(/\/login/)
})
