import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h } from "vue";
import { createI18n } from "vue-i18n";
import { navigationConfig } from "@/config/navigation";
import type { ComboResponse, ProviderConnectionResponse } from "@/lib/api";
import en from "@/locales/en.json";
import CombosView from "./CombosView.vue";

const mockGetCombos = vi.fn();
const mockCreateCombo = vi.fn();
const mockUpdateCombo = vi.fn();
const mockDeleteCombo = vi.fn();
const mockGetComboAliases = vi.fn();
const mockCreateComboAlias = vi.fn();
const mockDeleteComboAlias = vi.fn();
const mockGetProviders = vi.fn();
const mockGetAvailableModels = vi.fn();

vi.mock("@/lib/api", () => ({
	useApi: () => ({
		getCombos: mockGetCombos,
		createCombo: mockCreateCombo,
		updateCombo: mockUpdateCombo,
		deleteCombo: mockDeleteCombo,
		getComboAliases: mockGetComboAliases,
		createComboAlias: mockCreateComboAlias,
		deleteComboAlias: mockDeleteComboAlias,
		getProviders: mockGetProviders,
		getAvailableModels: mockGetAvailableModels,
	}),
}));

// The shadcn Button pulls in reka-ui + class-variance-authority. Rendering it
// as a plain <button> keeps the assertions on real DOM nodes.
vi.mock("@/components/ui/button", () => ({
	Button: defineComponent({
		props: {
			type: String,
			variant: String,
			size: String,
			disabled: Boolean,
			dataTestid: String,
		},
		emits: ["click"],
		setup(props, { emit, slots }) {
			return () =>
				h(
					"button",
					{
						type: props.type ?? "button",
						disabled: props.disabled,
						"data-testid": props.dataTestid,
						onClick: () => emit("click"),
					},
					slots.default?.(),
				);
		},
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
			{
				providerId: "provider-anthropic",
				model: "claude-3-5-sonnet-latest",
				connectionId: null,
				priority: 2,
			},
		],
		createdAt: "2026-10-24T00:00:00Z",
		updatedAt: "2026-10-24T00:00:00Z",
		...overrides,
	};
}

function provider(
	overrides: Partial<ProviderConnectionResponse> = {},
): ProviderConnectionResponse {
	return {
		id: "provider-openai",
		providerKind: "openai",
		providerRuntimeId: "openai",
		authType: "apiKey",
		name: "OpenAI Primary",
		priority: 1,
		isActive: true,
		config: { maxConcurrent: 10 },
		testStatus: { status: "connected" },
		createdAt: "2026-01-01T00:00:00Z",
		updatedAt: "2026-01-01T00:00:00Z",
		...overrides,
	} as ProviderConnectionResponse;
}

function mountView() {
	const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });
	return mount(CombosView, { global: { plugins: [i18n] } });
}

describe("CombosView", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		vi.unstubAllGlobals();
		mockGetCombos.mockResolvedValue({ combos: [combo()] });
		mockCreateCombo.mockResolvedValue(combo({ id: "combo-new" }));
		mockUpdateCombo.mockResolvedValue(combo({ name: "Renamed" }));
		mockDeleteCombo.mockResolvedValue(undefined);
		mockGetComboAliases.mockResolvedValue([
			{ alias: "rook/coding", comboId: "combo-1" },
		]);
		mockCreateComboAlias.mockResolvedValue({
			alias: "rook/reasoning",
			comboId: "combo-1",
		});
		mockDeleteComboAlias.mockResolvedValue(undefined);
		mockGetProviders.mockResolvedValue([
			provider(),
			provider({
				id: "provider-anthropic",
				providerKind: "anthropic",
				name: "Anthropic Primary",
			}),
		]);
		mockGetAvailableModels.mockResolvedValue({
			models: [
				{
					providerId: "provider-openai",
					providerName: "OpenAI Primary",
					providerKind: "openai",
					models: ["gpt-4o", "gpt-4o-mini"],
				},
				{
					providerId: "provider-anthropic",
					providerName: "Anthropic Primary",
					providerKind: "anthropic",
					models: ["claude-3-5-sonnet-latest"],
				},
			],
		});
	});

	it("lists the combos with provider names, models and priorities", async () => {
		const wrapper = mountView();
		await flushPromises();

		const text = wrapper.text();
		expect(text).toContain("Production fallback");
		expect(text).toContain("OpenAI Primary");
		expect(text).toContain("gpt-4o");
		expect(text).toContain("Anthropic Primary");
		expect(text).toContain("claude-3-5-sonnet-latest");
		expect(wrapper.findAll('[data-testid="combo-card"]')).toHaveLength(1);
		expect(text).toContain("rook/coding");
		expect(mockGetComboAliases).toHaveBeenCalledWith("combo-1");
	});

	it("translates step-level validation errors in English", async () => {
		const wrapper = mountView();
		await flushPromises();
		await wrapper.find('[data-testid="combo-edit-combo-1"]').trigger("click");
		await flushPromises();
		await wrapper.find('[data-testid="combo-step-provider-0"]').setValue("");
		await wrapper.find('[data-testid="combo-step-model-0"]').setValue("");
		await wrapper.find('[data-testid="combo-step-priority-1"]').setValue("1");
		await wrapper.find('[data-testid="combo-form"]').trigger("submit");
		await flushPromises();
		expect(wrapper.text()).toContain(en.combos.errors.step_provider_required);
		expect(wrapper.text()).toContain(en.combos.errors.step_priority_duplicate);
		await wrapper
			.find('[data-testid="combo-step-provider-0"]')
			.setValue("provider-openai");
		await wrapper.find('[data-testid="combo-step-model-0"]').setValue("");
		await wrapper.find('[data-testid="combo-step-priority-1"]').setValue("2");
		await wrapper.find('[data-testid="combo-form"]').trigger("submit");
		await flushPromises();
		expect(wrapper.text()).toContain(en.combos.errors.step_model_required);
	});

	it("shows the empty state with a create action when there are no combos", async () => {
		mockGetCombos.mockResolvedValueOnce({ combos: [] });
		const wrapper = mountView();
		await flushPromises();

		expect(wrapper.text()).toContain(en.combos.empty);
		expect(wrapper.find('[data-testid="combos-create"]').exists()).toBe(true);
	});

	it("associates the combo alias input with its label", async () => {
		const wrapper = mountView();
		await flushPromises();

		const input = wrapper.get('[data-testid="combo-alias-input-combo-1"]');
		expect(input.attributes("id")).toBe("combo-alias-input-combo-1");
		expect(
			wrapper.find('label[for="combo-alias-input-combo-1"]').exists(),
		).toBe(true);
	});

	it("adds an alias and validates its rook prefix and format locally", async () => {
		const wrapper = mountView();
		await flushPromises();

		await wrapper
			.find('[data-testid="combo-alias-input-combo-1"]')
			.setValue("bad name");
		await wrapper
			.find('[data-testid="combo-alias-form-combo-1"]')
			.trigger("submit");
		await flushPromises();

		expect(mockCreateComboAlias).not.toHaveBeenCalled();
		expect(wrapper.text()).toContain(
			en.combos.aliasErrors.alias_prefix_required,
		);

		await wrapper
			.find('[data-testid="combo-alias-input-combo-1"]')
			.setValue("  rook/reasoning  ");
		await wrapper
			.find('[data-testid="combo-alias-form-combo-1"]')
			.trigger("submit");
		await flushPromises();

		expect(mockCreateComboAlias).toHaveBeenCalledWith(
			"combo-1",
			"rook/reasoning",
		);
		expect(wrapper.text()).toContain("rook/reasoning");
	});

	it("deletes an alias only after confirmation", async () => {
		vi.stubGlobal(
			"confirm",
			vi.fn(() => true),
		);
		const wrapper = mountView();
		await flushPromises();

		await wrapper
			.find('[data-testid="combo-alias-delete-combo-1-rook-coding"]')
			.trigger("click");
		await flushPromises();

		expect(globalThis.confirm).toHaveBeenCalledWith(
			en.combos.aliasDeleteConfirm.replace("{alias}", "rook/coding"),
		);
		expect(mockDeleteComboAlias).toHaveBeenCalledWith("combo-1", "rook/coding");
	});

	it("keeps an alias when the user cancels its deletion", async () => {
		vi.stubGlobal(
			"confirm",
			vi.fn(() => false),
		);
		const wrapper = mountView();
		await flushPromises();

		await wrapper
			.find('[data-testid="combo-alias-delete-combo-1-rook-coding"]')
			.trigger("click");
		await flushPromises();

		expect(mockDeleteComboAlias).not.toHaveBeenCalled();
		expect(wrapper.text()).toContain("rook/coding");
	});

	it("shows the backend error when alias creation fails", async () => {
		mockCreateComboAlias.mockRejectedValueOnce(
			new Error("API Error 409: alias already exists"),
		);
		const wrapper = mountView();
		await flushPromises();

		await wrapper
			.find('[data-testid="combo-alias-input-combo-1"]')
			.setValue("rook/existing");
		await wrapper
			.find('[data-testid="combo-alias-form-combo-1"]')
			.trigger("submit");
		await flushPromises();

		expect(wrapper.text()).toContain("API Error 409: alias already exists");
	});

	it("creates a combo from the inline form", async () => {
		const wrapper = mountView();
		await flushPromises();

		await wrapper.find('[data-testid="combos-create"]').trigger("click");
		await wrapper.find('[data-testid="combo-name"]').setValue("Fast responses");
		await wrapper
			.find('[data-testid="combo-step-provider-0"]')
			.setValue("provider-anthropic");
		await flushPromises();

		const modelSelect = wrapper.find('[data-testid="combo-step-model-0"]')
			.element as HTMLSelectElement;
		expect(modelSelect.value).toBe("claude-3-5-sonnet-latest");

		await wrapper.find('[data-testid="combo-form"]').trigger("submit");
		await flushPromises();

		expect(mockCreateCombo).toHaveBeenCalledWith({
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
		});
		expect(wrapper.find('[data-testid="combo-form"]').exists()).toBe(false);
	});

	it("adds and removes steps while respecting the ten-step limit", async () => {
		const wrapper = mountView();
		await flushPromises();

		await wrapper.find('[data-testid="combos-create"]').trigger("click");

		for (let index = 0; index < 9; index += 1) {
			await wrapper.find('[data-testid="combo-step-add"]').trigger("click");
		}

		expect(
			wrapper.findAll('[data-testid^="combo-step-provider-"]'),
		).toHaveLength(10);
		expect(
			wrapper.find('[data-testid="combo-step-add"]').attributes("disabled"),
		).toBeDefined();

		await wrapper.find('[data-testid="combo-step-remove-9"]').trigger("click");
		expect(
			wrapper.findAll('[data-testid^="combo-step-provider-"]'),
		).toHaveLength(9);
	});

	it("blocks the submission and explains why when the name is empty", async () => {
		const wrapper = mountView();
		await flushPromises();

		await wrapper.find('[data-testid="combos-create"]').trigger("click");
		await wrapper.find('[data-testid="combo-form"]').trigger("submit");
		await flushPromises();

		expect(mockCreateCombo).not.toHaveBeenCalled();
		expect(wrapper.text()).toContain(en.combos.errors.name_required);
		expect(wrapper.find('[data-testid="combo-form"]').exists()).toBe(true);
	});

	it("shows the backend validation message when the save fails", async () => {
		mockCreateCombo.mockRejectedValueOnce(
			new Error("API Error 400: combo name cannot be empty"),
		);
		const wrapper = mountView();
		await flushPromises();

		await wrapper.find('[data-testid="combos-create"]').trigger("click");
		await wrapper.find('[data-testid="combo-name"]').setValue("Valid name");
		await wrapper
			.find('[data-testid="combo-step-provider-0"]')
			.setValue("provider-openai");
		await wrapper.find('[data-testid="combo-form"]').trigger("submit");
		await flushPromises();

		expect(wrapper.text()).toContain(
			"API Error 400: combo name cannot be empty",
		);
		expect(wrapper.find('[data-testid="combo-form"]').exists()).toBe(true);
	});

	it("loads an existing combo into the form when editing", async () => {
		const wrapper = mountView();
		await flushPromises();

		await wrapper.find('[data-testid="combo-edit-combo-1"]').trigger("click");
		await flushPromises();

		const nameInput = wrapper.find('[data-testid="combo-name"]')
			.element as HTMLInputElement;
		expect(nameInput.value).toBe("Production fallback");
		expect(
			wrapper.findAll('[data-testid^="combo-step-provider-"]'),
		).toHaveLength(2);

		await wrapper.find('[data-testid="combo-name"]').setValue("Renamed");
		await wrapper.find('[data-testid="combo-form"]').trigger("submit");
		await flushPromises();

		expect(mockUpdateCombo).toHaveBeenCalledWith(
			"combo-1",
			expect.objectContaining({ name: "Renamed" }),
		);
	});

	it("deletes a combo only after the user confirms", async () => {
		const confirmMock = vi.fn(() => true);
		vi.stubGlobal("confirm", confirmMock);
		const wrapper = mountView();
		await flushPromises();

		await wrapper.find('[data-testid="combo-delete-combo-1"]').trigger("click");
		await flushPromises();

		expect(confirmMock).toHaveBeenCalledWith(
			en.combos.deleteConfirm.replace("{name}", "Production fallback"),
		);
		expect(mockDeleteCombo).toHaveBeenCalledWith("combo-1");
	});

	it("keeps the combo when the user cancels the confirmation", async () => {
		vi.stubGlobal(
			"confirm",
			vi.fn(() => false),
		);
		const wrapper = mountView();
		await flushPromises();

		await wrapper.find('[data-testid="combo-delete-combo-1"]').trigger("click");
		await flushPromises();

		expect(mockDeleteCombo).not.toHaveBeenCalled();
		expect(wrapper.findAll('[data-testid="combo-card"]')).toHaveLength(1);
	});

	it("shows a retryable error when the list cannot be loaded", async () => {
		mockGetCombos.mockRejectedValueOnce(
			new Error("API Error 503: unavailable"),
		);
		const wrapper = mountView();
		await flushPromises();

		expect(wrapper.text()).toContain("API Error 503: unavailable");

		mockGetCombos.mockResolvedValueOnce({ combos: [combo()] });
		await wrapper.find('[data-testid="combos-retry"]').trigger("click");
		await flushPromises();

		expect(mockGetCombos).toHaveBeenCalledTimes(2);
		expect(wrapper.text()).toContain("Production fallback");
	});

	it("keeps the /combos route in the main navigation", () => {
		const urls = navigationConfig.main.map((item) => item.url);

		expect(urls).toContain("/combos");
	});
});
