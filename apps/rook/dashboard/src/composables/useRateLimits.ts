import { ref } from "vue";
import type {
	CreateRateLimitRequest,
	RateLimitRuleResponse,
	RateLimitScope,
	RateLimitStatusResponse,
} from "@/lib/api";
import { useApi } from "@/lib/api";

export function useRateLimits() {
	const api = useApi();
	const rules = ref<RateLimitRuleResponse[]>([]);
	const status = ref<RateLimitStatusResponse | null>(null);
	const loading = ref(false);
	const saving = ref(false);
	const error = ref<string | null>(null);
	const availability = ref<"available" | "unavailable" | "error">("available");

	async function fetch(): Promise<void> {
		loading.value = true;
		error.value = null;
		try {
			rules.value = await api.getRateLimits();
			availability.value = "available";
		} catch (cause) {
			const message =
				cause instanceof Error ? cause.message : "Failed to load rate limits";
			error.value = message;
			availability.value = message.includes("404") ? "unavailable" : "error";
		} finally {
			loading.value = false;
		}
	}
	async function create(data: CreateRateLimitRequest): Promise<boolean> {
		saving.value = true;
		error.value = null;
		try {
			await api.createRateLimit(data);
			await fetch();
			return true;
		} catch (cause) {
			error.value =
				cause instanceof Error ? cause.message : "Failed to create rate limit";
			return false;
		} finally {
			saving.value = false;
		}
	}
	async function update(
		id: string,
		data: Partial<
			Pick<CreateRateLimitRequest, "requests_per_minute" | "requests_per_day">
		>,
	): Promise<boolean> {
		saving.value = true;
		error.value = null;
		try {
			await api.updateRateLimit(id, data);
			await fetch();
			return true;
		} catch (cause) {
			error.value =
				cause instanceof Error ? cause.message : "Failed to update rate limit";
			return false;
		} finally {
			saving.value = false;
		}
	}
	async function remove(id: string): Promise<boolean> {
		saving.value = true;
		error.value = null;
		try {
			await api.deleteRateLimit(id);
			rules.value = rules.value.filter((rule) => rule.id !== id);
			return true;
		} catch (cause) {
			error.value =
				cause instanceof Error ? cause.message : "Failed to delete rate limit";
			return false;
		} finally {
			saving.value = false;
		}
	}
	async function fetchStatus(
		scope: RateLimitScope,
		target: string,
	): Promise<void> {
		error.value = null;
		try {
			status.value = await api.getRateLimitStatus(scope, target);
		} catch (cause) {
			error.value =
				cause instanceof Error
					? cause.message
					: "Failed to load rate limit status";
		}
	}
	return {
		rules,
		status,
		loading,
		saving,
		error,
		availability,
		fetch,
		create,
		update,
		remove,
		fetchStatus,
	};
}
