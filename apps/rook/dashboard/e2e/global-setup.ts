import { chromium, request } from '@playwright/test'
import { mkdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const __dirname = path.dirname(fileURLToPath(import.meta.url))

const API_BASE_URL = process.env.API_BASE_URL || 'http://localhost:8080'
const DASHBOARD_URL = process.env.DASHBOARD_URL || 'http://localhost:5173'
const ADMIN_PASSWORD = process.env.ADMIN_PASSWORD || 'Admin123!234'

export const AUTH_STATE_PATH = path.join(__dirname, '.auth', 'admin.json')

async function getCsrfToken(
  api: Awaited<ReturnType<typeof request.newContext>>,
): Promise<{ token: string; cookie: string }> {
  const res = await api.get(`${API_BASE_URL}/login`)
  const body = await res.json()
  const token = body.csrf_token as string
  const cookie = res.headers()['set-cookie']?.match(/csrf_token=([^;]+)/)?.[1] ?? token
  return { token, cookie }
}

async function saveAuthState(): Promise<void> {
  // Launch a real browser so we can capture browser-level cookies (storageState).
  const browser = await chromium.launch()
  const context = await browser.newContext()
  const page = await context.newPage()

  // Obtain CSRF token via the page's request context (shares cookies with the page).
  const csrfRes = await page.request.get(`${API_BASE_URL}/login`)
  const csrfBody = await csrfRes.json()
  const csrfToken = csrfBody.csrf_token as string
  const csrfCookie = csrfRes.headers()['set-cookie']?.match(/csrf_token=([^;]+)/)?.[1] ?? csrfToken

  const loginRes = await page.request.post(`${API_BASE_URL}/login`, {
    data: { username: 'admin', password: ADMIN_PASSWORD },
    headers: {
      'Content-Type': 'application/json',
      'X-CSRF-Token': csrfToken,
      'Cookie': `csrf_token=${csrfCookie}`,
    },
  })

  if (!loginRes.ok()) {
    await browser.close()
    throw new Error(`[globalSetup] saveAuthState login failed: ${loginRes.status()} ${await loginRes.text()}`)
  }

  // The auth_token is issued by the backend (port 8080).  The frontend (Vite, port 5173)
  // proxies /api/* to the backend, so the cookie must be registered for the FRONTEND
  // origin — otherwise the browser won't send it with proxied API requests.
  const authToken = loginRes.headers()['set-cookie']?.match(/auth_token=([^;]+)/)?.[1]
  if (!authToken) {
    await browser.close()
    throw new Error('[globalSetup] Login response did not include auth_token cookie')
  }

  await context.addCookies([
    {
      name: 'auth_token',
      value: authToken,
      url: DASHBOARD_URL,
      httpOnly: true,
      sameSite: 'Lax',
      secure: false,
    },
    // csrf_token is intentionally NOT stored in storageState.
    // The backend sets it as HttpOnly (XSS protection) and delivers it via
    // GET /login response body. The frontend fetches a fresh token before
    // each state-changing request — no need to persist it across sessions.
  ])

  // Navigate to the app so Vue router and auth store initialise with the cookie.
  await page.goto(`${DASHBOARD_URL}/`)
  await page.waitForLoadState('networkidle')

  // Persist cookies + localStorage for reuse by tests.
  await mkdir(path.dirname(AUTH_STATE_PATH), { recursive: true })
  await context.storageState({ path: AUTH_STATE_PATH })
  await browser.close()

  console.log(`[globalSetup] ✓ Auth state saved to ${AUTH_STATE_PATH}`)
}

async function globalSetup(): Promise<void> {
  const api = await request.newContext()

  try {
    const statusRes = await api.get(`${API_BASE_URL}/api/bootstrap/status`)
    if (!statusRes.ok()) {
      throw new Error(`Backend unreachable: ${statusRes.status()} — is rook running on ${API_BASE_URL}?`)
    }

    const status = await statusRes.json()

    if (!status.is_initialized) {
      // System is fresh — bootstrap with the test password
      const csrf = await getCsrfToken(api)

      // The bootstrap API no longer returns setup_token in the status response.
      // Obtain it from the environment variable printed at server startup.
      const setupToken = process.env.SETUP_TOKEN
      if (!setupToken) {
        throw new Error(
          'SETUP_TOKEN env var is required to bootstrap an uninitialized system.\n' +
            'Start rook once to print the token to logs, or set it via the ROOK_SETUP_TOKEN env var.',
        )
      }

      const setupRes = await api.post(`${API_BASE_URL}/api/bootstrap/setup`, {
        data: {
          setup_token: setupToken,
          password: ADMIN_PASSWORD,
        },
        headers: {
          'Content-Type': 'application/json',
          'X-CSRF-Token': csrf.token,
          'Cookie': `csrf_token=${csrf.cookie}`,
        },
      })

      if (!setupRes.ok()) {
        throw new Error(`Bootstrap setup failed: ${setupRes.status()} ${await setupRes.text()}`)
      }

      console.log(`[globalSetup] ✓ Backend bootstrapped with test password`)
    } else {
      // Already initialized — verify we can login with the expected password
      const csrf = await getCsrfToken(api)

      const loginRes = await api.post(`${API_BASE_URL}/login`, {
        data: { username: 'admin', password: ADMIN_PASSWORD },
        headers: {
          'Content-Type': 'application/json',
          'X-CSRF-Token': csrf.token,
          'Cookie': `csrf_token=${csrf.cookie}`,
        },
      })

      if (!loginRes.ok()) {
        throw new Error(
          `[globalSetup] Cannot login with ADMIN_PASSWORD="${ADMIN_PASSWORD}".\n` +
            `The backend DB was initialized with a different password.\n` +
            `Fix: delete ~/.local/share/cortex/rook/rook.db, restart rook, then re-run tests.\n` +
            `Or set ADMIN_PASSWORD env var to match your existing admin password.`,
        )
      }

      console.log(`[globalSetup] ✓ Backend already initialized, login verified`)
    }
  } finally {
    await api.dispose()
  }

  // Save the authenticated browser session for test reuse (avoids per-test logins).
  await saveAuthState()
}

export default globalSetup
