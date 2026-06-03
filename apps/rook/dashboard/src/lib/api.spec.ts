import { beforeEach, describe, expect, it, vi } from 'vitest'

describe('Rook auth API client', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
    localStorage.clear()
    document.cookie = 'csrf_token=; Max-Age=0; path=/'
  })

  it('fetches a CSRF token before logging in and echoes it in the login header', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ csrf_token: 'csrf-from-login' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ session_id: 'session-1', expires_at: '2026-01-01T00:00:00Z' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))

    vi.stubGlobal('fetch', fetchMock)
    vi.resetModules()
    const { useApi } = await import('./api')

    const result = await useApi().login({ username: 'admin', password: 'test-fixture-password' })

    expect(result.sessionId).toBe('session-1')
    expect(fetchMock).toHaveBeenNthCalledWith(1, '/login', expect.objectContaining({
      credentials: 'include',
    }))
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/login', expect.objectContaining({
      method: 'POST',
      credentials: 'include',
      headers: expect.objectContaining({
        'Content-Type': 'application/json',
        'X-CSRF-Token': 'csrf-from-login',
      }),
      body: JSON.stringify({ username: 'admin', password: 'test-fixture-password' }),
    }))
  })

  it('fetches bootstrap status from the public bootstrap endpoint', async () => {
    const fetchMock = vi.fn().mockResolvedValueOnce(new Response(JSON.stringify({
      is_initialized: false,
      admin_user_exists: true,
      // SECURITY: setup_token must NEVER appear in the status response body.
      // The token is out-of-band only (printed to server logs at startup).
    }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    }))

    vi.stubGlobal('fetch', fetchMock)
    vi.resetModules()
    const { useApi } = await import('./api')

    const status = await useApi().getBootstrapStatus()

    // setup_token must not be present in the response — it is an out-of-band
    // secret only printed to server logs. Exposing it via HTTP would allow
    // unauthenticated remote takeover of fresh installations.
    expect(status).toEqual({
      isInitialized: false,
      adminUserExists: true,
    })
    expect(status).not.toHaveProperty('setupToken')
    expect(fetchMock).toHaveBeenCalledWith('/api/bootstrap/status', expect.objectContaining({
      credentials: 'include',
    }))
  })

  it('uses a CSRF token when submitting the first-time admin password', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ csrf_token: 'csrf-for-setup' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ api_key: 'rk_admin_initial' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))

    vi.stubGlobal('fetch', fetchMock)
    vi.resetModules()
    const { useApi } = await import('./api')

    const result = await useApi().setupBootstrap({ setupToken: 'setup-token', password: 'test-fixture-password' })

    expect(result.apiKey).toBe('rk_admin_initial')
    expect(fetchMock).toHaveBeenNthCalledWith(2, '/api/bootstrap/setup', expect.objectContaining({
      method: 'POST',
      credentials: 'include',
      headers: expect.objectContaining({
        'Content-Type': 'application/json',
        'X-CSRF-Token': 'csrf-for-setup',
      }),
      body: JSON.stringify({ setup_token: 'setup-token', password: 'test-fixture-password' }),
    }))
  })

  it('fetches available model groups from /api/models with session credentials', async () => {
    const fetchMock = vi.fn().mockResolvedValueOnce(new Response(JSON.stringify({
      models: [
        {
          providerId: '00000000-0000-0000-0000-000000000001',
          providerName: 'OpenAI Primary',
          providerKind: 'openai',
          models: ['gpt-4o', 'gpt-4-turbo'],
        },
        {
          providerId: '00000000-0000-0000-0000-000000000002',
          providerName: 'Anthropic Primary',
          providerKind: 'anthropic',
          models: ['claude-3-5-sonnet-latest'],
        },
      ],
    }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    }))

    vi.stubGlobal('fetch', fetchMock)
    vi.resetModules()
    const { useApi } = await import('./api')

    const result = await useApi().getAvailableModels()

    expect(result.models).toHaveLength(2)
    expect(result.models[0]!.providerId).toBe('00000000-0000-0000-0000-000000000001')
    expect(result.models[0]!.providerName).toBe('OpenAI Primary')
    expect(result.models[0]!.providerKind).toBe('openai')
    expect(result.models[0]!.models).toEqual(['gpt-4o', 'gpt-4-turbo'])
    expect(result.models[1]!.models).toEqual(['claude-3-5-sonnet-latest'])
    expect(fetchMock).toHaveBeenCalledWith('/api/models', expect.objectContaining({
      credentials: 'include',
    }))
  })

  it('returns an empty models list when no providers are configured', async () => {
    const fetchMock = vi.fn().mockResolvedValueOnce(new Response(JSON.stringify({ models: [] }), {
      status: 200,
      headers: { 'Content-Type': 'application/json' },
    }))

    vi.stubGlobal('fetch', fetchMock)
    vi.resetModules()
    const { useApi } = await import('./api')

    const result = await useApi().getAvailableModels()

    expect(result.models).toEqual([])
  })
})
