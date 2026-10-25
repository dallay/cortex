import { mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { defineComponent } from "vue";
import en from "@/locales/en.json";
import ProvidersQuotaView from "./ProvidersQuotaView.vue";

const fetch = vi.fn();
const quotaItem = {
	providerKind: "openai",
	connectionCount: 1,
	activeConnectionCount: 1,
	coverage: "cortex_routed",
	usageAvailability: "available",
	accountQuota: { availability: "not_provided" },
	localThresholds: [{ warning: 0.5, error: 0.8, connectionCount: 1 }],
	rateLimitObservation: {
		source: "recorded_routed_request_status",
		availability: "available",
		window: "last_7d",
		rateLimitedRequests: 1,
		requests: 10,
		ratio: 0.1,
	},
	localWarningLevel: "ok",
	allTime: {
		availability: "available",
		rateLimitAvailability: "available",
		requests: 10,
		rateLimitedRequests: 1,
		promptTokens: 10,
		completionTokens: 10,
		cacheReadTokens: 0,
		cacheCreationTokens: 0,
		reasoningTokens: 0,
		totalTokens: 20,
		costUsd: null as number | null,
		costAvailability: "unknown",
	},
	last24h: {
		availability: "no_data",
		rateLimitAvailability: "available",
		requests: 0,
		rateLimitedRequests: 0,
		promptTokens: 0,
		completionTokens: 0,
		cacheReadTokens: 0,
		reasoningTokens: 0,
		totalTokens: 0,
		costUsd: null,
		costAvailability: "no_data",
	},
	last7d: {
		availability: "available",
		rateLimitAvailability: "available",
		requests: 10,
		rateLimitedRequests: 1,
		promptTokens: 10,
		completionTokens: 10,
		cacheReadTokens: 0,
		reasoningTokens: 0,
		totalTokens: 20,
		costUsd: 1.25 as number | null,
		costAvailability: "complete",
	},
	trend: Array.from({ length: 7 }, (_, index) => ({
		date: `2026-01-0${index + 1}`,
		usageAvailability: "available",
		requests: 10 + index,
		rateLimitedRequests: 1,
		totalTokens: 20 + index,
		costUsd: 0.25,
		costAvailability: "complete",
	})),
};
vi.mock("@/composables/useProvidersQuota", () => ({
	useProvidersQuota: () => ({
		items: { value: [quotaItem] },
		generatedAt: { value: "2026-01-01T00:00:00Z" },
		loading: { value: false },
		error: { value: null },
		stale: { value: false },
		fetch,
	}),
}));
vi.mock("vue-i18n", () => ({
	useI18n: () => ({
		t: (key: string) => {
			const keys = key.split(".");
			let value: unknown = en;
			for (const part of keys)
				value = (value as Record<string, unknown>)?.[part];
			return typeof value === "string" ? value : key;
		},
		d: (date: Date) => (Number.isNaN(date.getTime()) ? "" : date.toISOString()),
		n: (value: number) => `$${value}`,
	}),
}));
const passthrough = defineComponent({ template: "<div><slot /></div>" });

describe("ProvidersQuotaView", () => {
	afterEach(() => {
		quotaItem.last7d.costUsd = 1.25;
		quotaItem.last7d.costAvailability = "complete";
		vi.clearAllMocks();
	});
	it("labels partial cost as unknown without showing a numeric subtotal", () => {
		quotaItem.last7d.costUsd = 1.25;
		quotaItem.last7d.costAvailability = "partial";
		const wrapper = mount(ProvidersQuotaView, {
			global: {
				stubs: {
					Button: passthrough,
					Table: passthrough,
					TableBody: passthrough,
					TableHeader: passthrough,
					TableRow: passthrough,
					TableCell: passthrough,
					TableHead: passthrough,
					Badge: passthrough,
					Alert: passthrough,
					AlertTitle: passthrough,
					AlertDescription: passthrough,
					Fuel: passthrough,
					RefreshCw: passthrough,
				},
			},
		});
		expect(wrapper.text()).toContain("Partial cost; total unknown");
		expect(wrapper.text()).toContain("Unknown");
		expect(
			wrapper.findAll("[data-testid=quota-row-openai] > div")[3].text(),
		).not.toContain("$1.25");
		wrapper.unmount();
	});
	it("renders seven days of trend usage with cost when available", () => {
		const wrapper = mount(ProvidersQuotaView, {
			global: {
				stubs: {
					Button: passthrough,
					Table: passthrough,
					TableBody: passthrough,
					TableHeader: passthrough,
					TableRow: passthrough,
					TableCell: passthrough,
					TableHead: passthrough,
					Badge: passthrough,
					Alert: passthrough,
					AlertTitle: passthrough,
					AlertDescription: passthrough,
					Fuel: passthrough,
					RefreshCw: passthrough,
				},
			},
		});
		expect(wrapper.find("[data-testid=quota-trend-openai]").exists()).toBe(
			true,
		);
		expect(wrapper.findAll("[data-testid=quota-trend-day]")).toHaveLength(7);
		expect(wrapper.text()).toContain("21");
		expect(wrapper.text()).toContain("$0.25");
		wrapper.unmount();
	});
});
