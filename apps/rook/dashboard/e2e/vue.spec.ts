import { test, expect } from '@playwright/test'

const skipBackend = !!process.env.SKIP_BACKEND

;(skipBackend ? test.skip : test)('redirects unauthenticated users from / to /login', async ({ page }) => {
  await page.goto('/')
  await page.waitForURL(/\/login/, { timeout: 10_000 })
  await expect(page).toHaveURL(/\/login/)
})
