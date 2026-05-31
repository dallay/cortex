/**
 * Health composable — fetches and caches health status from the backend.
 */
import { ref, computed, onMounted } from 'vue'
import { useApi, type HealthResponse } from '@/lib/api'

export function useHealth() {
  const api = useApi()
  const data = ref<HealthResponse | null>(null)
  const loading = ref(false)
  const error = ref<string | null>(null)

  async function fetch() {
    loading.value = true
    error.value = null
    try {
      data.value = await api.getHealth()
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to fetch health'
      console.error('[useHealth]', e)
    } finally {
      loading.value = false
    }
  }

  const isHealthy = computed(() => data.value?.status === 'healthy')
  const isDegraded = computed(() => data.value?.status === 'degraded')
  const hasProviders = computed(() =>
    data.value && data.value.status !== 'no_providers_configured'
  )

  const healthyProviders = computed(() =>
    data.value?.providers.filter(p => p.healthy) ?? []
  )

  const unhealthyProviders = computed(() =>
    data.value?.providers.filter(p => !p.healthy) ?? []
  )

  const averageLatency = computed(() => {
    const latencies = data.value?.providers
      .filter(p => p.latency_ms !== null)
      .map(p => p.latency_ms as number) ?? []

    if (latencies.length === 0) return null
    return Math.round(latencies.reduce((a, b) => a + b, 0) / latencies.length)
  })

  onMounted(fetch)

  return {
    data,
    loading,
    error,
    fetch,
    isHealthy,
    isDegraded,
    hasProviders,
    healthyProviders,
    unhealthyProviders,
    averageLatency,
  }
}
