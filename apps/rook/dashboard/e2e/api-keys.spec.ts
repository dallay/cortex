import { test, expect, Page } from '@playwright/test'

// =============================================================================
// Test Configuration
// =============================================================================

const API_BASE_URL = process.env.API_BASE_URL || 'http://localhost:8080'
const DASHBOARD_URL = process.env.DASHBOARD_URL || 'http://localhost:5173'
const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD || 'Admin123!234'

function cookieValue(setCookie: string | null, name: string): string | null {
  if (!setCookie) return null
  const match = setCookie.match(new RegExp(`${name}=([^;]+)`))
  return match?.[1] ?? null
}

// =============================================================================
// Helper: Get CSRF token via API
// =============================================================================

async function getCsrfToken(page: Page): Promise<{ token: string; cookie: string }> {
  const response = await page.request.get(`${API_BASE_URL}/login`)
  const body = await response.json()
  const token = body.csrf_token as string
  return {
    token,
    cookie: cookieValue(response.headers()['set-cookie'] ?? null, 'csrf_token') ?? token,
  }
}

// =============================================================================
// Helper: Login via API and set cookies in browser
// =============================================================================

async function loginAsAdmin(page: Page, password: string = ADMIN_PASSWORD): Promise<void> {
  const csrf = await getCsrfToken(page)

  const loginResponse = await page.request.post(`${API_BASE_URL}/login`, {
    data: {
      username: 'admin',
      password: password
    },
    headers: {
      'Content-Type': 'application/json',
      'X-CSRF-Token': csrf.token,
      'Cookie': `csrf_token=${csrf.cookie}`
    }
  })

  if (!loginResponse.ok()) {
    throw new Error(`Login failed: ${loginResponse.status()} ${await loginResponse.text()}`)
  }

  const authCookie = cookieValue(loginResponse.headers()['set-cookie'] ?? null, 'auth_token')
  if (!authCookie) {
    throw new Error('Login response did not set auth_token cookie')
  }

  await page.context().addCookies([
    {
      name: 'csrf_token',
      value: csrf.cookie,
      url: DASHBOARD_URL,
      httpOnly: true,
      sameSite: 'Strict',
    },
    {
      name: 'auth_token',
      value: authCookie,
      url: DASHBOARD_URL,
      httpOnly: true,
      sameSite: 'Lax',
    },
  ])
}

// =============================================================================
// Helper: Create API key via API
// =============================================================================

async function createApiKeyViaApi(
  page: Page,
  label: string,
  scopes: string[] = ['read'],
  tier: string = 'free'
): Promise<{ id: string; plaintextKey: string }> {
  const csrf = await getCsrfToken(page)
  const cookies = await page.context().cookies(DASHBOARD_URL)
  const authCookie = cookies.find(c => c.name === 'auth_token')?.value || ''

  const response = await page.request.post(`${API_BASE_URL}/api/api-keys`, {
    data: { label, scopes, tier, expiresAt: null },
    headers: {
      'Content-Type': 'application/json',
      'X-CSRF-Token': csrf.token,
      'Cookie': `csrf_token=${csrf.cookie}; auth_token=${authCookie}`
    }
  })

  if (!response.ok()) {
    throw new Error(`Failed to create API key: ${response.status()} ${await response.text()}`)
  }

  const data = await response.json()
  return { id: data.key.id, plaintextKey: data.plaintextKey }
}

// =============================================================================
// Test: Dashboard loads without crashing
// =============================================================================

test.describe('Dashboard', () => {
  test('loads the home page', async ({ page }) => {
    // Set API base URL for the frontend
    await page.goto(DASHBOARD_URL)
    await page.waitForLoadState('networkidle')

    // Should see the home page title
    await expect(page.getByRole('heading', { name: /Dashboard/i })).toBeVisible({ timeout: 10000 })
  })
})

// =============================================================================
// Test: API Keys Page - Loading and Empty State
// =============================================================================

test.describe('API Keys - List View', () => {
  test.beforeEach(async ({ page }) => {
    // Login first
    await page.goto(DASHBOARD_URL)
    await page.waitForLoadState('networkidle')

    // Login via API
    await loginAsAdmin(page)

    // Navigate to API keys page
    await page.goto(`${DASHBOARD_URL}/api-keys`)
    await page.waitForLoadState('networkidle')
  })

  test('shows page title', async ({ page }) => {
    await expect(page.getByRole('heading', { name: /api keys/i })).toBeVisible()
  })

  test('shows empty state when no keys exist', async ({ page }) => {
    await expect(page.getByText(/no api keys yet/i)).toBeVisible()
  })

  test('shows Create API Key button', async ({ page }) => {
    await expect(page.getByRole('button', { name: /create api key/i })).toBeVisible()
  })
})

// =============================================================================
// Test: API Keys - Create Flow
// =============================================================================

test.describe('API Keys - Create Flow', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(DASHBOARD_URL)
    await page.waitForLoadState('networkidle')
    await loginAsAdmin(page)
    await page.goto(`${DASHBOARD_URL}/api-keys`)
    await page.waitForLoadState('networkidle')
  })

  test('opens create modal when clicking Create API Key button', async ({ page }) => {
    await page.getByRole('button', { name: /create api key/i }).click()
    await expect(page.getByRole('dialog')).toBeVisible()
    await expect(page.getByText(/create api key/i).first()).toBeVisible()
  })

  test('shows validation errors when submitting empty form', async ({ page }) => {
    await page.getByRole('button', { name: /create api key/i }).click()
    await page.getByRole('button', { name: /create key/i }).click()
    await expect(page.getByText(/label is required/i)).toBeVisible()
    await expect(page.getByText(/at least one scope is required/i)).toBeVisible()
  })

  test('creates API key with valid form data', async ({ page }) => {
    await page.getByRole('button', { name: /create api key/i }).click()

    // Fill in the form
    await page.getByLabel(/label/i).fill('test-agent-key')
    await page.getByText(/read/i).click()

    // Submit
    await page.getByRole('button', { name: /create key/i }).click()

    // Should show the warning about saving the key
    await expect(
      page.getByText(/save this key now — it will not be shown again/i)
    ).toBeVisible()

    // Should show the plaintext key
    const keyDisplay = page.locator('code')
    await expect(keyDisplay).toBeVisible()

    // Copy button should be visible
    await expect(page.getByRole('button', { name: /copy/i })).toBeVisible()

    // Click Done to close
    await page.getByRole('button', { name: /done/i }).click()

    // Modal should close
    await expect(page.getByRole('dialog')).not.toBeVisible()

    // The key should now appear in the list
    await expect(page.getByText(/test-agent-key/i)).toBeVisible()
  })
})

// =============================================================================
// Test: API Keys - Edit Flow
// =============================================================================

test.describe('API Keys - Edit Flow', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(DASHBOARD_URL)
    await page.waitForLoadState('networkidle')
    await loginAsAdmin(page)
    await page.goto(`${DASHBOARD_URL}/api-keys`)
    await page.waitForLoadState('networkidle')

    // Create a key to edit
    await createApiKeyViaApi(page, 'key-to-edit', ['read'], 'free')
  })

  test('opens edit modal when clicking edit button', async ({ page }) => {
    // Reload to see the created key
    await page.reload()
    await page.waitForLoadState('networkidle')

    // Find the key row
    const row = page.locator('tbody tr').filter({ hasText: 'key-to-edit' })

    // Click edit button (pencil icon)
    await row.locator('button').first().click()

    // Modal should be visible
    await expect(page.getByRole('dialog')).toBeVisible()
    await expect(page.getByText(/edit api key/i)).toBeVisible()
  })

  test('pre-fills form with existing key data', async ({ page }) => {
    await page.reload()
    await page.waitForLoadState('networkidle')

    const row = page.locator('tbody tr').filter({ hasText: 'key-to-edit' })
    await row.locator('button').first().click()

    // Label should be pre-filled
    await expect(page.getByLabel(/label/i)).toHaveValue('key-to-edit')
  })

  test('updates key successfully', async ({ page }) => {
    await page.reload()
    await page.waitForLoadState('networkidle')

    const row = page.locator('tbody tr').filter({ hasText: 'key-to-edit' })
    await row.locator('button').first().click()

    // Change the label
    await page.getByLabel(/label/i).fill('updated-key-label')

    // Save changes
    await page.getByRole('button', { name: /save changes/i }).click()

    // Modal should close
    await expect(page.getByRole('dialog')).not.toBeVisible()

    // Key should show updated label
    await expect(page.getByText(/updated-key-label/i)).toBeVisible()
  })
})

// =============================================================================
// Test: API Keys - Revoke Flow
// =============================================================================

test.describe('API Keys - Revoke Flow', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto(DASHBOARD_URL)
    await page.waitForLoadState('networkidle')
    await loginAsAdmin(page)
    await page.goto(`${DASHBOARD_URL}/api-keys`)
    await page.waitForLoadState('networkidle')

    // Create a key to revoke
    await createApiKeyViaApi(page, 'key-to-revoke', ['read'], 'free')
  })

  test('shows confirmation dialog when revoking', async ({ page }) => {
    await page.reload()
    await page.waitForLoadState('networkidle')

    const row = page.locator('tbody tr').filter({ hasText: 'key-to-revoke' })

    // Click revoke button (trash icon)
    await row.locator('button').nth(1).click()

    // Confirmation dialog should appear
    await expect(page.getByRole('dialog')).toBeVisible()
    await expect(page.getByText(/revoke api key/i)).toBeVisible()
  })

  test('revokes key successfully and shows Revoked status', async ({ page }) => {
    await page.reload()
    await page.waitForLoadState('networkidle')

    const row = page.locator('tbody tr').filter({ hasText: 'key-to-revoke' })

    // Click revoke button
    await row.locator('button').nth(1).click()

    // Confirm revocation
    await page.getByRole('button', { name: /revoke key/i }).click()

    // Wait for dialog to close and status to update
    await page.waitForTimeout(500)

    // Key should now show Revoked status
    await page.reload()
    await page.waitForLoadState('networkidle')

    const revokedRow = page.locator('tbody tr').filter({ hasText: 'key-to-revoke' })
    await expect(revokedRow.locator('text=Revoked')).toBeVisible()
  })
})

// =============================================================================
// Test: API Keys - Pagination (if more than 20 keys exist)
// =============================================================================

test.describe('API Keys - Pagination', () => {
  test('pagination controls exist', async ({ page }) => {
    await page.goto(DASHBOARD_URL)
    await page.waitForLoadState('networkidle')
    await loginAsAdmin(page)
    await page.goto(`${DASHBOARD_URL}/api-keys`)
    await page.waitForLoadState('networkidle')

    // If there are keys, pagination should be visible
    const hasKeys = await page.locator('tbody tr').count() > 0
    if (hasKeys) {
      await expect(page.getByRole('button', { name: /previous/i })).toBeVisible()
      await expect(page.getByRole('button', { name: /next/i })).toBeVisible()
    }
  })
})
