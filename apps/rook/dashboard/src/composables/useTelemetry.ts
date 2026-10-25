import { computed, ref } from "vue";
import type {
	PercentileStatsResponse,
	ProviderTelemetryResponse,
	TelemetrySummaryResponse,
} from "@/lib/api";
import { useApi } from "@/lib/api";

export function formatMilliseconds(value: number | null | undefined): string {
	if (value == null || !Number.isFinite(value)) return "—";
	return `${Math.round(value)} ms`;
}

export function useTelemetry() {
	const api = useApi();
	const data = ref<TelemetrySummaryResponse | null>(null);
	const provider = ref<ProviderTelemetryResponse | null>(null);
	const latency = ref<PercentileStatsResponse | null>(null);
	const loading = ref(false);
	const error = ref<string | null>(null);
	const availability = ref<"available" | "unavailable" | "error">("available");
	const providers = computed(() => data.value?.providers ?? []);
	async function fetch() {
		loading.value = true;
		error.value = null;
		try {
			data.value = await api.getTelemetrySummary();
			availability.value = "available";
		} catch (cause) {
			const message =
				cause instanceof Error ? cause.message : "Failed to load telemetry";
			error.value = message;
			availability.value = message.includes("501") ? "unavailable" : "error";
		} finally {
			loading.value = false;
		}
	}
	async function inspect(providerId: string) {
		try {
			[provider.value, latency.value] = await Promise.all([
				api.getProviderTelemetry(providerId),
				api.getProviderLatency(providerId),
			]);
		} catch (cause) {
			error.value =
				cause instanceof Error
					? cause.message
					: "Failed to load provider telemetry";
		}
	}
	return {
		data,
		providers,
		provider,
		latency,
		loading,
		error,
		availability,
		fetch,
		inspect,
	};
}
