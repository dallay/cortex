/**
 * API Keys composable — CRUD operations for external agent API keys.
 *
 * API keys are service accounts that allow external agents (e.g., opencode, hermes)
 * to authenticate against Rook's OpenAI-compatible APIs.
 *
 * Note: The management API requires session authentication.
 * If not authenticated, calls will fail with 401.
 */
import { ref } from 'vue'
import { useApi, type ApiKeyRecordResponse, type CreateApiKeyRequest, type UpdateApiKeyRequest } from '@/lib/api'

export function useApiKeys() {
  const api = useApi()
  const apiKeys = ref<ApiKeyRecordResponse[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)
  const total = ref(0)
  const limit = ref(20)
  const offset = ref(0)

  async function fetch(keysLimit = 20, keysOffset = 0) {
    loading.value = true
    error.value = null
    limit.value = keysLimit
    offset.value = keysOffset
    try {
      const response = await api.getApiKeys(keysLimit, keysOffset)
      apiKeys.value = response.keys
      total.value = response.pagination.total
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to fetch API keys'
      console.error('[useApiKeys]', e)
    } finally {
      loading.value = false
    }
  }

  async function create(data: CreateApiKeyRequest): Promise<{ key: ApiKeyRecordResponse; plaintextKey: string } | null> {
    try {
      const created = await api.createApiKey(data)
      await fetch(limit.value, offset.value) // Refresh list
      return created
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to create API key'
      return null
    }
  }

  async function update(id: string, data: UpdateApiKeyRequest): Promise<ApiKeyRecordResponse | null> {
    try {
      const updated = await api.updateApiKey(id, data)
      const index = apiKeys.value.findIndex(k => k.id === id)
      if (index !== -1) {
        apiKeys.value[index] = updated
      }
      return updated
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to update API key'
      return null
    }
  }

  async function revoke(id: string): Promise<boolean> {
    try {
      await api.revokeApiKey(id)
      const index = apiKeys.value.findIndex(k => k.id === id)
      if (index !== -1) {
        apiKeys.value[index].isActive = false
        apiKeys.value[index].revokedAt = new Date().toISOString()
      }
      return true
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to revoke API key'
      return false
    }
  }

  async function rotate(id: string): Promise<{ key: ApiKeyRecordResponse; plaintextKey: string } | null> {
    try {
      const result = await api.rotateApiKey(id)
      const index = apiKeys.value.findIndex(k => k.id === id)
      if (index !== -1) {
        // Update keyPrefix in place so the list stays consistent
        apiKeys.value[index].keyPrefix = result.key.keyPrefix
      }
      return result
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to rotate API key'
      return null
    }
  }

  async function remove(id: string): Promise<boolean> {
    return revoke(id) // Soft delete is a revoke
  }

  function nextPage() {
    fetch(limit.value, offset.value + limit.value)
  }

  function prevPage() {
    const newOffset = Math.max(0, offset.value - limit.value)
    fetch(limit.value, newOffset)
  }

  function setPage(page: number) {
    fetch(limit.value, page * limit.value)
  }

  const totalPages = () => Math.ceil(total.value / limit.value)
  const currentPage = () => Math.floor(offset.value / limit.value)

  return {
    apiKeys,
    loading,
    error,
    total,
    limit,
    offset,
    fetch,
    create,
    update,
    revoke,
    rotate,
    remove,
    nextPage,
    prevPage,
    setPage,
    totalPages,
    currentPage,
  }
}