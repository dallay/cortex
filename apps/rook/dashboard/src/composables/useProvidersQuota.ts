import { computed, onMounted, onUnmounted, ref } from "vue";
import { type ProvidersQuotaResponse, useApi } from "@/lib/api";

const REFRESH_INTERVAL_MS = 60_000;

export function useProvidersQuota() {
	const api = useApi();
	const quota = ref<ProvidersQuotaResponse | null>(null);
	const loading = ref(false);
	const error = ref<string | null>(null);
	const stale = computed(() => error.value !== null && quota.value !== null);
	let inFlight: Promise<void> | null = null;
	let timer: ReturnType<typeof setInterval> | undefined;

	function fetch(): Promise<void> {
		if (inFlight) return inFlight;
		loading.value = true;
		error.value = null;
		inFlight = api
			.getProvidersQuota()
			.then((response) => {
				quota.value = response;
			})
			.catch((e: unknown) => {
				error.value =
					e instanceof Error ? e.message : "Failed to fetch provider usage";
			})
			.finally(() => {
				loading.value = false;
				inFlight = null;
			});
		return inFlight;
	}

	onMounted(() => {
		void fetch();
		timer = setInterval(() => void fetch(), REFRESH_INTERVAL_MS);
	});
	onUnmounted(() => {
		if (timer !== undefined) clearInterval(timer);
	});

	return {
		quota,
		items: computed(() => quota.value?.items ?? []),
		generatedAt: computed(() => quota.value?.generatedAt ?? null),
		loading,
		error,
		stale,
		fetch,
	};
}
