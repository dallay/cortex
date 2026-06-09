import { flushPromises, mount } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent, h, nextTick } from "vue";
import { createI18n } from "vue-i18n";
import type { ProviderConnectionResponse } from "@/lib/api";
import en from "@/locales/en.json";
import AddProviderDialog from "./AddProviderDialog.vue";

// ---------------------------------------------------------------------------
// useProviders mock — controllable per test
// ---------------------------------------------------------------------------

type TestResult =
	| {
			valid: true;
			status: "ok";
			latencyMs: number;
			error: null;
			warning: null;
			method: string;
	  }
	| {
			valid: true;
			status: "warning";
			latencyMs: number;
			error: null;
			warning: string;
			method: string;
	  }
	| {
			valid: true;
			status: "unknown";
			latencyMs: null;
			error: null;
			warning: string;
			method: "not_supported";
	  }
	| {
			valid: false;
			status: "unhealthy" | "expired";
			latencyMs: number | null;
			error: string;
			warning: null;
			method: string | null;
	  };
type TestFn = (payload: unknown) => Promise<TestResult | undefined>;

const mockState = {
	testCredentialsImpl: vi.fn<() => TestFn>(),
	create: vi.fn(),
	update: vi.fn(),
	remove: vi.fn(),
	fetch: vi.fn(),
	providerById: { value: new Map<string, ProviderConnectionResponse>() },
};

vi.mock("@/composables/useProviders", () => ({
	useProviders: () => ({
		create: (...args: unknown[]) => mockState.create(...args),
		update: (...args: unknown[]) => mockState.update(...args),
		remove: (...args: unknown[]) => mockState.remove(...args),
		testCredentials: (payload: unknown) => {
			const fn = mockState.testCredentialsImpl();
			return fn(payload);
		},
		fetch: (...args: unknown[]) => mockState.fetch(...args),
		providerById: mockState.providerById,
	}),
}));

// ---------------------------------------------------------------------------
// shadcn-vue / reka-ui stubs
//
// Buttons are rendered as real <button> elements so the native `disabled`
// attribute can be asserted. Other primitives are passthrough divs that
// forward data-testid from the parent. This matches the pattern in
// `ApiKeyForm.spec.ts`.
// ---------------------------------------------------------------------------

vi.mock("@/components/ui/button", () => ({
	Button: defineComponent({
		name: "Button",
		props: {
			type: String,
			variant: String,
			disabled: Boolean,
			dataTestid: String,
		},
		emits: ["click"],
		setup(props, { slots, emit, attrs }) {
			return () => {
				// dataTestid is declared as a prop and accepts the kebab-case
				// form from the parent; fall back to the attribute for legacy callers.
				const testid =
					props.dataTestid ??
					(attrs["data-testid"] as string | undefined) ??
					"mock-button";
				return h(
					"button",
					{
						type: (props.type as string) ?? "button",
						disabled: props.disabled === true,
						"data-testid": testid,
						onClick: () => emit("click"),
					},
					slots.default?.(),
				);
			};
		},
	}),
}));

vi.mock("@/components/ui/input", () => ({
	Input: defineComponent({
		name: "Input",
		props: {
			id: String,
			type: String,
			modelValue: { type: [String, Number, null], default: "" },
			placeholder: String,
			disabled: Boolean,
			min: [String, Number],
			max: [String, Number],
			required: Boolean,
		},
		emits: ["update:modelValue"],
		setup(props, { emit }) {
			return () => {
				const isNumber = props.type === "number";
				const cast = (v: string) => (isNumber && v !== "" ? Number(v) : v);
				return h("input", {
					id: props.id,
					type: (props.type as string) ?? "text",
					value: props.modelValue ?? "",
					placeholder: props.placeholder,
					disabled: props.disabled,
					min: props.min as string | number | undefined,
					max: props.max as string | number | undefined,
					"data-testid": props.id ? `input-${props.id}` : "input",
					onInput: (e: Event) =>
						emit(
							"update:modelValue",
							cast((e.target as HTMLInputElement).value),
						),
				});
			};
		},
	}),
}));

vi.mock("@/components/ui/label", () => ({
	Label: defineComponent({
		name: "Label",
		props: { for: String },
		setup(props, { slots }) {
			return () => h("label", { for: props.for }, slots.default?.());
		},
	}),
}));

vi.mock("@/components/ui/switch", () => ({
	Switch: defineComponent({
		name: "Switch",
		props: { id: String, checked: Boolean, disabled: Boolean },
		emits: ["update:checked"],
		setup(props, { emit }) {
			return () =>
				h("input", {
					id: props.id,
					type: "checkbox",
					checked: props.checked === true,
					disabled: props.disabled,
					"data-testid": props.id ? `switch-${props.id}` : "switch",
					onChange: (e: Event) =>
						emit("update:checked", (e.target as HTMLInputElement).checked),
				});
		},
	}),
}));

vi.mock("@/components/ui/toggle-group", () => ({
	ToggleGroup: defineComponent({
		name: "ToggleGroup",
		props: { modelValue: String, type: String },
		emits: ["update:modelValue"],
		setup(props, { slots, emit }) {
			return () =>
				h(
					"div",
					{ "data-testid": "mock-togglegroup", "data-value": props.modelValue },
					[
						h("input", {
							type: "hidden",
							value: props.modelValue,
							"data-testid": "mock-togglegroup-value",
						}),
						slots.default?.(),
						h(
							"button",
							{
								type: "button",
								"data-testid": "mock-togglegroup-set-oauth",
								onClick: () => emit("update:modelValue", "oauth"),
							},
							"set-oauth",
						),
					],
				);
		},
	}),
	ToggleGroupItem: defineComponent({
		name: "ToggleGroupItem",
		props: {
			value: String,
			disabled: Boolean,
			ariaLabel: String,
			dataTestid: String,
		},
		setup(props, { slots }) {
			return () =>
				h(
					"button",
					{
						type: "button",
						disabled: props.disabled === true,
						"data-testid": props.dataTestid ?? `mock-toggle-${props.value}`,
						"data-value": props.value,
						"data-disabled": props.disabled ? "true" : "false",
						"aria-label": props.ariaLabel,
					},
					slots.default?.(),
				);
		},
	}),
}));

vi.mock("@/components/ui/select", () => ({
	Select: defineComponent({
		name: "Select",
		props: { modelValue: String },
		emits: ["update:modelValue"],
		setup(props, { slots, emit }) {
			return () =>
				h("div", { "data-testid": "mock-select" }, [
					h("input", {
						type: "hidden",
						value: props.modelValue,
						"data-testid": "mock-select-value",
					}),
					slots.default?.(),
					h(
						"button",
						{
							type: "button",
							"data-testid": "mock-select-trigger",
							onClick: () => emit("update:modelValue", "gemini"),
						},
						"open",
					),
				]);
		},
	}),
	SelectTrigger: defineComponent({
		name: "SelectTrigger",
		props: { id: String, dataTestid: String },
		setup(props, { slots, attrs }) {
			return () =>
				h(
					"div",
					{
						id: props.id,
						"data-testid":
							props.dataTestid ??
							(attrs["data-testid"] as string) ??
							"select-trigger",
					},
					slots.default?.(),
				);
		},
	}),
	SelectValue: defineComponent({
		name: "SelectValue",
		setup(_props, { slots }) {
			return () => h("span", slots.default?.());
		},
	}),
	SelectContent: defineComponent({
		name: "SelectContent",
		setup(_props, { slots }) {
			return () =>
				h("div", { "data-testid": "select-content" }, slots.default?.());
		},
	}),
	SelectItem: defineComponent({
		name: "SelectItem",
		props: { value: String },
		setup(props, { slots }) {
			return () =>
				h(
					"div",
					{
						"data-testid": `select-item-${props.value}`,
						"data-value": props.value,
					},
					slots.default?.(),
				);
		},
	}),
}));

vi.mock("@/components/ui/dialog", () => ({
	Dialog: defineComponent({
		name: "Dialog",
		props: { open: Boolean },
		emits: ["update:open"],
		setup(props, { slots }) {
			return () =>
				props.open
					? h(
							"div",
							{ "data-testid": "dialog-root", "data-open": "true" },
							slots.default?.(),
						)
					: null;
		},
	}),
	DialogContent: defineComponent({
		name: "DialogContent",
		setup(_props, { slots, attrs }) {
			return () =>
				h(
					"div",
					{
						"data-testid":
							(attrs["data-testid"] as string) ?? "mock-dialog-content",
					},
					slots.default?.(),
				);
		},
	}),
	DialogHeader: defineComponent({
		name: "DialogHeader",
		setup(_props, { slots }) {
			return () =>
				h("div", { "data-testid": "mock-dialog-header" }, slots.default?.());
		},
	}),
	DialogTitle: defineComponent({
		name: "DialogTitle",
		setup(_props, { slots }) {
			return () =>
				h("div", { "data-testid": "mock-dialog-title" }, slots.default?.());
		},
	}),
	DialogDescription: defineComponent({
		name: "DialogDescription",
		setup(_props, { slots }) {
			return () =>
				h(
					"div",
					{ "data-testid": "mock-dialog-description" },
					slots.default?.(),
				);
		},
	}),
	DialogFooter: defineComponent({
		name: "DialogFooter",
		setup(_props, { slots }) {
			return () =>
				h("div", { "data-testid": "mock-dialog-footer" }, slots.default?.());
		},
	}),
}));

vi.mock("@/components/ui/alert-dialog", () => ({
	AlertDialog: defineComponent({
		name: "AlertDialog",
		props: { open: Boolean },
		emits: ["update:open"],
		setup(props, { slots }) {
			return () =>
				props.open
					? h(
							"div",
							{ "data-testid": "alertdialog-root", "data-open": "true" },
							slots.default?.(),
						)
					: null;
		},
	}),
	AlertDialogContent: defineComponent({
		name: "AlertDialogContent",
		setup(_props, { slots, attrs }) {
			return () =>
				h(
					"div",
					{
						"data-testid":
							(attrs["data-testid"] as string) ?? "mock-alertdialog-content",
					},
					slots.default?.(),
				);
		},
	}),
	AlertDialogHeader: defineComponent({
		name: "AlertDialogHeader",
		setup(_props, { slots }) {
			return () =>
				h(
					"div",
					{ "data-testid": "mock-alertdialog-header" },
					slots.default?.(),
				);
		},
	}),
	AlertDialogTitle: defineComponent({
		name: "AlertDialogTitle",
		setup(_props, { slots }) {
			return () =>
				h(
					"div",
					{ "data-testid": "mock-alertdialog-title" },
					slots.default?.(),
				);
		},
	}),
	AlertDialogDescription: defineComponent({
		name: "AlertDialogDescription",
		setup(_props, { slots }) {
			return () =>
				h(
					"div",
					{ "data-testid": "mock-alertdialog-description" },
					slots.default?.(),
				);
		},
	}),
	AlertDialogFooter: defineComponent({
		name: "AlertDialogFooter",
		setup(_props, { slots }) {
			return () =>
				h(
					"div",
					{ "data-testid": "mock-alertdialog-footer" },
					slots.default?.(),
				);
		},
	}),
	AlertDialogAction: defineComponent({
		name: "AlertDialogAction",
		setup(_props, { slots, emit, attrs }) {
			return () =>
				h(
					"button",
					{
						type: "button",
						"data-testid":
							(attrs["data-testid"] as string) ?? "mock-alertdialog-action",
						onClick: () => emit("click"),
					},
					slots.default?.(),
				);
		},
	}),
	AlertDialogCancel: defineComponent({
		name: "AlertDialogCancel",
		setup(_props, { slots, attrs }) {
			return () =>
				h(
					"button",
					{
						type: "button",
						"data-testid":
							(attrs["data-testid"] as string) ?? "mock-alertdialog-cancel",
					},
					slots.default?.(),
				);
		},
	}),
}));

vi.mock("@/components/PasswordInput.vue", () => ({
	default: defineComponent({
		name: "PasswordInput",
		props: {
			id: String,
			modelValue: { type: String, default: "" },
			placeholder: String,
			disabled: Boolean,
		},
		emits: ["update:modelValue"],
		setup(props, { emit }) {
			return () =>
				h("input", {
					id: props.id,
					type: "password",
					value: props.modelValue,
					placeholder: props.placeholder,
					disabled: props.disabled,
					"data-testid": props.id ? `input-${props.id}` : "password-input",
					onInput: (e: Event) =>
						emit("update:modelValue", (e.target as HTMLInputElement).value),
				});
		},
	}),
}));

vi.mock("@lucide/vue", () => {
	const icon = defineComponent({
		name: "Icon",
		setup: () => () => h("span", { "data-testid": "icon" }),
	});
	return {
		AlertCircle: icon,
		AlertTriangle: icon,
		CheckCircle2: icon,
		Eye: icon,
		EyeOff: icon,
		Loader2: icon,
		Plus: icon,
		Trash2: icon,
	};
});

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const i18n = createI18n({ legacy: false, locale: "en", messages: { en } });

function makeConnection(
	overrides: Partial<ProviderConnectionResponse> = {},
): ProviderConnectionResponse {
	return {
		id: "conn-1",
		providerKind: "ollama",
		providerRuntimeId: "ollama-local",
		authType: "apiKey",
		name: "Local Ollama",
		priority: 50,
		isActive: true,
		config: {
			maxConcurrent: 1,
			quotaWindowThresholds: { warning: 0.8, error: 0.95 },
			defaultModel: "llama3.1",
			baseUrl: "http://localhost:11434",
		},
		testStatus: {
			status: "active",
			lastTestAt: "2024-01-01T00:00:00Z",
			latencyMs: 42,
			error: null,
		},
		createdAt: "2024-01-01T00:00:00Z",
		updatedAt: "2024-01-01T00:00:00Z",
		...overrides,
	};
}

function mountDialog(props: Record<string, unknown> = {}) {
	return mount(AddProviderDialog, {
		props: { open: true, mode: "create", ...props },
		global: { plugins: [i18n] },
		attachTo: document.body,
	});
}

async function setInputValue(
	wrapper: ReturnType<typeof mount>,
	testid: string,
	value: string,
) {
	const input = wrapper.find<HTMLInputElement>(`[data-testid="${testid}"]`);
	expect(input.exists(), `input ${testid} should exist`).toBe(true);
	await input.setValue(value);
	await nextTick();
}

beforeEach(() => {
	mockState.testCredentialsImpl.mockReset();
	mockState.testCredentialsImpl.mockImplementation(() => async () => ({
		valid: true as const,
		status: "ok" as const,
		latencyMs: 100,
		error: null,
		warning: null,
		method: "models_list",
	}));
	mockState.create.mockReset();
	mockState.create.mockResolvedValue(makeConnection());
	mockState.update.mockReset();
	mockState.update.mockResolvedValue(makeConnection());
	mockState.remove.mockReset();
	mockState.remove.mockResolvedValue(true);
	mockState.fetch.mockReset();
	mockState.fetch.mockResolvedValue(undefined);
	mockState.providerById.value = new Map();
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("AddProviderDialog — create mode", () => {
	it("renders without a kind selector when providerKind is pre-scoped (ollama)", async () => {
		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		expect(wrapper.find('[data-testid="kind-select-trigger"]').exists()).toBe(
			false,
		);
		expect(wrapper.find('[data-testid="dialog-root"]').exists()).toBe(true);
	});

	it("renders with a kind selector when providerKind is undefined", async () => {
		const wrapper = mountDialog({ providerKind: undefined });
		await flushPromises();
		expect(wrapper.find('[data-testid="kind-select-trigger"]').exists()).toBe(
			true,
		);
	});

	it("does NOT show the delete button in create mode", async () => {
		const wrapper = mountDialog({ mode: "create" });
		await flushPromises();
		expect(wrapper.find('[data-testid="delete-button"]').exists()).toBe(false);
	});
});

describe("AddProviderDialog — edit mode", () => {
	it("shows the delete button and pre-fills the form from the cached connection", async () => {
		const conn = makeConnection({ name: "My Local Ollama", priority: 75 });
		mockState.providerById.value = new Map([[conn.id, conn]]);

		const wrapper = mountDialog({ mode: "edit", connectionId: conn.id });
		await flushPromises();

		expect(wrapper.find('[data-testid="delete-button"]').exists()).toBe(true);
		const nameInput = wrapper.find<HTMLInputElement>(
			'[data-testid="input-displayName"]',
		);
		expect(nameInput.element.value).toBe("My Local Ollama");
		const priorityInput = wrapper.find<HTMLInputElement>(
			'[data-testid="input-priority"]',
		);
		expect(priorityInput.element.value).toBe("75");
	});

	it("fetches the provider list when the connection is not in the cache", async () => {
		const conn = makeConnection();
		// First fetch call: hydrate the cache
		mockState.fetch.mockImplementationOnce(async () => {
			mockState.providerById.value = new Map([[conn.id, conn]]);
		});

		const wrapper = mountDialog({ mode: "edit", connectionId: conn.id });
		await flushPromises();

		expect(mockState.fetch).toHaveBeenCalledTimes(1);
		const nameInput = wrapper.find<HTMLInputElement>(
			'[data-testid="input-displayName"]',
		);
		expect(nameInput.element.value).toBe("Local Ollama");
	});

	it("opens a delete confirmation when the delete button is clicked", async () => {
		const conn = makeConnection();
		mockState.providerById.value = new Map([[conn.id, conn]]);

		const wrapper = mountDialog({ mode: "edit", connectionId: conn.id });
		await flushPromises();

		expect(wrapper.find('[data-testid="alertdialog-root"]').exists()).toBe(
			false,
		);

		await wrapper.find('[data-testid="delete-button"]').trigger("click");
		await nextTick();

		expect(wrapper.find('[data-testid="alertdialog-root"]').exists()).toBe(
			true,
		);
		expect(wrapper.find('[data-testid="delete-cancel"]').exists()).toBe(true);
		expect(wrapper.find('[data-testid="delete-confirm-button"]').exists()).toBe(
			true,
		);
	});

	it("calls remove and emits deleted when the confirmation is accepted", async () => {
		const conn = makeConnection();
		mockState.providerById.value = new Map([[conn.id, conn]]);

		const wrapper = mountDialog({ mode: "edit", connectionId: conn.id });
		await flushPromises();

		await wrapper.find('[data-testid="delete-button"]').trigger("click");
		await nextTick();
		await wrapper
			.find('[data-testid="delete-confirm-button"]')
			.trigger("click");
		await flushPromises();

		expect(mockState.remove).toHaveBeenCalledWith(conn.id);
		const deleted = wrapper.emitted("deleted");
		expect(deleted).toBeTruthy();
		expect(deleted!.at(-1)).toEqual([conn.id]);
	});
});

describe("AddProviderDialog — test before save", () => {
	it("disables the save button when the form has not been tested", async () => {
		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		await setInputValue(wrapper, "input-displayName", "Local Ollama");
		await setInputValue(wrapper, "input-apiKey", "sk-test");

		const saveButton = wrapper.find<HTMLButtonElement>(
			'[data-testid="save-button"]',
		);
		expect(saveButton.exists()).toBe(true);
		expect(saveButton.element.disabled).toBe(true);
	});

	it("enables the save button after a successful testCredentials call", async () => {
		mockState.testCredentialsImpl.mockImplementation(() => async () => ({
			valid: true as const,
			status: "ok" as const,
			latencyMs: 87,
			error: null,
			warning: null,
			method: "models_list",
		}));

		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		await setInputValue(wrapper, "input-displayName", "Local Ollama");
		await setInputValue(wrapper, "input-apiKey", "sk-test");

		await wrapper.find('[data-testid="test-button"]').trigger("click");
		await flushPromises();

		expect(mockState.testCredentialsImpl).toHaveBeenCalled();
		expect(wrapper.find('[data-testid="test-result"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="test-result-icon-ok"]').exists()).toBe(
      true,
    );

		const saveButton = wrapper.find<HTMLButtonElement>(
			'[data-testid="save-button"]',
		);
		expect(saveButton.element.disabled).toBe(false);
	});

	it("keeps the save button disabled after a failed testCredentials call", async () => {
		mockState.testCredentialsImpl.mockImplementation(() => async () => ({
			valid: false as const,
			status: "unhealthy" as const,
			latencyMs: 30,
			error: "auth rejected: HTTP 401",
			warning: null,
			method: "models_list",
		}));

		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		await setInputValue(wrapper, "input-displayName", "Local Ollama");
		await setInputValue(wrapper, "input-apiKey", "sk-bad");

		await wrapper.find('[data-testid="test-button"]').trigger("click");
		await flushPromises();

		expect(wrapper.find('[data-testid="test-result"]').exists()).toBe(true);
		const saveButton = wrapper.find<HTMLButtonElement>(
			'[data-testid="save-button"]',
		);
		expect(saveButton.element.disabled).toBe(true);
	});

	it("disables the test button until displayName and apiKey are filled", async () => {
		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		const testButton = wrapper.find<HTMLButtonElement>(
			'[data-testid="test-button"]',
		);
		expect(testButton.exists()).toBe(true);
		expect(testButton.element.disabled).toBe(true);
	});

	// --- new: 3-state test result block (ok / warning / invalid) ---

	it("enables save and shows a yellow alert on a warning response (HTTP 429)", async () => {
		mockState.testCredentialsImpl.mockImplementation(() => async () => ({
			valid: true as const,
			status: "warning" as const,
			latencyMs: 17,
			error: null,
			warning: "Rate limited, but credentials are valid",
			method: "chat_probe",
		}));

		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		await setInputValue(wrapper, "input-displayName", "Local Ollama");
		await setInputValue(wrapper, "input-apiKey", "sk-test");

		await wrapper.find('[data-testid="test-button"]').trigger("click");
		await flushPromises();

		// Yellow alert must be visible.
		expect(
			wrapper.find('[data-testid="test-result-icon-warning"]').exists(),
		).toBe(true);
    expect(wrapper.find('[data-testid="test-result-icon-ok"]').exists()).toBe(
      false,
    );
		expect(
			wrapper.find('[data-testid="test-result-icon-error"]').exists(),
		).toBe(false);

		// The warning text bubbles through to the message line.
		const message = wrapper.find('[data-testid="test-result-message"]');
		expect(message.exists()).toBe(true);
		expect(message.text()).toContain("Rate limited");

		// Save must be enabled even though status is "warning" — that's
		// the whole point of the new shape.
		const saveButton = wrapper.find<HTMLButtonElement>(
			'[data-testid="save-button"]',
		);
		expect(saveButton.element.disabled).toBe(false);
	});

	it("disables save and shows a red alert on an invalid response (HTTP 401)", async () => {
		mockState.testCredentialsImpl.mockImplementation(() => async () => ({
			valid: false as const,
			status: "unhealthy" as const,
			latencyMs: 30,
			error: "auth rejected: HTTP 401 — invalid key",
			warning: null,
			method: "models_list",
		}));

		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		await setInputValue(wrapper, "input-displayName", "Local Ollama");
		await setInputValue(wrapper, "input-apiKey", "sk-bad");

		await wrapper.find('[data-testid="test-button"]').trigger("click");
		await flushPromises();

		expect(
			wrapper.find('[data-testid="test-result-icon-error"]').exists(),
		).toBe(true);
		expect(
			wrapper.find('[data-testid="test-result-icon-warning"]').exists(),
		).toBe(false);

		const saveButton = wrapper.find<HTMLButtonElement>(
			'[data-testid="save-button"]',
		);
		expect(saveButton.element.disabled).toBe(true);
	});

	it("enables save on an unknown response (no probe available) and shows no warning icon", async () => {
		mockState.testCredentialsImpl.mockImplementation(() => async () => ({
			valid: true as const,
			status: "unknown" as const,
			latencyMs: null,
			error: null,
			warning: "health_check_not_supported",
			method: "not_supported",
		}));

		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		await setInputValue(wrapper, "input-displayName", "Local Ollama");
		await setInputValue(wrapper, "input-apiKey", "sk-test");

		await wrapper.find('[data-testid="test-button"]').trigger("click");
		await flushPromises();

		// Unknown renders as the neutral/unknown icon (AlertCircle
		// in muted color), NOT as the warning or error icon.
		expect(
			wrapper.find('[data-testid="test-result-icon-unknown"]').exists(),
		).toBe(true);
		expect(
			wrapper.find('[data-testid="test-result-icon-warning"]').exists(),
		).toBe(false);
		expect(
			wrapper.find('[data-testid="test-result-icon-error"]').exists(),
		).toBe(false);

		// Save enabled — the previous `ok === true` rule blocked this
		// for Unknown (because ok was `null`); the new `valid === true`
		// rule correctly enables it.
		const saveButton = wrapper.find<HTMLButtonElement>(
			'[data-testid="save-button"]',
		);
		expect(saveButton.element.disabled).toBe(false);
	});
});

describe("AddProviderDialog — auth type gating", () => {
	it("disables the OAuth toggle when the catalog entry only supports apikey (ollama)", async () => {
		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		const oauthToggle = wrapper.find<HTMLButtonElement>(
			'[data-testid="auth-type-oauth"]',
		);
		expect(oauthToggle.exists()).toBe(true);
		expect(oauthToggle.element.disabled).toBe(true);
		expect(wrapper.find('[data-testid="oauth-coming-soon"]').exists()).toBe(
			true,
		);
	});
});

describe("AddProviderDialog — open/close", () => {
	it("hides the dialog when the open prop is set to false", async () => {
		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		expect(wrapper.find('[data-testid="dialog-root"]').exists()).toBe(true);

		await wrapper.setProps({ open: false });
		await nextTick();
		expect(wrapper.find('[data-testid="dialog-root"]').exists()).toBe(false);
	});
});

// ---------------------------------------------------------------------------
// Ollama Cloud — end-to-end flow
//
// These tests exercise the full create-provider pipeline for the new
// `ollama-cloud` kind, which is the first kind in the catalog that
// requires Bearer auth at the HTTP layer. The flow covered:
//
//   1. Catalog default base URL is `https://ollama.com`
//   2. Kind selector is hidden (providerKind is pre-scoped)
//   3. OAuth is not offered (only apikey is supported)
//   4. Test + save buttons enable only after apiKey is filled
//   5. The payload sent to the API carries:
//        - providerKind = "ollama-cloud"
//        - credentials.apiKey = the user-typed key
//        - config.baseUrl = "https://ollama.com"
//
// Combined with the wiremock tests in
// `crates/infrastructure/providers-ollama/tests/provider.rs`, this gives
// end-to-end coverage from the form to the Bearer-auth HTTP call.
// ---------------------------------------------------------------------------

describe("AddProviderDialog — Ollama Cloud (e2e)", () => {
	it("pre-scopes the dialog to ollama-cloud (no kind selector)", async () => {
		const wrapper = mountDialog({ providerKind: "ollama-cloud" });
		await flushPromises();
		expect(wrapper.find('[data-testid="kind-select-trigger"]').exists()).toBe(
			false,
		);
		expect(wrapper.find('[data-testid="dialog-root"]').exists()).toBe(true);
	});

	it("hides the baseUrl field (managed endpoint is vendor-fixed)", async () => {
		const wrapper = mountDialog({ providerKind: "ollama-cloud" });
		await flushPromises();
		// The field is hidden — Ollama Cloud's URL is fixed by the
		// vendor and the user has no reason to override it. We still
		// send the default URL in the request payload, so behavior is
		// unchanged (see the test-payload tests below).
		const baseUrlInput = wrapper.find<HTMLInputElement>(
			'[data-testid="input-baseUrl"]',
		);
    expect(baseUrlInput.exists(), "baseUrl input should be hidden").toBe(false);
	});

	it("shows the baseUrl field for self-hosted Ollama (user can override)", async () => {
		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		const baseUrlInput = wrapper.find<HTMLInputElement>(
			'[data-testid="input-baseUrl"]',
		);
    expect(baseUrlInput.exists(), "baseUrl input should be visible").toBe(true);
		expect(baseUrlInput.element.value).toBe("http://localhost:11434");
	});

	it("disables the OAuth toggle (ollama-cloud only supports apikey)", async () => {
		const wrapper = mountDialog({ providerKind: "ollama-cloud" });
		await flushPromises();
		const oauthToggle = wrapper.find<HTMLButtonElement>(
			'[data-testid="auth-type-oauth"]',
		);
		expect(oauthToggle.exists()).toBe(true);
		expect(oauthToggle.element.disabled).toBe(true);
		expect(wrapper.find('[data-testid="oauth-coming-soon"]').exists()).toBe(
			true,
		);
	});

	it("keeps the test button disabled until displayName and apiKey are filled", async () => {
		const wrapper = mountDialog({ providerKind: "ollama-cloud" });
		await flushPromises();
		const testButton = wrapper.find<HTMLButtonElement>(
			'[data-testid="test-button"]',
		);
		expect(testButton.element.disabled).toBe(true);

		// Only displayName is not enough — apiKey is required.
		await setInputValue(wrapper, "input-displayName", "My Cloud");
		expect(testButton.element.disabled).toBe(true);

		// With both, the test button enables.
		await setInputValue(wrapper, "input-apiKey", "ollama-cloud-key-1234");
		expect(testButton.element.disabled).toBe(false);
	});

	it("sends a properly shaped test payload (kind, apiKey, baseUrl) to testCredentials", async () => {
		let captured: unknown;
		mockState.testCredentialsImpl.mockImplementation(
			() => async (payload: unknown) => {
				captured = payload;
				return {
					valid: true as const,
					status: "ok" as const,
					latencyMs: 99,
					error: null,
					warning: null,
					method: "models_list",
				};
			},
		);

		const wrapper = mountDialog({ providerKind: "ollama-cloud" });
		await flushPromises();
		await setInputValue(wrapper, "input-displayName", "My Cloud");
		await setInputValue(wrapper, "input-apiKey", "ollama-cloud-key-1234");

		await wrapper.find('[data-testid="test-button"]').trigger("click");
		await flushPromises();

		expect(captured, "testCredentials should have been called").toBeDefined();
		const payload = captured as {
			providerKind: string;
			authType: string;
			credentials: { apiKey: string };
			config: { baseUrl: string };
		};
		expect(payload.providerKind).toBe("ollama-cloud");
		expect(payload.authType).toBe("apiKey");
		expect(payload.credentials.apiKey).toBe("ollama-cloud-key-1234");
		expect(payload.config.baseUrl).toBe("https://ollama.com");
	});

	it("saves the connection with the ollama-cloud kind and credentials", async () => {
		const wrapper = mountDialog({ providerKind: "ollama-cloud" });
		await flushPromises();
		await setInputValue(wrapper, "input-displayName", "My Cloud");
		await setInputValue(wrapper, "input-apiKey", "ollama-cloud-key-1234");

		// Test must pass before save is enabled.
		await wrapper.find('[data-testid="test-button"]').trigger("click");
		await flushPromises();

		const saveButton = wrapper.find<HTMLButtonElement>(
			'[data-testid="save-button"]',
		);
		expect(saveButton.element.disabled).toBe(false);
		await saveButton.trigger("click");
		await flushPromises();

		expect(mockState.create).toHaveBeenCalledTimes(1);
		const createArg = mockState.create.mock.calls[0][0] as {
			providerKind: string;
			authType: string;
			name: string;
			credentials: { apiKey: string };
			config: { baseUrl: string };
		};
		expect(createArg.providerKind).toBe("ollama-cloud");
		expect(createArg.authType).toBe("apiKey");
		expect(createArg.name).toBe("My Cloud");
		expect(createArg.credentials.apiKey).toBe("ollama-cloud-key-1234");
		expect(createArg.config.baseUrl).toBe("https://ollama.com");

		expect(wrapper.emitted("saved")).toBeTruthy();
	});

	it("respects a user-overridden base URL for self-hosted Ollama (advanced use case)", async () => {
		let captured: unknown;
		mockState.testCredentialsImpl.mockImplementation(
			() => async (payload: unknown) => {
				captured = payload;
				return {
					valid: true as const,
					status: "ok" as const,
					latencyMs: 50,
					error: null,
					warning: null,
					method: "models_list",
				};
			},
		);

		const wrapper = mountDialog({ providerKind: "ollama" });
		await flushPromises();
		await setInputValue(wrapper, "input-displayName", "Proxy");
		await setInputValue(wrapper, "input-apiKey", "key-via-proxy");
		// The user can point to a local proxy that fronts the Ollama server.
		await setInputValue(
			wrapper,
			"input-baseUrl",
			"http://localhost:9999/ollama",
		);

		await wrapper.find('[data-testid="test-button"]').trigger("click");
		await flushPromises();

		const payload = captured as { config: { baseUrl: string } };
		expect(payload.config.baseUrl).toBe("http://localhost:9999/ollama");
	});
});
