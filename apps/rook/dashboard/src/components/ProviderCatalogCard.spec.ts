/**
 * ProviderCatalogCard contract tests.
 *
 * The card is presentational: it renders one catalog entry, links to the
 * provider detail page, and emits `add` when the user asks for the
 * "add provider" flow. These tests pin the observable contract:
 *
 *   - the display name and description come from the i18n keys, and fall
 *     back to the raw label when a key is missing;
 *   - the status badge reflects `hasActiveConnections`;
 *   - the card links to `/providers/<kind>`;
 *   - the connection count is localized (singular vs plural);
 *   - the "add" button emits the kind and does not navigate.
 */
import { flushPromises, mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { createI18n } from "vue-i18n";
import { createMemoryHistory, createRouter } from "vue-router";
import { Badge } from "@/components/ui/badge";
import type { ProviderCatalogItem } from "@/composables/useProviderCatalog";
import en from "@/locales/en.json";
import ProviderCatalogCard from "./ProviderCatalogCard.vue";

const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });

function makeItem(
	overrides: Partial<ProviderCatalogItem> = {},
): ProviderCatalogItem {
	return {
		kind: "openai",
		category: "api-key",
		displayNameKey: "providers.kind.openai.name",
		descriptionKey: "providers.kind.openai.description",
		defaultBaseUrl: "https://api.openai.com/v1",
		authTypes: ["apikey"],
		defaultModels: [],
		label: "OpenAI",
		iconKey: "openai",
		connectionCount: 2,
		hasActiveConnections: true,
		configuredModels: [],
		...overrides,
	};
}

async function mountCard(item: ProviderCatalogItem) {
	const router = createRouter({
		history: createMemoryHistory(),
		routes: [
			{ path: "/", name: "home", component: { template: "<div>home</div>" } },
			{
				path: "/providers/:providerKind",
				name: "provider-details",
				component: { template: "<div>details</div>" },
			},
		],
	});
	await router.push("/");
	await router.isReady();

	const wrapper = mount(ProviderCatalogCard, {
		props: { item },
		global: { plugins: [i18n, router] },
	});

	return { wrapper, router };
}

describe("ProviderCatalogCard", () => {
	it("renders the translated display name and description", async () => {
		const { wrapper } = await mountCard(makeItem());

		const title = wrapper.get('[data-testid="provider-card-openai"] h3');
		expect(title.text()).toBe("OpenAI");
		expect(wrapper.text()).toContain(
			"OpenAI's GPT models for chat and reasoning.",
		);
	});

	it("falls back to the raw label when the i18n keys are missing", async () => {
		const { wrapper } = await mountCard(
			makeItem({
				kind: "custom-gateway",
				label: "Custom Gateway",
				displayNameKey: "",
				descriptionKey: "",
			}),
		);

		const title = wrapper.get(
			'[data-testid="provider-card-custom-gateway"] h3',
		);
		expect(title.text()).toBe("Custom Gateway");
		expect(wrapper.text().match(/Custom Gateway/g)).toHaveLength(2);
	});

	it("marks the provider as active when it has active connections", async () => {
		const { wrapper } = await mountCard(
			makeItem({ hasActiveConnections: true }),
		);

		const badge = wrapper.getComponent(Badge);
		expect(badge.text()).toBe("Active");
		expect(badge.props("variant")).toBe("default");
	});

	it("marks the provider as not configured when it has no active connections", async () => {
		const { wrapper } = await mountCard(
			makeItem({ hasActiveConnections: false }),
		);

		const badge = wrapper.getComponent(Badge);
		expect(badge.text()).toBe("Not configured");
		expect(badge.props("variant")).toBe("secondary");
	});

	it("links the card body to the provider detail route", async () => {
		const { wrapper } = await mountCard(makeItem({ kind: "groq" }));

		const link = wrapper.get('[data-testid="provider-card-link-groq"]');
		expect(link.attributes("href")).toBe("/providers/groq");
	});

	it("localizes the connection count in singular and plural form", async () => {
		const single = await mountCard(makeItem({ connectionCount: 1 }));
		expect(single.wrapper.text()).toContain("1 connection");
		expect(single.wrapper.text()).not.toContain("1 connections");

		const plural = await mountCard(makeItem({ connectionCount: 3 }));
		expect(plural.wrapper.text()).toContain("3 connections");
	});

	it("emits the provider kind when the add button is clicked", async () => {
		const { wrapper } = await mountCard(makeItem({ kind: "anthropic" }));

		await wrapper
			.get('[data-testid="provider-card-add-anthropic"]')
			.trigger("click");

		expect(wrapper.emitted("add")).toEqual([["anthropic"]]);
	});

	it("does not navigate when the add button is clicked", async () => {
		const { wrapper, router } = await mountCard(makeItem());

		await wrapper
			.get('[data-testid="provider-card-add-openai"]')
			.trigger("click");

		expect(router.currentRoute.value.path).toBe("/");
		expect(router.currentRoute.value.name).toBe("home");
	});

	it("navigates to the provider detail route when the card link is followed", async () => {
		const { wrapper, router } = await mountCard(makeItem({ kind: "ollama" }));

		await wrapper
			.get('[data-testid="provider-card-link-ollama"]')
			.trigger("click");
		await flushPromises();

		expect(router.currentRoute.value.path).toBe("/providers/ollama");
	});
});
