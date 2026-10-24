import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h } from "vue";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import ModelAliasesView from "./ModelAliasesView.vue";

const getModelAliases = vi.fn();
const createModelAlias = vi.fn();
const deleteModelAlias = vi.fn();
vi.mock("@/lib/api", () => ({
	useApi: () => ({ getModelAliases, createModelAlias, deleteModelAlias }),
}));
vi.mock("@/components/ui/button", () => ({
	Button: defineComponent({
		props: { type: String, disabled: Boolean, dataTestid: String },
		emits: ["click"],
		setup(props, { slots, emit }) {
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

const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });

describe("ModelAliasesView", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		vi.unstubAllGlobals();
		getModelAliases.mockResolvedValue([
			{
				alias: "chat-fast",
				canonical: "gpt-4o-mini",
				providerId: null,
				createdAt: "2026-10-24T00:00:00Z",
			},
		]);
		createModelAlias.mockResolvedValue(undefined);
		deleteModelAlias.mockResolvedValue(undefined);
	});
	it("lists model aliases and makes the flow distinct from combo aliases", async () => {
		const wrapper = mount(ModelAliasesView, { global: { plugins: [i18n] } });
		await flushPromises();
		expect(wrapper.text()).toContain("chat-fast");
		expect(wrapper.text()).toContain("gpt-4o-mini");
		expect(wrapper.text()).not.toContain("rook/coding");
	});
	it("rejects reserved rook aliases before calling the API", async () => {
		const wrapper = mount(ModelAliasesView, { global: { plugins: [i18n] } });
		await flushPromises();
		await wrapper
			.find('[data-testid="model-alias-input"]')
			.setValue(" ROOK/coding ");
		await wrapper
			.find('[data-testid="model-alias-canonical"]')
			.setValue("gpt-4o");
		await wrapper.find('[data-testid="model-alias-form"]').trigger("submit");
		await flushPromises();
		expect(createModelAlias).not.toHaveBeenCalled();
		expect(wrapper.text()).toContain(
			en.modelAliases.errors.reserved_combo_prefix,
		);
	});
	it("creates regular model aliases and reloads the list", async () => {
		const wrapper = mount(ModelAliasesView, { global: { plugins: [i18n] } });
		await flushPromises();
		await wrapper
			.find('[data-testid="model-alias-input"]')
			.setValue("chat-balanced");
		await wrapper
			.find('[data-testid="model-alias-canonical"]')
			.setValue("gpt-4o");
		await wrapper.find('[data-testid="model-alias-form"]').trigger("submit");
		await flushPromises();
		expect(createModelAlias).toHaveBeenCalledWith({
			alias: "chat-balanced",
			canonical: "gpt-4o",
			providerId: null,
		});
		expect(getModelAliases).toHaveBeenCalledTimes(2);
	});
	it("deletes an alias after confirmation", async () => {
		vi.stubGlobal(
			"confirm",
			vi.fn(() => true),
		);
		const wrapper = mount(ModelAliasesView, { global: { plugins: [i18n] } });
		await flushPromises();
		await wrapper
			.find('[data-testid="model-alias-delete-chat-fast"]')
			.trigger("click");
		await flushPromises();
		expect(deleteModelAlias).toHaveBeenCalledWith("chat-fast");
	});
});
