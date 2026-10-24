import { computed, ref } from "vue";
import type {
	UsageCostResponse,
	UsageEntryResponse,
	UsageQuery,
	UsageSummaryResponse,
} from "@/lib/api";
import { useApi } from "@/lib/api";

export type UsageAvailability = "available" | "unavailable" | "error";

export function useUsage() {
	const api = useApi();
	const entries = ref<UsageEntryResponse[]>([]);
	const total = ref(0);
	const summary = ref<UsageSummaryResponse | null>(null);
	const cost = ref<UsageCostResponse | null>(null);
	const loading = ref(false);
	const error = ref<string | null>(null);
	const availability = ref<UsageAvailability>("available");
	const query = ref<UsageQuery>({ offset: 0, limit: 25 });

	const costStatus = computed(() => {
		if (
			!cost.value ||
			cost.value.known_cost_entries + cost.value.unknown_cost_entries === 0
		)
			return "no_data";
		if (cost.value.known_cost_entries === 0) return "unknown";
		if (cost.value.unknown_cost_entries > 0) return "partial";
		return "complete";
	});

	async function fetch(nextQuery: UsageQuery = query.value): Promise<void> {
		query.value = { ...nextQuery };
		loading.value = true;
		error.value = null;
		availability.value = "available";
		try {
			const results = await Promise.all([
				api.getUsage(query.value),
				api.getUsageSummary(query.value),
				api.getUsageCost(query.value),
			]);
			entries.value = results[0].entries;
			total.value = results[0].total;
			summary.value = results[1];
			cost.value = results[2];
		} catch (cause) {
			const message =
				cause instanceof Error ? cause.message : "Failed to load usage";
			if (
				message.includes("503") ||
				message.includes("USAGE_RECORDER_UNAVAILABLE")
			)
				availability.value = "unavailable";
			else availability.value = "error";
			error.value = message;
		} finally {
			loading.value = false;
		}
	}

	return {
		entries,
		total,
		summary,
		cost,
		costStatus,
		loading,
		error,
		availability,
		query,
		fetch,
	};
}
