import { ref } from "vue";
import type { CircuitStateResponse } from "@/lib/api";
import { useApi } from "@/lib/api";

export function useResilience() {
	const api = useApi();
	const states = ref<CircuitStateResponse[]>([]);
	const loading = ref(false);
	const error = ref<string | null>(null);
	async function fetch() {
		loading.value = true;
		error.value = null;
		try {
			states.value = (await api.getResilience()).circuit_states;
		} catch (cause) {
			error.value =
				cause instanceof Error
					? cause.message
					: "Failed to load resilience state";
		} finally {
			loading.value = false;
		}
	}
	async function reset(provider: string) {
		try {
			await api.resetProviderResilience(provider);
			await fetch();
			return true;
		} catch (cause) {
			error.value =
				cause instanceof Error ? cause.message : "Failed to reset provider";
			return false;
		}
	}
	return { states, loading, error, fetch, reset };
}
