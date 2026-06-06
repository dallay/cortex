/**
 * Providers composable — CRUD operations for provider connections.
 *
 * Note: The management API requires session authentication.
 * If not authenticated, calls will fail with 401.
 */
import { ref, computed } from 'vue'
import { useApi, type ProviderConnectionResponse, type CreateProviderRequest, type UpdateProviderRequest, type TestCredentialsPayload, type TestConnectionResponse } from '@/lib/api'

export function useProviders() {
  const api = useApi()
  const providers = ref<ProviderConnectionResponse[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)

  async function fetch() {
    loading.value = true
    error.value = null
    try {
      providers.value = await api.getProviders()
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to fetch providers'
      console.error('[useProviders]', e)
      // Keep existing data on error
    } finally {
      loading.value = false
    }
  }

  async function create(data: CreateProviderRequest): Promise<ProviderConnectionResponse | null> {
    try {
      const created = await api.createProvider(data)
      providers.value.push(created)
      return created
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to create provider'
      return null
    }
  }

  async function update(id: string, data: UpdateProviderRequest): Promise<ProviderConnectionResponse | null> {
    try {
      const updated = await api.updateProvider(id, data)
      const index = providers.value.findIndex(p => p.id === id)
      if (index !== -1) {
        providers.value[index] = updated
      }
      return updated
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to update provider'
      return null
    }
  }

  async function remove(id: string): Promise<boolean> {
    try {
      await api.deleteProvider(id)
      providers.value = providers.value.filter(p => p.id !== id)
      return true
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to delete provider'
      return false
    }
  }

  async function test(id: string) {
    try {
      return await api.testProvider(id)
    } catch (e) {
      console.error('[useProviders] test failed', e)
      return null
    }
  }

  async function testCredentials(
    payload: TestCredentialsPayload
  ): Promise<TestConnectionResponse | null> {
    try {
      return await api.testCredentials(payload)
    } catch (error) {
      console.error('Test credentials failed:', error)
      return null
    }
  }

  const activeProviders = computed(() =>
    providers.value.filter(p => p.testStatus.status === 'active')
  )

  const providerById = computed(() => {
    const map = new Map<string, ProviderConnectionResponse>()
    providers.value.forEach(p => map.set(p.id, p))
    return map
  })

  return {
    providers,
    loading,
    error,
    fetch,
    create,
    update,
    remove,
    test,
    testCredentials,
    activeProviders,
    providerById,
  }
}
