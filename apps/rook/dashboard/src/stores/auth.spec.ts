import { setActivePinia, createPinia } from 'pinia'
import { beforeEach, describe, expect, it, vi } from 'vitest'

const apiMock = vi.hoisted(() => ({
  getBootstrapStatus: vi.fn(),
  setupBootstrap: vi.fn(),
  login: vi.fn(),
  logout: vi.fn(),
}))

vi.mock('@/lib/api', () => ({
  useApi: () => apiMock,
}))

describe('auth store', () => {
  beforeEach(() => {
    setActivePinia(createPinia())
    vi.clearAllMocks()
  })

  it('marks bootstrap as required when the backend is not initialized', async () => {
    apiMock.getBootstrapStatus.mockResolvedValueOnce({
      isInitialized: false,
      adminUserExists: true,
      setupToken: 'setup-token',
    })

    const { useAuthStore } = await import('./auth')
    const store = useAuthStore()

    await store.loadBootstrapStatus()

    expect(store.bootstrapRequired).toBe(true)
    expect(store.initialized).toBe(true)
    expect(store.setupToken).toBe('setup-token')
    expect(store.isAuthenticated).toBe(false)
  })

  it('marks the admin session authenticated after successful login', async () => {
    apiMock.login.mockResolvedValueOnce({
      sessionId: 'session-1',
      expiresAt: '2026-01-01T00:00:00Z',
    })

    const { useAuthStore } = await import('./auth')
    const store = useAuthStore()

    await store.login('Admin123!234')

    expect(apiMock.login).toHaveBeenCalledWith({ username: 'admin', password: 'Admin123!234' })
    expect(store.isAuthenticated).toBe(true)
    expect(store.currentUser?.username).toBe('admin')
    expect(store.error).toBeNull()
  })

  it('sets up the first admin password and authenticates the UI session', async () => {
    apiMock.setupBootstrap.mockResolvedValueOnce({ apiKey: 'rk_admin_initial' })
    apiMock.login.mockResolvedValueOnce({
      sessionId: 'session-1',
      expiresAt: '2026-01-01T00:00:00Z',
    })

    const { useAuthStore } = await import('./auth')
    const store = useAuthStore()
    store.setupToken = 'setup-token'

    await store.setupAdminPassword('Admin123!234')

    expect(apiMock.setupBootstrap).toHaveBeenCalledWith({
      setupToken: 'setup-token',
      password: 'Admin123!234',
    })
    expect(apiMock.login).toHaveBeenCalledWith({ username: 'admin', password: 'Admin123!234' })
    expect(store.bootstrapRequired).toBe(false)
    expect(store.initialApiKey).toBe('rk_admin_initial')
    expect(store.isAuthenticated).toBe(true)
  })

  it('requires a setup token before submitting first-time password setup', async () => {
    const { useAuthStore } = await import('./auth')
    const store = useAuthStore()

    await expect(store.setupAdminPassword('Admin123!234')).rejects.toThrow('Setup token is missing')

    expect(apiMock.setupBootstrap).not.toHaveBeenCalled()
    expect(store.error).toBe('Setup token is missing')
  })
})
