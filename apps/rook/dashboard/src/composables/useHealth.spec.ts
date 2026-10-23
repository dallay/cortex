import { mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, nextTick } from "vue";

const mockGetHealth = vi.fn();

vi.mock("@/lib/api", () => ({
	useApi: () => ({
		getHealth: mockGetHealth,
	}),
}));

describe("useHealth", () => {
	beforeEach(() => {
		mockGetHealth.mockReset();
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	it("defaults appName to Rook and version to empty when health data is null", async () => {
		mockGetHealth.mockReturnValue(new Promise(() => {})); // Never resolves
		const { useHealth } = await import("./useHealth");
		const health = useHealth();

		expect(health.appName.value).toBe("Rook");
		expect(health.version.value).toBe("");
		expect(health.isHealthy.value).toBe(false);
		expect(health.isDegraded.value).toBe(false);
		expect(health.hasProviders.value).toBeNull();
		expect(health.healthyProviders.value).toEqual([]);
		expect(health.unhealthyProviders.value).toEqual([]);
		expect(health.averageLatency.value).toBeNull();
	});

	it("populates appName and version from health endpoint response", async () => {
		mockGetHealth.mockResolvedValue({
			status: "healthy",
			app_name: "Rook Custom",
			version: "0.2.0",
			providers: [],
		});

		const { useHealth } = await import("./useHealth");
		const health = useHealth();
		await nextTick();
		await new Promise((resolve) => setTimeout(resolve, 0));

		expect(health.appName.value).toBe("Rook Custom");
		expect(health.version.value).toBe("v0.2.0");
	});

	it("preserves v prefix if version already starts with v", async () => {
		mockGetHealth.mockResolvedValue({
			status: "healthy",
			app_name: "Rook",
			version: "v1.0.0",
			providers: [],
		});

		const { useHealth } = await import("./useHealth");
		const health = useHealth();
		await health.fetch();
		await nextTick();

		expect(health.version.value).toBe("v1.0.0");
	});

	it("computes provider status, healthy/unhealthy providers, and average latency", async () => {
		mockGetHealth.mockResolvedValue({
			status: "degraded",
			app_name: "Rook",
			version: "0.1.0",
			providers: [
				{ id: "p1", healthy: true, latency_ms: 100, last_error: null },
				{ id: "p2", healthy: false, latency_ms: 200, last_error: "Failed" },
				{ id: "p3", healthy: true, latency_ms: null, last_error: null },
			],
		});

		const { useHealth } = await import("./useHealth");
		const health = useHealth();
		await health.fetch();
		await nextTick();

		expect(health.isHealthy.value).toBe(false);
		expect(health.isDegraded.value).toBe(true);
		expect(health.hasProviders.value).toBe(true);
		expect(health.healthyProviders.value).toHaveLength(2);
		expect(health.healthyProviders.value.map((p) => p.id)).toEqual([
			"p1",
			"p3",
		]);
		expect(health.unhealthyProviders.value).toHaveLength(1);
		expect(health.unhealthyProviders.value[0]!.id).toBe("p2");
		expect(health.averageLatency.value).toBe(150);
	});

	it("handles no_providers_configured status correctly", async () => {
		mockGetHealth.mockResolvedValue({
			status: "no_providers_configured",
			app_name: "Rook",
			version: "0.1.0",
			providers: [],
		});

		const { useHealth } = await import("./useHealth");
		const health = useHealth();
		await health.fetch();
		await nextTick();

		expect(health.isHealthy.value).toBe(false);
		expect(health.isDegraded.value).toBe(false);
		expect(health.hasProviders.value).toBe(false);
		expect(health.averageLatency.value).toBeNull();
	});

	it("handles fetch errors with Error object and string exception", async () => {
		const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});

		mockGetHealth.mockRejectedValueOnce(new Error("Connection refused"));

		const { useHealth } = await import("./useHealth");
		const health = useHealth();
		await health.fetch();
		await nextTick();

		expect(health.error.value).toBe("Connection refused");
		expect(health.loading.value).toBe(false);

		mockGetHealth.mockRejectedValueOnce("String error");
		await health.fetch();
		await nextTick();

		expect(health.error.value).toBe("Failed to fetch health");

		consoleSpy.mockRestore();
	});

	it("calls fetch onMounted when used inside component setup", async () => {
		mockGetHealth.mockResolvedValue({
			status: "healthy",
			app_name: "Rook",
			version: "0.1.0",
			providers: [],
		});

		const { useHealth } = await import("./useHealth");
		const TestComponent = defineComponent({
			setup() {
				const health = useHealth();
				return { health };
			},
			template: "<div>{{ health.appName }}</div>",
		});

		mount(TestComponent);
		await nextTick();

		expect(mockGetHealth).toHaveBeenCalled();
	});
});
