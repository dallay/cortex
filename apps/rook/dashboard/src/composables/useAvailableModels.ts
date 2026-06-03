/**
 * useAvailableModels — composable that returns the model ids the
 * dashboard can offer in the API key restriction UI, grouped by
 * active provider connection.
 *
 * Data flow:
 *   1. Fetches the model catalog from `GET /api/models` (a list of
 *      groups, one per active provider connection).
 *   2. Crosses the response with the list of active providers from
 *      `useProviders()` so the consumer has the full
 *      `ProviderConnectionResponse` for each group (for display
 *      purposes, e.g. the provider name in the form).
 *   3. Exposes `modelsByProvider` (the cross) and the standard
 *      `loading` / `error` / `fetch` API.
 *
 * The HTTP-level fetch is intentionally separated from the
 * composable's own reactivity: callers can decide whether to
 * pre-fetch or fetch-on-demand.
 */
import { computed, ref, type ComputedRef, type Ref } from 'vue'
import { useApi, type ProviderConnectionResponse, type ProviderModelsGroup } from '@/lib/api'
import { useProviders } from '@/composables/useProviders'

export interface ModelsByProvider {
  provider: ProviderConnectionResponse
  models: string[]
}

export function useAvailableModels() {
  const api = useApi()
  const { providers, fetch: fetchProviders } = useProviders()

  const groups = ref<ProviderModelsGroup[]>([])
  const loading = ref(false)
  const error = ref<string | null>(null)
  const fetched = ref(false)

  async function fetch() {
    loading.value = true
    error.value = null
    try {
      const response = await api.getAvailableModels()
      groups.value = response.models
      fetched.value = true
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to fetch available models'
      // Keep `groups` as-is so the UI degrades gracefully.
    } finally {
      loading.value = false
    }
  }

  /**
   * The cross between active providers (from useProviders) and the
   * model catalog (from the API). Defensive filters handle the case
   * where a provider is no longer active or has no models in the
   * catalog — the server already filters, but we re-apply here to
   * stay safe.
   */
  const modelsByProvider: ComputedRef<ModelsByProvider[]> = computed(() => {
    if (!fetched.value) return []
    const byId = new Map<string, ProviderModelsGroup>()
    for (const group of groups.value) {
      byId.set(group.providerId, group)
    }
    return providers.value
      .filter((p) => p.isActive)
      .map((provider) => {
        const group = byId.get(provider.id)
        return {
          provider,
          models: group?.models ?? [],
        }
      })
      .filter((entry) => entry.models.length > 0)
  })

  return {
    groups: groups as Ref<ProviderModelsGroup[]>,
    modelsByProvider,
    loading: loading as Ref<boolean>,
    error: error as Ref<string | null>,
    fetched: fetched as Ref<boolean>,
    fetch,
    // Re-exported so callers can pre-fetch providers in parallel.
    fetchProviders,
  }
}
