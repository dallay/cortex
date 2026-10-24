import { ref } from "vue";
import type {
	CachedCompletionResponse,
	CacheSignatureResponse,
	CacheStatsResponse,
} from "@/lib/api";
import { useApi } from "@/lib/api";

export function useCache() {
	const api = useApi();
	const stats = ref<CacheStatsResponse | null>(null);
	const signatures = ref<CacheSignatureResponse[]>([]);
	const selectedSignature = ref<CachedCompletionResponse | null>(null);
	const loading = ref(false);
	const error = ref<string | null>(null);
	async function fetch() {
		loading.value = true;
		error.value = null;
		try {
			[stats.value, signatures.value] = await Promise.all([
				api.getCacheStats(),
				api.getCacheSignatures(),
			]);
		} catch (cause) {
			error.value =
				cause instanceof Error ? cause.message : "Failed to load cache";
		} finally {
			loading.value = false;
		}
	}
	async function inspect(signature: string) {
		try {
			selectedSignature.value = await api.getCacheSignature(signature);
		} catch (cause) {
			error.value =
				cause instanceof Error
					? cause.message
					: "Failed to inspect cache entry";
		}
	}
	async function clear() {
		try {
			await api.clearCache();
			selectedSignature.value = null;
			await fetch();
			return true;
		} catch (cause) {
			error.value =
				cause instanceof Error ? cause.message : "Failed to clear cache";
			return false;
		}
	}
	async function remove(signature: string) {
		try {
			await api.deleteCacheSignature(signature);
			signatures.value = signatures.value.filter(
				(entry) => entry.signature !== signature,
			);
			if (selectedSignature.value?.signature === signature)
				selectedSignature.value = null;
			return true;
		} catch (cause) {
			error.value =
				cause instanceof Error ? cause.message : "Failed to remove cache entry";
			return false;
		}
	}
	return {
		stats,
		signatures,
		selectedSignature,
		loading,
		error,
		fetch,
		inspect,
		clear,
		remove,
	};
}
