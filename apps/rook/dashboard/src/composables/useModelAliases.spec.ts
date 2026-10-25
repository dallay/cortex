import { beforeEach, describe, expect, it, vi } from "vitest";

const getModelAliases = vi.fn();
const createModelAlias = vi.fn();
const deleteModelAlias = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({ getModelAliases, createModelAlias, deleteModelAlias }),
}));

describe("useModelAliases", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		getModelAliases.mockResolvedValue([]);
		createModelAlias.mockResolvedValue(undefined);
		deleteModelAlias.mockResolvedValue(undefined);
	});
	it("loads the aliases", async () => {
		getModelAliases.mockResolvedValueOnce([
			{
				alias: "chat-fast",
				canonical: "gpt-4o-mini",
				providerId: null,
				createdAt: "2026-10-24T00:00:00Z",
			},
		]);
		const { useModelAliases } = await import("./useModelAliases");
		const state = useModelAliases();
		await state.fetch();
		expect(state.aliases.value[0]?.alias).toBe("chat-fast");
	});
	it("creates and reloads aliases", async () => {
		const { useModelAliases } = await import("./useModelAliases");
		const state = useModelAliases();
		expect(
			await state.create({
				alias: "chat",
				canonical: "gpt-4o",
				providerId: null,
			}),
		).toBe(true);
		expect(getModelAliases).toHaveBeenCalledTimes(1);
	});
	it("keeps an alias after delete failure", async () => {
		deleteModelAlias.mockRejectedValueOnce(new Error("not found"));
		const { useModelAliases } = await import("./useModelAliases");
		const state = useModelAliases();
		state.aliases.value = [
			{ alias: "chat", canonical: "gpt-4o", providerId: null, createdAt: "" },
		];
		expect(await state.remove("chat")).toBe(false);
		expect(state.aliases.value).toHaveLength(1);
	});
});
