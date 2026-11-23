/**
 * ConnectionListItem — one row of the provider connection list.
 *
 * The row is presentational: it shows the connection status, the OAuth
 * state, the configured models and the available actions. The parent
 * view owns every state change, so the row only emits events.
 *
 * These tests pin the observable contract: the status colour, the
 * conditional rows (latency, OAuth, error), the truncated model list and
 * the emitted events.
 */
import { mount } from "@vue/test-utils";
import { describe, expect, it } from "vitest";
import { createI18n } from "vue-i18n";
import type { ProviderConnectionResponse } from "@/lib/api";
import en from "@/locales/en.json";
import ConnectionListItem from "./ConnectionListItem.vue";

const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });

function makeConnection(
	overrides: Partial<ProviderConnectionResponse> = {},
): ProviderConnectionResponse {
	return {
		id: "conn-1",
		providerKind: "openai",
		providerRuntimeId: "openai",
		authType: "apiKey",
		name: "Production",
		priority: 0,
		isActive: true,
		config: {},
		testStatus: { status: "active", latencyMs: 120 },
		createdAt: "2026-01-01T00:00:00Z",
		updatedAt: "2026-01-01T00:00:00Z",
		...overrides,
	} as ProviderConnectionResponse;
}

function mountRow(
	connection: ProviderConnectionResponse,
	props: Record<string, unknown> = {},
) {
	return mount(ConnectionListItem, {
		props: { connection, ...props },
		global: { plugins: [i18n] },
	});
}

describe("ConnectionListItem status", () => {
	it("renders the connection name and its test status", () => {
		const wrapper = mountRow(makeConnection());

		expect(wrapper.text()).toContain("Production");
		expect(wrapper.text()).toContain("active");
	});

	it("colours an active connection green", () => {
		const wrapper = mountRow(makeConnection());

		expect(wrapper.find(".text-green-500").exists()).toBe(true);
	});

	it("colours an unhealthy connection as destructive", () => {
		const wrapper = mountRow(
			makeConnection({
				testStatus: { status: "unhealthy", latencyMs: 10 },
			} as Partial<ProviderConnectionResponse>),
		);

		expect(wrapper.find(".text-destructive").exists()).toBe(true);
	});

	it("colours an untested connection yellow", () => {
		const wrapper = mountRow(
			makeConnection({
				testStatus: { status: "unknown" },
			} as Partial<ProviderConnectionResponse>),
		);

		expect(wrapper.find(".text-yellow-500").exists()).toBe(true);
	});

	it("marks a disabled connection with the enabled badge", () => {
		const wrapper = mountRow(makeConnection({ isActive: false }));

		expect(wrapper.text()).toContain("Enabled");
	});

	it("shows the latency only when it is known", () => {
		const withLatency = mountRow(makeConnection());
		expect(withLatency.text()).toContain("120ms");

		const withoutLatency = mountRow(
			makeConnection({
				testStatus: { status: "active" },
			} as Partial<ProviderConnectionResponse>),
		);
		expect(withoutLatency.text()).not.toContain("ms");
	});

	it("shows the test error message", () => {
		const wrapper = mountRow(
			makeConnection({
				testStatus: { status: "unhealthy", error: "invalid api key" },
			} as Partial<ProviderConnectionResponse>),
		);

		expect(wrapper.text()).toContain("invalid api key");
	});
});

describe("ConnectionListItem oauth state", () => {
	it("shows the OAuth status and its expiry", () => {
		const wrapper = mountRow(
			makeConnection({
				authType: "oauth",
				oauthStatus: "connected",
				oauthExpiresAt: 1767225600,
			} as Partial<ProviderConnectionResponse>),
		);

		expect(wrapper.text()).toContain("OAuth connected");
		expect(wrapper.text()).toContain("Token expires");
	});

	it("warns when the OAuth session needs a reauthorization", () => {
		const wrapper = mountRow(
			makeConnection({
				authType: "oauth",
				oauthStatus: "reauthentication_required",
			} as Partial<ProviderConnectionResponse>),
		);

		expect(wrapper.text()).toContain("Reauthorization required");
	});
});

describe("ConnectionListItem models", () => {
	it("lists at most three models and reports the overflow", () => {
		const wrapper = mountRow(makeConnection(), {
			models: ["gpt-5", "gpt-4", "o3", "o4-mini"],
		});

		expect(wrapper.text()).toContain("gpt-5");
		expect(wrapper.text()).toContain("o3");
		expect(wrapper.text()).not.toContain("o4-mini");
		expect(wrapper.text()).toContain("+1");
	});

	it("shows no overflow badge when every model fits", () => {
		const wrapper = mountRow(makeConnection(), { models: ["gpt-5", "gpt-4"] });

		expect(wrapper.text()).not.toContain("+");
	});
});

describe("ConnectionListItem actions", () => {
	it("emits test when the test button is clicked", async () => {
		const wrapper = mountRow(makeConnection());

		await wrapper
			.get('[data-testid="connection-test-conn-1"]')
			.trigger("click");

		expect(wrapper.emitted("test")).toEqual([["conn-1"]]);
	});

	it("emits refresh when the OAuth refresh button is clicked", async () => {
		const wrapper = mountRow(
			makeConnection({
				authType: "oauth",
				oauthStatus: "connected",
			} as Partial<ProviderConnectionResponse>),
		);

		await wrapper
			.get('[data-testid="connection-oauth-refresh-conn-1"]')
			.trigger("click");

		expect(wrapper.emitted("refresh")).toEqual([["conn-1"]]);
	});

	it("emits edit when the edit button is clicked", async () => {
		const wrapper = mountRow(makeConnection());

		await wrapper
			.get('[data-testid="connection-edit-conn-1"]')
			.trigger("click");

		expect(wrapper.emitted("edit")).toEqual([["conn-1"]]);
	});

	it("emits delete when the delete button is clicked", async () => {
		const wrapper = mountRow(makeConnection());

		await wrapper
			.get('[data-testid="connection-delete-conn-1"]')
			.trigger("click");

		expect(wrapper.emitted("delete")).toEqual([["conn-1"]]);
	});

	it("emits toggle with the new state when the switch is flipped", async () => {
		const wrapper = mountRow(makeConnection());

		await wrapper
			.get('[data-testid="connection-toggle-conn-1"]')
			.trigger("click");

		expect(wrapper.emitted("toggle")).toEqual([
			[{ id: "conn-1", enabled: false }],
		]);
	});

	it("disables the actions while the row is busy", () => {
		const wrapper = mountRow(makeConnection(), { busy: true, testing: true });

		expect(
			wrapper
				.get('[data-testid="connection-test-conn-1"]')
				.attributes("disabled"),
		).toBeDefined();
		expect(
			wrapper
				.get('[data-testid="connection-edit-conn-1"]')
				.attributes("disabled"),
		).toBeDefined();
	});
});
