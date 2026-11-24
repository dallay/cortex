import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import CodexAccountsDialog from "./CodexAccountsDialog.vue";

const api = vi.hoisted(() => ({
	getCodexAccounts: vi.fn(),
	startCodexLogin: vi.fn(),
	cancelCodexLogin: vi.fn(),
}));
vi.mock("@/lib/api", () => ({ useApi: () => api }));

const browser = vi.hoisted(() => ({ isLoopbackBrowser: vi.fn(() => true) }));
vi.mock("@/lib/browserHost", () => browser);

function mountDialog() {
	return mount(CodexAccountsDialog, {
		props: { open: true },
		global: {
			plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })],
			stubs: {
				Dialog: { template: "<div><slot /></div>" },
				DialogContent: { template: "<div><slot /></div>" },
				DialogHeader: { template: "<div><slot /></div>" },
				DialogTitle: { template: "<h2><slot /></h2>" },
				DialogDescription: { template: "<p><slot /></p>" },
			},
		},
	});
}

afterEach(() => {
	vi.resetAllMocks();
	browser.isLoopbackBrowser.mockReturnValue(true);
});

describe("Codex accounts dialog", () => {
	it("shows linked accounts without asking for API credentials and opens a safe sign-in link", async () => {
		const accounts = [{ id: "one" }, { id: "two" }];
		const authUrl = "https://auth.openai.com/oauth/authorize?state=example";
		api.getCodexAccounts
			.mockResolvedValueOnce({ enabled: true, accounts, attempt: null })
			.mockResolvedValueOnce({
				enabled: true,
				accounts,
				attempt: { id: "three", status: "pending" },
			})
			.mockResolvedValueOnce({
				enabled: true,
				accounts,
				attempt: { id: "three", status: "cancelled" },
			});
		api.startCodexLogin.mockResolvedValueOnce({ id: "three", authUrl });
		const wrapper = mountDialog();
		try {
			await flushPromises();
			expect(wrapper.text()).toContain("Account one");
			expect(wrapper.text()).not.toContain("ChatGPT plan:");
			expect(wrapper.text()).toContain("Account two");
			expect(wrapper.find("input, select").exists()).toBe(false);
			await wrapper.get("button").trigger("click");
			await flushPromises();
			expect(api.startCodexLogin).toHaveBeenCalledOnce();
			expect(wrapper.get("a").attributes()).toMatchObject({
				href: authUrl,
				target: "_blank",
				rel: "noopener noreferrer",
			});
			expect(wrapper.get('[role="status"]').text()).toContain(
				"Waiting for ChatGPT",
			);
			await wrapper.get("button").trigger("click");
			await flushPromises();
			expect(api.cancelCodexLogin).toHaveBeenCalledWith("three");
			expect(wrapper.get('[role="status"]').text()).toContain("cancelled");
			expect(wrapper.find("a").exists()).toBe(false);
		} finally {
			wrapper.unmount();
		}
	});

	it("does not resurrect a pending authorization link after closing the dialog", async () => {
		api.getCodexAccounts
			.mockResolvedValueOnce({ enabled: true, accounts: [], attempt: null })
			.mockResolvedValue({
				enabled: true,
				accounts: [],
				attempt: { id: "one", status: "pending" },
			});
		let finish!: (login: { id: string; authUrl: string }) => void;
		api.startCodexLogin.mockReturnValueOnce(
			new Promise((resolve) => {
				finish = resolve;
			}),
		);
		const wrapper = mountDialog();
		try {
			await flushPromises();
			await wrapper.get("button").trigger("click");
			await wrapper.setProps({ open: false });
			finish({
				id: "one",
				authUrl: "https://auth.openai.com/oauth/authorize?state=synthetic",
			});
			await flushPromises();
			await wrapper.setProps({ open: true });
			await flushPromises();
			expect(wrapper.find("a").exists()).toBe(false);
		} finally {
			wrapper.unmount();
		}
	});

	it("explains that the operator has not configured Codex", async () => {
		api.getCodexAccounts.mockResolvedValueOnce({
			enabled: false,
			accounts: [],
			attempt: null,
		});
		const wrapper = mountDialog();
		try {
			await flushPromises();
			expect(wrapper.text()).toContain("not configured on this Rook server");
			expect(wrapper.text()).not.toContain("same host as the Rook server");
			expect(wrapper.find("button, a").exists()).toBe(false);
		} finally {
			wrapper.unmount();
		}
	});

	it("explains that linking requires a browser on the Rook host", async () => {
		browser.isLoopbackBrowser.mockReturnValue(false);
		api.getCodexAccounts.mockResolvedValueOnce({
			enabled: true,
			accounts: [],
			attempt: null,
		});
		const wrapper = mountDialog();
		try {
			await flushPromises();
			expect(wrapper.text()).toContain("same host as the Rook server");
			expect(wrapper.text()).not.toContain(
				"not configured on this Rook server",
			);
			expect(wrapper.find("button, a").exists()).toBe(false);
		} finally {
			wrapper.unmount();
		}
	});
});
