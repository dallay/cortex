import { flushPromises, mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createI18n } from "vue-i18n";
import en from "@/locales/en.json";
import CodexAccountsDialog from "./CodexAccountsDialog.vue";

const api = vi.hoisted(() => ({
	getCodexAccounts: vi.fn(),
	startCodexLogin: vi.fn(),
	cancelCodexLogin: vi.fn(),
	disconnectCodexAccount: vi.fn(),
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
				AlertDialog: {
					props: ["open"],
					template: '<div v-if="open"><slot /></div>',
				},
				AlertDialogContent: { template: "<div><slot /></div>" },
				AlertDialogHeader: { template: "<div><slot /></div>" },
				AlertDialogTitle: { template: "<h2><slot /></h2>" },
				AlertDialogDescription: { template: "<p><slot /></p>" },
				AlertDialogFooter: { template: "<div><slot /></div>" },
				AlertDialogAction: {
					emits: ["click"],
					template:
						"<button @click=\"$emit('click', $event)\"><slot /></button>",
				},
				AlertDialogCancel: { template: "<button><slot /></button>" },
			},
		},
	});
}

afterEach(() => {
	vi.useRealTimers();
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
			await wrapper.get('[data-testid="codex-start"]').trigger("click");
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
			await wrapper.get('[data-testid="codex-cancel"]').trigger("click");
			await flushPromises();
			expect(api.cancelCodexLogin).toHaveBeenCalledWith("three");
			expect(wrapper.get('[role="status"]').text()).toContain("cancelled");
			expect(wrapper.find("a").exists()).toBe(false);
		} finally {
			wrapper.unmount();
		}
	});

	it("disconnects a linked account only after explicit confirmation", async () => {
		api.getCodexAccounts
			.mockResolvedValueOnce({
				enabled: true,
				accounts: [{ id: "one" }],
				attempt: null,
			})
			.mockResolvedValueOnce({ enabled: true, accounts: [], attempt: null });
		api.disconnectCodexAccount.mockResolvedValueOnce(undefined);
		const wrapper = mountDialog();
		try {
			await flushPromises();
			await wrapper
				.get('[data-testid="codex-disconnect-one"]')
				.trigger("click");

			expect(api.disconnectCodexAccount).not.toHaveBeenCalled();
			expect(wrapper.text()).toContain("does not sign you out of OpenAI");

			await wrapper
				.get('[data-testid="codex-disconnect-confirm"]')
				.trigger("click");
			await flushPromises();

			expect(api.disconnectCodexAccount).toHaveBeenCalledOnce();
			expect(api.disconnectCodexAccount).toHaveBeenCalledWith("one");
			expect(wrapper.text()).not.toContain("Account one");
		} finally {
			wrapper.unmount();
		}
	});

	it("closes and emits the snapshot only when the attempt it started becomes linked", async () => {
		vi.useFakeTimers();
		const linkedSnapshot = {
			enabled: true,
			accounts: [{ id: "linked-account" }],
			attempt: { id: "attempt-1", status: "linked" as const },
		};
		api.getCodexAccounts
			.mockResolvedValueOnce({ enabled: true, accounts: [], attempt: null })
			.mockResolvedValueOnce({
				enabled: true,
				accounts: [],
				attempt: { id: "attempt-1", status: "pending" },
			})
			.mockResolvedValueOnce(linkedSnapshot);
		api.startCodexLogin.mockResolvedValueOnce({
			id: "attempt-1",
			authUrl: "https://auth.openai.com/oauth/authorize?state=example",
		});
		const wrapper = mountDialog();
		try {
			await flushPromises();
			await wrapper.get('[data-testid="codex-start"]').trigger("click");
			await flushPromises();

			expect(wrapper.emitted("update:open")).toBeUndefined();
			vi.advanceTimersByTime(2000);
			await flushPromises();

			expect(wrapper.emitted("update:open")).toEqual([[false]]);
			expect(wrapper.emitted("linked")).toEqual([[linkedSnapshot]]);
		} finally {
			wrapper.unmount();
		}
	});

	it("does not close for a linked terminal state from an older attempt", async () => {
		const staleSnapshot = {
			enabled: true,
			accounts: [{ id: "older-account" }],
			attempt: { id: "old-attempt", status: "linked" as const },
		};
		api.getCodexAccounts
			.mockResolvedValueOnce({ enabled: true, accounts: [], attempt: null })
			.mockResolvedValueOnce(staleSnapshot);
		api.startCodexLogin.mockResolvedValueOnce({
			id: "attempt-1",
			authUrl: "https://auth.openai.com/oauth/authorize?state=example",
		});
		const wrapper = mountDialog();
		try {
			await flushPromises();
			await wrapper.get('[data-testid="codex-start"]').trigger("click");
			await flushPromises();

			expect(wrapper.props("open")).toBe(true);
			expect(wrapper.emitted("update:open")).toBeUndefined();
			expect(wrapper.emitted("linked")).toBeUndefined();
			expect(wrapper.get('[role="status"]').text()).toContain(
				"ChatGPT account linked.",
			);
		} finally {
			wrapper.unmount();
		}
	});

	it("keeps the dialog open and does not emit linked after cancellation", async () => {
		api.getCodexAccounts
			.mockResolvedValueOnce({ enabled: true, accounts: [], attempt: null })
			.mockResolvedValueOnce({
				enabled: true,
				accounts: [],
				attempt: { id: "attempt-1", status: "pending" },
			})
			.mockResolvedValueOnce({
				enabled: true,
				accounts: [],
				attempt: { id: "attempt-1", status: "cancelled" },
			});
		api.startCodexLogin.mockResolvedValueOnce({
			id: "attempt-1",
			authUrl: "https://auth.openai.com/oauth/authorize?state=example",
		});
		const wrapper = mountDialog();
		try {
			await flushPromises();
			await wrapper.get('[data-testid="codex-start"]').trigger("click");
			await flushPromises();
			await wrapper.get('[data-testid="codex-cancel"]').trigger("click");
			await flushPromises();

			expect(wrapper.props("open")).toBe(true);
			expect(wrapper.emitted("update:open")).toBeUndefined();
			expect(wrapper.emitted("linked")).toBeUndefined();
			expect(wrapper.get('[role="status"]').text()).toContain(
				"Sign-in cancelled.",
			);
		} finally {
			wrapper.unmount();
		}
	});

	it.each(["expired", "failed"] as const)(
		"keeps the dialog open after the current attempt is %s",
		async (status) => {
			api.getCodexAccounts
				.mockResolvedValueOnce({ enabled: true, accounts: [], attempt: null })
				.mockResolvedValueOnce({
					enabled: true,
					accounts: [],
					attempt: { id: "attempt-1", status },
				});
			api.startCodexLogin.mockResolvedValueOnce({
				id: "attempt-1",
				authUrl: "https://auth.openai.com/oauth/authorize?state=example",
			});
			const wrapper = mountDialog();
			try {
				await flushPromises();
				await wrapper.get('[data-testid="codex-start"]').trigger("click");
				await flushPromises();

				expect(wrapper.props("open")).toBe(true);
				expect(wrapper.emitted("update:open")).toBeUndefined();
				expect(wrapper.emitted("linked")).toBeUndefined();
				expect(wrapper.get('[role="status"]').text()).toContain(status);
			} finally {
				wrapper.unmount();
			}
		},
	);

	it("keeps the dialog open and exposes retry after an account status error", async () => {
		api.getCodexAccounts
			.mockResolvedValueOnce({ enabled: true, accounts: [], attempt: null })
			.mockRejectedValueOnce(new Error("request failed"));
		api.startCodexLogin.mockResolvedValueOnce({
			id: "attempt-1",
			authUrl: "https://auth.openai.com/oauth/authorize?state=example",
		});
		const wrapper = mountDialog();
		try {
			await flushPromises();
			await wrapper.get('[data-testid="codex-start"]').trigger("click");
			await flushPromises();

			expect(wrapper.props("open")).toBe(true);
			expect(wrapper.emitted("update:open")).toBeUndefined();
			expect(wrapper.emitted("linked")).toBeUndefined();
			expect(wrapper.text()).toContain(
				"Rook could not load or change the Codex accounts",
			);
			expect(wrapper.findComponent({ name: "ErrorBanner" }).exists()).toBe(
				true,
			);
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
			await wrapper.get('[data-testid="codex-start"]').trigger("click");
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

	it("does not restart polling when a login request completes after disposal", async () => {
		api.getCodexAccounts.mockResolvedValue({
			enabled: true,
			accounts: [],
			attempt: null,
		});
		let finish!: (login: { id: string; authUrl: string }) => void;
		api.startCodexLogin.mockReturnValueOnce(
			new Promise((resolve) => {
				finish = resolve;
			}),
		);
		const wrapper = mountDialog();
		await flushPromises();
		await wrapper.get('[data-testid="codex-start"]').trigger("click");
		wrapper.unmount();

		finish({
			id: "attempt-after-dispose",
			authUrl: "https://auth.openai.com/oauth/authorize?state=synthetic",
		});
		await flushPromises();

		expect(api.getCodexAccounts).toHaveBeenCalledOnce();
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
