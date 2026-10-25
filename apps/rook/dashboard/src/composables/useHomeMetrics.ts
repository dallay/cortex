import { ref } from "vue";
import type { UsageCostResponse, UsageSummaryResponse } from "@/lib/api";
import { useApi } from "@/lib/api";

export function useHomeMetrics() {
	const api = useApi();
	const apiKeyCount = ref<number | null>(null);
	const usage = ref<UsageSummaryResponse | null>(null);
	const cost = ref<UsageCostResponse | null>(null);
	const loading = ref(false);
	async function fetch() {
		loading.value = true;
		try {
			const results = await Promise.allSettled([
				api.getApiKeyCount(),
				api.getUsageSummary({
					start: new Date(Date.now() - 24 * 60 * 60 * 1000).toISOString(),
				}),
				api.getUsageCost({
					start: new Date(Date.now() - 7 * 24 * 60 * 60 * 1000).toISOString(),
				}),
			]);
			apiKeyCount.value =
				results[0].status === "fulfilled" ? results[0].value : null;
			usage.value = results[1].status === "fulfilled" ? results[1].value : null;
			cost.value = results[2].status === "fulfilled" ? results[2].value : null;
		} finally {
			loading.value = false;
		}
	}
	return { apiKeyCount, usage, cost, loading, fetch };
}
