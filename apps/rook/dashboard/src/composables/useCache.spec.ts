import { beforeEach, describe, expect, it, vi } from "vitest";

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
describe("useCache", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getCacheStats.mockResolvedValue({
			hits: 1,
			misses: 2,
			evictions: 0,
			entries: 1,
			max_entries: 10,
		});
		getCacheSignatures.mockResolvedValue([]);
		clearCache.mockResolvedValue(undefined);
		deleteCacheSignature.mockResolvedValue(undefined);
	});
	it("loads stats and signature inventory", async () => {
		const { useCache } = await import("./useCache");
		const state = useCache();
		await state.fetch();
		expect(state.stats.value?.hits).toBe(1);
	});
	it("clears and removes signatures after successful invalidation", async () => {
		const { useCache } = await import("./useCache");
		const state = useCache();
		await state.fetch();
		state.signatures.value = [
			{
				signature: "a".repeat(64),
				request_id: "r",
				model: "m",
				provider: "p",
				cached_at: "",
				last_accessed: "",
				request_metadata: {
					message_count: 0,
					has_tools: false,
					max_tokens: null,
					temperature: null,
				},
			},
		];
		await state.clear();
		expect(getCacheStats).toHaveBeenCalled();
		await state.remove("a".repeat(64));
		expect(state.signatures.value).toHaveLength(0);
	});
});
