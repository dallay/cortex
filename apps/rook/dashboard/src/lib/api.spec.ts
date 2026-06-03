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

  // Regression: WebKit (Safari) cookie-jar race. Each state-changing request
  // re-fetched GET /login, and Safari sometimes registered the new csrf_token
  // cookie after the body had already been read but before the next fetch
  // fired. Caching the token in memory collapses the per-request round-trip
  // and removes the race entirely. See issue #82.
  it('caches the CSRF token across state-changing requests', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ csrf_token: 'csrf-cached' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ session_id: 's1', expires_at: '2026-01-01T00:00:00Z' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ session_id: 's2', expires_at: '2026-01-01T00:00:00Z' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))

    vi.stubGlobal('fetch', fetchMock)
    vi.resetModules()
    const { useApi } = await import('./api')

    await useApi().login({ username: 'admin', password: 'test-fixture-password' })
    await useApi().login({ username: 'admin', password: 'test-fixture-password' })

    // Exactly one GET /login across the two state-changing calls
    const getLoginCalls = fetchMock.mock.calls.filter(([url, init]) => {
      const method = ((init as RequestInit | undefined)?.method ?? 'GET').toUpperCase()
      return url === '/login' && method === 'GET'
    })
    expect(getLoginCalls).toHaveLength(1)

    // Both POSTs must carry the same cached token
    const loginPost1 = fetchMock.mock.calls[1]?.[1] as RequestInit | undefined
    const loginPost2 = fetchMock.mock.calls[2]?.[1] as RequestInit | undefined
    expect((loginPost1?.headers as Record<string, string> | undefined)?.['X-CSRF-Token']).toBe('csrf-cached')
    expect((loginPost2?.headers as Record<string, string> | undefined)?.['X-CSRF-Token']).toBe('csrf-cached')
  })

  // Regression: even with the in-memory cache, the first request after
  // server-side token rotation or after a cookie-jar failure must retry with
  // a fresh token instead of propagating csrf_missing to the caller.
  it('refetches the CSRF token and retries when the backend returns csrf_missing', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ csrf_token: 'csrf-stale' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ error: 'csrf_missing', message: 'CSRF token required' }), {
        status: 403,
        headers: { 'Content-Type': 'application/json' },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ csrf_token: 'csrf-fresh' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ session_id: 's1', expires_at: '2026-01-01T00:00:00Z' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))

    vi.stubGlobal('fetch', fetchMock)
    vi.resetModules()
    const { useApi } = await import('./api')

    const result = await useApi().login({ username: 'admin', password: 'test-fixture-password' })

    expect(result.sessionId).toBe('s1')
    // 1 CSRF fetch + 1 failing POST + 1 CSRF refetch + 1 retry POST = 4
    expect(fetchMock).toHaveBeenCalledTimes(4)

    // The retry must use the freshly fetched token, not the stale one
    const retryPost = fetchMock.mock.calls[3]?.[1] as RequestInit | undefined
    expect((retryPost?.headers as Record<string, string> | undefined)?.['X-CSRF-Token']).toBe('csrf-fresh')
  })

  // Regression: when the backend refuses to issue a CSRF token (network down,
  // 5xx, etc.) the request fails with csrf_missing. Previously the failure
  // was swallowed silently and surfaced only as a generic 403 in the dialog
  // — issue #82 lists "stop silently swallowing errors" as fix #4. We now
  // surface a console.warn so future regressions are visible in DevTools.
  it('logs a warning when the CSRF token fetch fails', async () => {
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {})

    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response('backend exploded', {
        status: 500,
        headers: { 'Content-Type': 'text/plain' },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({ error: 'csrf_missing', message: 'CSRF token required' }), {
        status: 403,
        headers: { 'Content-Type': 'application/json' },
      }))
      .mockResolvedValueOnce(new Response('backend still down', {
        status: 500,
        headers: { 'Content-Type': 'text/plain' },
      }))

    vi.stubGlobal('fetch', fetchMock)
    vi.resetModules()
    const { useApi } = await import('./api')

    await expect(
      useApi().createApiKey({
        label: 'test',
        scopes: ['chat:read'],
        tier: 'Free',
        expiresAt: null,
      })
    ).rejects.toThrow(/csrf_missing/)

    // console.warn must have fired at least once with a CSRF-related message
    expect(warnSpy).toHaveBeenCalled()
    const messages = warnSpy.mock.calls.map((args) => String(args[0] ?? ''))
    expect(messages.some((m) => /CSRF/i.test(m))).toBe(true)

warnSpy.mockRestore()
  })

  // Regression: after a successful login, the CSRF token from the login
  // response body must seed the cache so the very first state-changing
  // request after login has a cached token and never needs GET /login.
  // Without this, the page had to make GET /login after every login to
  // fetch the CSRF token, which races with the Set-Cookie landing in
  // WebKit's cookie jar and caused the 403 csrf_missing on the first
  // state-changing request. See issue #82 (suggested fix #2).
  it('seeds the CSRF cache from the login response body', async () => {
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify({ csrf_token: 'csrf-before-login' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
      .mockResolvedValueOnce(new Response(JSON.stringify({
        session_id: 'session-after-login',
        expires_at: '2026-01-01T00:00:00Z',
        csrf_token: 'csrf-from-login-body', // ← the login response ALSO carries a token
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
      // Subsequent POST (createApiKey) — must NOT call GET /login again
      .mockResolvedValueOnce(new Response(JSON.stringify({
        key: { id: 'k1', label: 'test' },
        plaintextKey: 'rk_test_abc',
      }), {
        status: 201,
        headers: { 'Content-Type': 'application/json' },
      }))

    vi.stubGlobal('fetch', fetchMock)
    vi.resetModules()
    const { useApi } = await import('./api')

    await useApi().login({ username: 'admin', password: 'test-fixture-password' })

    // Now make a state-changing request (no GET /login should fire)
    await useApi().createApiKey({
      label: 'test',
      scopes: ['chat:read'],
      tier: 'Free',
      expiresAt: null,
    })

    const getLoginCalls = fetchMock.mock.calls.filter(([url, init]) => {
      const method = ((init as RequestInit | undefined)?.method ?? 'GET').toUpperCase()
      return url === '/login' && method === 'GET'
    })
    // Only the token-fetch for the login POST itself — NOT for the createApiKey
    expect(getLoginCalls).toHaveLength(1)
  })

  // The login response body carries a fresh csrf_token (the same one the
  // backend sets as the cookie). The cache must be seeded from the login
  // response body, NOT from the pre-login GET /login. This matters because
  // in WebKit, the cookie from POST /login's Set-Cookie may not be visible
  // to the next request — but if the cache holds the matching token from the
  // response body, the double-submit succeeds on the retry path. The cache
  // must therefore be OVERWRITTEN after a successful login, not just set
  // by the pre-login GET. See issue #82 (suggested fix #2).
  it('overwrites the cached CSRF token with the one from the login response body', async () => {
    const fetchMock = vi.fn()
      // Pre-login GET /login — returns token "csrf-before"
      .mockResolvedValueOnce(new Response(JSON.stringify({ csrf_token: 'csrf-before' }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
      // POST /login — returns a DIFFERENT token "csrf-from-login-body"
      .mockResolvedValueOnce(new Response(JSON.stringify({
        session_id: 's1',
        expires_at: '2026-01-01T00:00:00Z',
        csrf_token: 'csrf-from-login-body', // ← backend sets THIS as the real cookie
      }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))
      // Next state-changing call — must use "csrf-from-login-body", NOT "csrf-before"
      .mockResolvedValueOnce(new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }))

    vi.stubGlobal('fetch', fetchMock)
    vi.resetModules()
    const { useApi } = await import('./api')

    await useApi().login({ username: 'admin', password: 'test-fixture-password' })
    await useApi().logout()

    const postCall = fetchMock.mock.calls[2]?.[1] as RequestInit | undefined
    expect((postCall?.headers as Record<string, string> | undefined)?.['X-CSRF-Token']).toBe(
      'csrf-from-login-body'
    )
  })
})
