import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";
import type { ProviderConnectionResponse } from "@/lib/api";
import en from "@/locales/en.json";
import CompatibleConnectionsSection from "./CompatibleConnectionsSection.vue";

const mockProviders = vi.hoisted(() => [] as ProviderConnectionResponse[]);
const mockUpdate = vi.hoisted(() => vi.fn());
const mockTest = vi.hoisted(() => vi.fn());
const mockFetch = vi.hoisted(() => vi.fn());

vi.mock("@/composables/useProviders", () => ({
	useProviders: () => ({
		providers: { value: mockProviders },
		update: (...args: unknown[]) => mockUpdate(...args),
		test: (...args: unknown[]) => mockTest(...args),
		fetch: () => mockFetch(),
	}),
}));

const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });

function mountSection() {
	return mount(CompatibleConnectionsSection, {
		global: { plugins: [i18n] },
	});
}

function makeConnection(overrides = {}): ProviderConnectionResponse {
	return {
		id: "conn-1",
		providerKind: "openai-compatible",
		name: "My Endpoint",
		isActive: true,
		updatedAt: "2026-01-01T00:00:00Z",
		...overrides,
	} as unknown as ProviderConnectionResponse;
}

describe("CompatibleConnectionsSection", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockProviders.length = 0;
		mockUpdate.mockResolvedValue(undefined);
		mockTest.mockResolvedValue(undefined);
		mockFetch.mockResolvedValue(undefined);
	});

	it("shows empty state when there are no compatible connections", () => {
		const wrapper = mountSection();
		expect(wrapper.text()).toContain("No compatible connections configured");
		expect(
			wrapper.findAll('[data-testid="compatible-connection-card"]').length,
		).toBe(0);
	});

	it("renders connection cards for compatible providers", () => {
		mockProviders.push(
			makeConnection({
				id: "conn-1",
				providerKind: "openai-compatible",
				name: "OpenAI EP",
			}),
			makeConnection({
				id: "conn-2",
				providerKind: "anthropic-compatible",
				name: "Anthropic EP",
			}),
		);
		const wrapper = mountSection();
		const cards = wrapper.findAll('[data-testid="compatible-connection-card"]');
		expect(cards.length).toBe(2);
		expect(wrapper.text()).toContain("OpenAI EP");
		expect(wrapper.text()).toContain("Anthropic EP");
	});

	it("filters out non-compatible providers", () => {
		mockProviders.push(
			makeConnection({
				id: "conn-openai",
				providerKind: "openai",
				name: "Standard OpenAI",
			}),
			makeConnection({
				id: "conn-compatible",
				providerKind: "openai-compatible",
				name: "My Compatible",
			}),
		);
		const wrapper = mountSection();
		const cards = wrapper.findAll('[data-testid="compatible-connection-card"]');
		expect(cards.length).toBe(1);
		expect(wrapper.text()).toContain("My Compatible");
		expect(wrapper.text()).not.toContain("Standard OpenAI");
	});

	it("emits add event with correct kind when Add OpenAI Compatible is clicked", async () => {
		const wrapper = mountSection();
		const button = wrapper.find('[data-testid="add-openai-compatible"]');
		await button.trigger("click");
		const emitted = wrapper.emitted("add");
		expect(emitted).toBeDefined();
		expect(emitted![0]).toEqual(["openai-compatible"]);
	});

	it("emits add event with correct kind when Add Anthropic Compatible is clicked", async () => {
		const wrapper = mountSection();
		const button = wrapper.find('[data-testid="add-anthropic-compatible"]');
		await button.trigger("click");
		const emitted = wrapper.emitted("add");
		expect(emitted).toBeDefined();
		expect(emitted![0]).toEqual(["anthropic-compatible"]);
	});

	it("shows protocol badge for OpenAI Compatible", () => {
		mockProviders.push(makeConnection({ providerKind: "openai-compatible" }));
		const wrapper = mountSection();
		expect(wrapper.text()).toContain("OpenAI");
	});

	it("shows protocol badge for Anthropic Compatible", () => {
		mockProviders.push(
			makeConnection({ providerKind: "anthropic-compatible" }),
		);
		const wrapper = mountSection();
		expect(wrapper.text()).toContain("Anthropic");
	});

	it("shows 'Connected' status for active connections", () => {
		mockProviders.push(makeConnection({ isActive: true }));
		const wrapper = mountSection();
		expect(wrapper.text()).toContain("Connected");
	});

	it("shows 'Disabled' status for inactive connections", () => {
		mockProviders.push(makeConnection({ isActive: false }));
		const wrapper = mountSection();
		expect(wrapper.text()).toContain("Disabled");
	});

	it("shows green dot for active connections", () => {
		mockProviders.push(makeConnection({ isActive: true }));
		const wrapper = mountSection();
		const greenDot = wrapper.find(".bg-green-500");
		expect(greenDot.exists()).toBe(true);
	});

	it("does not show green dot for inactive connections", () => {
		mockProviders.push(makeConnection({ isActive: false }));
		const wrapper = mountSection();
		const greenDot = wrapper.find(".bg-green-500");
		expect(greenDot.exists()).toBe(false);
	});

	it("calls test and fetch when Test button is clicked", async () => {
		mockProviders.push(makeConnection({ id: "conn-1" }));
		const wrapper = mountSection();
		// Find the Test button by its text content inside the card
		const card = wrapper.find('[data-testid="compatible-connection-card"]');
		const buttons = card.findAll("button");
		// The last button in the card is the Test button (Switch is not a button, Test is)
		const testButton = buttons[buttons.length - 1];
		await testButton.trigger("click");
		await flushPromises();
		expect(mockTest).toHaveBeenCalledWith("conn-1");
		expect(mockFetch).toHaveBeenCalled();
	});

	it("disables test button while testing", async () => {
		mockTest.mockImplementation(
			() => new Promise((resolve) => setTimeout(resolve, 100)),
		);
		mockProviders.push(makeConnection({ id: "conn-1" }));
		const wrapper = mountSection();
		const card = wrapper.find('[data-testid="compatible-connection-card"]');
		const buttons = card.findAll("button");
		const testButton = buttons[buttons.length - 1];
		await testButton.trigger("click");
		// Button should be disabled while testing
		expect(testButton.attributes("disabled")).toBeDefined();
		await flushPromises();
	});

	it("toggles connection enabled state via update", async () => {
		mockProviders.push(makeConnection({ id: "conn-1", isActive: true }));
		const wrapper = mountSection();
		const switchComponent = wrapper.findComponent({ name: "Switch" });
		await switchComponent.vm.$emit("update:checked", false);
		await flushPromises();
		expect(mockUpdate).toHaveBeenCalledWith("conn-1", {
			expectedUpdatedAt: "2026-01-01T00:00:00Z",
			isActive: false,
		});
	});

	it("shows section header with category name and description", () => {
		const wrapper = mountSection();
		expect(wrapper.text()).toContain("Compatible Providers");
	});

	it("displays untitled when connection has no name", () => {
		mockProviders.push(makeConnection({ name: "" }));
		const wrapper = mountSection();
		expect(wrapper.text()).toContain("Untitled");
	});
});
