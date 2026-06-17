import {computed, ref} from "vue";
import {type ProvidersQuotaResponse, useApi} from "@/lib/api";

export function useProvidersQuota() {
  const api = useApi();
  const quota = ref<ProvidersQuotaResponse | null>(null);
  const loading = ref(false);
  const error = ref<string | null>(null);

  async function fetch() {
    loading.value = true;
    error.value = null;
    try {
      quota.value = await api.getProvidersQuota();
    } catch (e) {
      error.value = e instanceof Error ? e.message : "Failed to fetch provider quota";
      console.error("[useProvidersQuota]", e);
    } finally {
      loading.value = false;
    }
  }

  const items = computed(() => quota.value?.items ?? []);
  const generatedAt = computed(() => quota.value?.generatedAt ?? null);

  return {
    quota,
    items,
    generatedAt,
    loading,
    error,
    fetch,
  };
}
