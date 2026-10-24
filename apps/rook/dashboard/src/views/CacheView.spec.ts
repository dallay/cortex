import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import CacheView from "./CacheView.vue";

const getCacheStats = vi.fn();
const getCacheSignatures = vi.fn();
const getCacheSignature = vi.fn();
const clearCache = vi.fn();
const deleteCacheSignature = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({
		getCacheStats,
		getCacheSignatures,
		getCacheSignature,
		clearCache,
		deleteCacheSignature,
	}),
}));
const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });
describe("CacheView", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getCacheStats.mockResolvedValue({
			hits: 9,
			misses: 3,
			evictions: 1,
			entries: 1,
			max_entries: 10,
		});
		getCacheSignatures.mockResolvedValue([
			{
				signature: "a".repeat(64),
				request_id: "req-1",
				model: "gpt-4o",
				provider: "openai",
				cached_at: "2026-10-24T00:00:00Z",
				last_accessed: "2026-10-24T00:01:00Z",
				request_metadata: {
					message_count: 2,
					has_tools: false,
					max_tokens: null,
					temperature: null,
				},
			},
		]);
		getCacheSignature.mockResolvedValue({});
		clearCache.mockResolvedValue(undefined);
		deleteCacheSignature.mockResolvedValue(undefined);
	});
	it("shows cache hit metrics and signature metadata", async () => {
		const wrapper = mount(CacheView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("75%");
		expect(wrapper.text()).toContain("req-1");
	});
	it("confirms full invalidation before calling the API", async () => {
		vi.stubGlobal(
			"confirm",
			vi.fn(() => false),
		);
		const wrapper = mount(CacheView, { global: { plugins: [i18n] } });
		await flushPromises();
		await wrapper.find('[data-testid="cache-clear"]').trigger("click");
		expect(clearCache).not.toHaveBeenCalled();
	});
});
