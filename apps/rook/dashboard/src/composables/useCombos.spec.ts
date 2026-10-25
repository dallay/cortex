import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ComboResponse, CreateComboRequest } from "@/lib/api";

const mockGetCombos = vi.fn();
const mockCreateCombo = vi.fn();
const mockUpdateCombo = vi.fn();
const mockDeleteCombo = vi.fn();
const mockGetComboAliases = vi.fn();
const mockCreateComboAlias = vi.fn();
const mockDeleteComboAlias = vi.fn();

vi.mock("@/lib/api", () => ({
	useApi: () => ({
		getCombos: mockGetCombos,
		createCombo: mockCreateCombo,
		updateCombo: mockUpdateCombo,
		deleteCombo: mockDeleteCombo,
		getComboAliases: mockGetComboAliases,
		createComboAlias: mockCreateComboAlias,
		deleteComboAlias: mockDeleteComboAlias,
	}),
}));

function combo(overrides: Partial<ComboResponse> = {}): ComboResponse {
	return {
		id: "combo-1",
		name: "Production fallback",
		strategy: "priority",
		steps: [
			{
				providerId: "provider-openai",
				model: "gpt-4o",
				connectionId: null,
				priority: 1,
			},
		],
		createdAt: "2026-10-24T00:00:00Z",
		updatedAt: "2026-10-24T00:00:00Z",
		...overrides,
	};
}

const payload: CreateComboRequest = {
	name: "Fast responses",
	strategy: "priority",
	steps: [
		{
			providerId: "provider-anthropic",
			model: "claude-3-5-sonnet-latest",
			connectionId: null,
			priority: 1,
		},
	],
};

describe("useCombos", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockGetCombos.mockResolvedValue({ combos: [] });
		mockCreateCombo.mockResolvedValue(combo({ id: "combo-new" }));
		mockUpdateCombo.mockResolvedValue(combo({ name: "Renamed" }));
		mockDeleteCombo.mockResolvedValue(undefined);
		mockGetComboAliases.mockResolvedValue([]);
		mockCreateComboAlias.mockResolvedValue({
			alias: "rook/coding",
			comboId: "combo-1",
		});
		mockDeleteComboAlias.mockResolvedValue(undefined);
	});

	it("loads combos and clears the previous error", async () => {
		mockGetCombos.mockResolvedValueOnce({ combos: [combo()] });
		const { useCombos } = await import("./useCombos");
		const store = useCombos();

		await store.fetch();

		expect(store.combos.value).toHaveLength(1);
		expect(store.combos.value[0]?.name).toBe("Production fallback");
		expect(store.loading.value).toBe(false);
		expect(store.error.value).toBeNull();
	});

	it("surfaces the failure message when the list cannot be loaded", async () => {
		mockGetCombos.mockRejectedValueOnce(
			new Error("API Error 503: unavailable"),
		);
		const { useCombos } = await import("./useCombos");
		const store = useCombos();

		await store.fetch();

		expect(store.combos.value).toEqual([]);
		expect(store.error.value).toBe("API Error 503: unavailable");
		expect(store.loading.value).toBe(false);
	});

	it("creates a combo and refreshes the list", async () => {
		const { useCombos } = await import("./useCombos");
		const store = useCombos();

		const created = await store.create(payload);

		expect(mockCreateCombo).toHaveBeenCalledWith(payload);
		expect(created?.id).toBe("combo-new");
		expect(mockGetCombos).toHaveBeenCalledTimes(1);
	});

	it("returns null and keeps the error when creation fails", async () => {
		mockCreateCombo.mockRejectedValueOnce(
			new Error("API Error 400: combo name cannot be empty"),
		);
		const { useCombos } = await import("./useCombos");
		const store = useCombos();

		const created = await store.create(payload);

		expect(created).toBeNull();
		expect(store.error.value).toBe("API Error 400: combo name cannot be empty");
		expect(mockGetCombos).not.toHaveBeenCalled();
	});

	it("replaces the updated combo in place without refetching", async () => {
		const { useCombos } = await import("./useCombos");
		const store = useCombos();
		store.combos.value = [combo(), combo({ id: "combo-2", name: "Other" })];

		const updated = await store.update("combo-1", payload);

		expect(mockUpdateCombo).toHaveBeenCalledWith("combo-1", payload);
		expect(updated?.name).toBe("Renamed");
		expect(store.combos.value.map((item) => item.name)).toEqual([
			"Renamed",
			"Other",
		]);
		expect(mockGetCombos).not.toHaveBeenCalled();
	});

	it("removes a combo from the list after a successful delete", async () => {
		const { useCombos } = await import("./useCombos");
		const store = useCombos();
		store.combos.value = [combo(), combo({ id: "combo-2" })];

		expect(await store.remove("combo-1")).toBe(true);
		expect(mockDeleteCombo).toHaveBeenCalledWith("combo-1");
		expect(store.combos.value.map((item) => item.id)).toEqual(["combo-2"]);
	});

	it("loads aliases for a combo and stores them by combo id", async () => {
		mockGetComboAliases.mockResolvedValueOnce([
			{ alias: "rook/coding", comboId: "combo-1" },
		]);
		const { useCombos } = await import("./useCombos");
		const store = useCombos();

		await store.fetchAliases("combo-1");

		expect(mockGetComboAliases).toHaveBeenCalledWith("combo-1");
		expect(store.aliasesByComboId.value["combo-1"]).toEqual([
			{ alias: "rook/coding", comboId: "combo-1" },
		]);
		expect(store.aliasError.value["combo-1"]).toBeNull();
	});

	it("creates an alias and appends it to the combo aliases", async () => {
		const { useCombos } = await import("./useCombos");
		const store = useCombos();

		const created = await store.createAlias("combo-1", "rook/coding");

		expect(mockCreateComboAlias).toHaveBeenCalledWith("combo-1", "rook/coding");
		expect(created?.alias).toBe("rook/coding");
		expect(store.aliasesByComboId.value["combo-1"]).toEqual([
			{ alias: "rook/coding", comboId: "combo-1" },
		]);
	});

	it("removes an alias locally only after a successful delete", async () => {
		const { useCombos } = await import("./useCombos");
		const store = useCombos();
		store.aliasesByComboId.value["combo-1"] = [
			{ alias: "rook/coding", comboId: "combo-1" },
			{ alias: "rook/chat", comboId: "combo-1" },
		];

		expect(await store.removeAlias("combo-1", "rook/coding")).toBe(true);
		expect(mockDeleteComboAlias).toHaveBeenCalledWith("combo-1", "rook/coding");
		expect(store.aliasesByComboId.value["combo-1"]).toEqual([
			{ alias: "rook/chat", comboId: "combo-1" },
		]);
	});

	it("keeps an alias when the delete fails", async () => {
		mockDeleteComboAlias.mockRejectedValueOnce(
			new Error("API Error 404: not found"),
		);
		const { useCombos } = await import("./useCombos");
		const store = useCombos();
		store.aliasesByComboId.value["combo-1"] = [
			{ alias: "rook/coding", comboId: "combo-1" },
		];

		expect(await store.removeAlias("combo-1", "rook/coding")).toBe(false);
		expect(store.aliasesByComboId.value["combo-1"]).toHaveLength(1);
		expect(store.aliasError.value["combo-1"]).toBe("API Error 404: not found");
	});

	it("keeps the combo when the delete fails", async () => {
		mockDeleteCombo.mockRejectedValueOnce(
			new Error("API Error 404: not found"),
		);
		const { useCombos } = await import("./useCombos");
		const store = useCombos();
		store.combos.value = [combo()];

		expect(await store.remove("combo-1")).toBe(false);
		expect(store.error.value).toBe("API Error 404: not found");
		expect(store.combos.value).toHaveLength(1);
	});
});
