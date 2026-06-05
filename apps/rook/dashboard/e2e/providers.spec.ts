import { test, expect } from '@playwright/test'

test.describe('Provider Management', () => {
  test.beforeEach(async ({ page }) => {
    // Navigate to the app and login
    await page.goto('/')
    
    // Check if we need to login
    const loginForm = page.locator('form')
    if (await loginForm.isVisible()) {
      await page.getByRole('textbox', { name: 'Password' }).fill('S3cr3tP@ssw0rd*123')
      await page.getByRole('button', { name: 'Sign in' }).click()
      await expect(page).toHaveURL('/')
    }
    
    // Navigate to providers page
    await page.goto('/providers')
  })

  test('should display providers page', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Providers' })).toBeVisible()
    await expect(page.getByText('Configure your AI providers')).toBeVisible()
  })

  test('should open add provider dialog', async ({ page }) => {
    await page.getByRole('button', { name: 'Add Provider' }).click()
    
    await expect(page.getByRole('dialog', { name: 'Add Provider' })).toBeVisible()
    await expect(page.getByText('Connect a new AI provider to Rook')).toBeVisible()
  })

  test('should validate required fields', async ({ page }) => {
    await page.getByRole('button', { name: 'Add Provider' }).click()
    
    // Save button should be disabled with empty form
    const saveButton = page.getByRole('button', { name: 'Save' }).last()
    await expect(saveButton).toBeDisabled()
    
    // Fill only name
    await page.getByRole('textbox', { name: 'Name' }).fill('Test Provider')
    await expect(saveButton).toBeDisabled()
    
    // Fill API key - now save should be enabled
    await page.getByRole('textbox', { name: 'API Key' }).fill('test-api-key-123')
    await expect(saveButton).toBeEnabled()
  })

  test('should add a new ollama provider', async ({ page }) => {
    // Click add provider button
    await page.getByRole('button', { name: 'Add Provider' }).click()
    
    // Fill form
    await page.getByRole('textbox', { name: 'Name' }).fill('Ollama Test E2E')
    await page.getByRole('textbox', { name: 'API Key' }).fill('ollama-test-key-e2e')
    await page.getByRole('textbox', { name: 'Base URL' }).fill('https://api.ollama.com')
    
    // Save
    await page.getByRole('button', { name: 'Save' }).last().click()
    
    // Dialog should close
    await expect(page.getByRole('dialog', { name: 'Add Provider' })).not.toBeVisible()
    
    // Provider should appear in the list
    await expect(page.getByText('Ollama Test E2E')).toBeVisible()
    await expect(page.getByText('ollama')).toBeVisible()
  })

  test('should show advanced configuration', async ({ page }) => {
    await page.getByRole('button', { name: 'Add Provider' }).click()
    
    // Advanced config should be collapsed initially
    await expect(page.getByLabel('Max Concurrent Requests')).not.toBeVisible()
    
    // Click to expand
    await page.getByRole('button', { name: 'Advanced Configuration' }).click()
    
    // Now advanced fields should be visible
    await expect(page.getByLabel('Max Concurrent Requests')).toBeVisible()
    await expect(page.getByLabel('Default Model')).toBeVisible()
  })

  test('should reset form when dialog is closed', async ({ page }) => {
    // Open dialog and fill some fields
    await page.getByRole('button', { name: 'Add Provider' }).click()
    await page.getByRole('textbox', { name: 'Name' }).fill('Test')
    await page.getByRole('textbox', { name: 'API Key' }).fill('key')
    
    // Close dialog
    await page.getByRole('button', { name: 'Close' }).click()
    
    // Reopen dialog
    await page.getByRole('button', { name: 'Add Provider' }).click()
    
    // Fields should be empty
    await expect(page.getByRole('textbox', { name: 'Name' })).toHaveValue('')
    await expect(page.getByRole('textbox', { name: 'API Key' })).toHaveValue('')
  })
})
