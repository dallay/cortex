import { afterEach, describe, expect, it, vi } from "vitest";
import { useProviderDescriptors } from "@/composables/useProviderDescriptors";
import { useApi } from "@/lib/api";

describe("useProviderDescriptors", () => {
	afterEach(() => vi.restoreAllMocks());

	it("loads descriptors and includes provider IDs absent from presentation metadata", async () => {
		vi.spyOn(useApi(), "getProviderDescriptors").mockResolvedValue([
			{
				id: "future-provider",
				label: "Future Provider",
				protocol: "openAiChat",
				authKinds: ["apiKey"],
				capabilityCeiling: {
					tools: "unknown",
					vision: "unknown",
					reasoning: "unknown",
					streaming: "unknown",
					contextTokens: null,
				},
				supportsOAuth: false,
				iconKey: "future-provider",
			},
		]);
		const state = useProviderDescriptors();
		await state.fetch();
		expect(state.catalog.value.map((item) => item.kind)).toEqual([
			"future-provider",
		]);
		expect(state.catalog.value[0].label).toBe("Future Provider");
		expect(state.loading.value).toBe(false);
		expect(state.error.value).toBeNull();
	});

	it("maps descriptor identity, icon key, and OAuth credential support", async () => {
		vi.spyOn(useApi(), "getProviderDescriptors").mockResolvedValue([
			{
				id: "openai-compatible",
				label: "OpenAI Compatible",
				protocol: "openAiChat",
				authKinds: ["apiKey", "oauth"],
				capabilityCeiling: {
					tools: "unknown",
					vision: "unknown",
					reasoning: "unknown",
					streaming: "unknown",
					contextTokens: null,
				},
				supportsOAuth: false,
				iconKey: "openai",
			},
		]);
		const state = useProviderDescriptors();
		await state.fetch();
		expect(state.catalog.value[0]).toMatchObject({
			kind: "openai-compatible",
			label: "OpenAI Compatible",
			iconKey: "openai",
			authTypes: ["apikey", "oauth"],
		});
	});

	it("exposes request errors and clears loading state", async () => {
		vi.spyOn(useApi(), "getProviderDescriptors").mockRejectedValue(
			new Error("not authorized"),
		);
		const state = useProviderDescriptors();
		await state.fetch();
		expect(state.error.value).toBe("not authorized");
		expect(state.loading.value).toBe(false);
	});
});
