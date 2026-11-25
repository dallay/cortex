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
import type { CodexAccountsResponse } from "@/lib/api";
import en from "@/locales/en.json";
import es from "@/locales/es.json";
import ProviderCatalogCard from "./ProviderCatalogCard.vue";

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

async function mountCard(
	item: ProviderCatalogItem,
	codexAccounts?: CodexAccountsResponse | null,
	locale: "en" | "es" = "en",
	codexAccountsLoading = false,
) {
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
		props: { item, codexAccounts, codexAccountsLoading },
		global: {
			plugins: [
				createI18n({ legacy: false, locale, messages: { en, es } }),
				router,
			],
		},
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

	it("shows Codex account totals separately and selects the localized account action", async () => {
		const zero = await mountCard(
			makeItem({
				kind: "openai-codex",
				connectionCount: 0,
				hasActiveConnections: false,
			}),
			{ enabled: true, accounts: [], attempt: null },
		);
		expect(zero.wrapper.getComponent(Badge).text()).toBe("Not linked");
		expect(zero.wrapper.text()).toContain("No linked accounts");
		expect(zero.wrapper.text()).not.toContain("0 connections");
		expect(
			zero.wrapper.get('[data-testid="provider-card-add-openai-codex"]').text(),
		).toContain("Link account");

		const one = await mountCard(
			makeItem({
				kind: "openai-codex",
				connectionCount: 0,
				hasActiveConnections: false,
			}),
			{ enabled: true, accounts: [{ id: "one" }], attempt: null },
		);
		expect(one.wrapper.getComponent(Badge).text()).toBe("Linked");
		expect(one.wrapper.text()).toContain("1 linked account");
		expect(one.wrapper.text()).not.toContain("1 account");
		expect(one.wrapper.text()).not.toContain("Not configured");
		expect(
			one.wrapper.get('[data-testid="provider-card-add-openai-codex"]').text(),
		).toContain("Manage accounts");

		const several = await mountCard(makeItem({ kind: "openai-codex" }), {
			enabled: true,
			accounts: [{ id: "one" }, { id: "two" }, { id: "three" }],
			attempt: null,
		});
		expect(several.wrapper.getComponent(Badge).text()).toBe("Linked");
		expect(several.wrapper.text()).toContain("3 linked accounts");
		expect(
			several.wrapper
				.get('[data-testid="provider-card-add-openai-codex"]')
				.text(),
		).toContain("Manage accounts");
	});

	it("shows a checking state instead of unavailable while Codex accounts are loading", async () => {
		const { wrapper } = await mountCard(
			makeItem({ kind: "openai-codex", connectionCount: 7 }),
			null,
			"en",
			true,
		);

		expect(wrapper.getComponent(Badge).text()).toBe("Checking linked accounts");
		expect(wrapper.text()).toContain("Checking linked accounts");
		expect(wrapper.text()).not.toContain("Unavailable");
		expect(wrapper.text()).not.toContain("7 connections");

		const checkingSpanish = await mountCard(
			makeItem({ kind: "openai-codex" }),
			null,
			"es",
			true,
		);
		expect(checkingSpanish.wrapper.getComponent(Badge).text()).toBe(
			"Comprobando cuentas vinculadas",
		);
	});

	it("does not present disabled or unavailable Codex accounts as zero", async () => {
		const disabled = await mountCard(makeItem({ kind: "openai-codex" }), {
			enabled: false,
			accounts: [],
			attempt: null,
		});
		expect(disabled.wrapper.getComponent(Badge).text()).toBe("Unavailable");
		expect(disabled.wrapper.text()).toContain(
			"Codex account linking is unavailable",
		);
		expect(disabled.wrapper.text()).not.toContain("0 accounts");
		expect(disabled.wrapper.text()).not.toContain("2 connections");
		expect(
			disabled.wrapper
				.get('[data-testid="provider-card-add-openai-codex"]')
				.text(),
		).toContain("Link account");

		const unavailable = await mountCard(
			makeItem({ kind: "openai-codex" }),
			null,
		);
		expect(unavailable.wrapper.getComponent(Badge).text()).toBe("Unavailable");
		expect(unavailable.wrapper.text()).not.toContain("0 linked accounts");
	});

	it("localizes Codex account badges and actions in Spanish", async () => {
		const zero = await mountCard(
			makeItem({ kind: "openai-codex" }),
			{ enabled: true, accounts: [], attempt: null },
			"es",
		);
		expect(zero.wrapper.getComponent(Badge).text()).toBe("Sin vincular");
		expect(zero.wrapper.text()).toContain("Sin cuentas vinculadas");
		expect(
			zero.wrapper.get('[data-testid="provider-card-add-openai-codex"]').text(),
		).toContain("Vincular cuenta");

		const one = await mountCard(
			makeItem({ kind: "openai-codex" }),
			{ enabled: true, accounts: [{ id: "one" }], attempt: null },
			"es",
		);
		expect(one.wrapper.getComponent(Badge).text()).toBe("Vinculada");
		expect(
			one.wrapper.get('[data-testid="provider-card-add-openai-codex"]').text(),
		).toContain("Administrar cuentas");
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
