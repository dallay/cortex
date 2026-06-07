import { expect, test } from "@playwright/test";

test.describe("Provider Management", () => {
	test.beforeEach(async ({ page }) => {
		// Navigate to the app and login
		await page.goto("/");

		// Check if we need to login
		const loginForm = page.locator("form");
		if (await loginForm.isVisible()) {
			await page
				.getByRole("textbox", { name: "Password" })
				.fill("S3cr3tP@ssw0rd*123");
			await page.getByRole("button", { name: "Sign in" }).click();
			await expect(page).toHaveURL("/");
		}

		// Navigate to providers page
		await page.goto("/providers");
	});

	test("should display providers page", async ({ page }) => {
		await expect(
			page.getByRole("heading", { name: "Providers" }),
		).toBeVisible();
		await expect(page.getByText("Configure your AI providers")).toBeVisible();
	});

	test("should open add provider dialog", async ({ page }) => {
		await page.getByRole("button", { name: "Add Provider" }).click();

		await expect(
			page.getByRole("dialog", { name: "Add Provider" }),
		).toBeVisible();
		await expect(
			page.getByText("Connect a new AI provider to Rook"),
		).toBeVisible();
	});

	test("should validate required fields", async ({ page }) => {
		await page.getByRole("button", { name: "Add Provider" }).click();

		// Save button should be disabled with empty form
		const saveButton = page.getByRole("button", { name: "Save" }).last();
		await expect(saveButton).toBeDisabled();

		// Fill only name
		await page.getByRole("textbox", { name: "Name" }).fill("Test Provider");
		await expect(saveButton).toBeDisabled();

		// Fill API key - now save should be enabled
		await page
			.getByRole("textbox", { name: "API Key" })
			.fill("test-api-key-123");
		await expect(saveButton).toBeEnabled();
	});

	test("should add a new ollama provider", async ({ page }) => {
		// Click add provider button
		await page.getByRole("button", { name: "Add Provider" }).click();

		// Fill form
		await page.getByRole("textbox", { name: "Name" }).fill("Ollama Test E2E");
		await page
			.getByRole("textbox", { name: "API Key" })
			.fill("ollama-test-key-e2e");
		await page
			.getByRole("textbox", { name: "Base URL" })
			.fill("https://api.ollama.com");

		// Save
		await page.getByRole("button", { name: "Save" }).last().click();

		// Dialog should close
		await expect(
			page.getByRole("dialog", { name: "Add Provider" }),
		).not.toBeVisible();

		// Provider should appear in the list
		await expect(page.getByText("Ollama Test E2E")).toBeVisible();
		await expect(page.getByText("ollama")).toBeVisible();
	});

	test("should show advanced configuration", async ({ page }) => {
		await page.getByRole("button", { name: "Add Provider" }).click();

		// Advanced config should be collapsed initially
		await expect(page.getByLabel("Max Concurrent Requests")).not.toBeVisible();

		// Click to expand
		await page.getByRole("button", { name: "Advanced Configuration" }).click();

		// Now advanced fields should be visible
		await expect(page.getByLabel("Max Concurrent Requests")).toBeVisible();
		await expect(page.getByLabel("Default Model")).toBeVisible();
	});

	test("should reset form when dialog is closed", async ({ page }) => {
		// Open dialog and fill some fields
		await page.getByRole("button", { name: "Add Provider" }).click();
		await page.getByRole("textbox", { name: "Name" }).fill("Test");
		await page.getByRole("textbox", { name: "API Key" }).fill("key");

		// Close dialog
		await page.getByRole("button", { name: "Close" }).click();

		// Reopen dialog
		await page.getByRole("button", { name: "Add Provider" }).click();

		// Fields should be empty
		await expect(page.getByRole("textbox", { name: "Name" })).toHaveValue("");
		await expect(page.getByRole("textbox", { name: "API Key" })).toHaveValue(
			"",
		);
	});
});

test.describe("Provider Catalog — Ollama Cloud card", () => {
	test.beforeEach(async ({ page }) => {
		await page.goto("/");
		const loginForm = page.locator("form");
		if (await loginForm.isVisible()) {
			await page
				.getByRole("textbox", { name: "Password" })
				.fill("S3cr3tP@ssw0rd*123");
			await page.getByRole("button", { name: "Sign in" }).click();
			await expect(page).toHaveURL("/");
		}
		await page.goto("/providers");
	});

	test("renders the Ollama Cloud card with auth-required description", async ({
		page,
	}) => {
		// The card must be present (regression for bug #1 — i18n dot-in-key).
		const card = page.getByTestId("provider-card-ollama-cloud");
		await expect(card).toBeVisible();

		// The card title resolves to the real name, not the raw i18n key.
		await expect(card.getByText("Ollama Cloud", { exact: true })).toBeVisible();

		// The description must mention the API key requirement (the new copy).
		await expect(
			card.getByText(/Requires an API key from ollama\.com\/settings\/keys/i),
		).toBeVisible();
	});

	test("clicking the Add button on the Ollama Cloud card opens a pre-scoped dialog", async ({
		page,
	}) => {
		await page.getByTestId("provider-card-add-ollama-cloud").click();

		// The dialog opens with a dynamic title — `Add {providerName}
		// connection` (i18n key `providers.form.createTitle`). For
		// ollama-cloud this resolves to "Add Ollama Cloud connection".
		await expect(
			page.getByRole("dialog", { name: /Add Ollama Cloud connection/i }),
		).toBeVisible();

		// No kind selector — providerKind is pre-scoped from the card click.
		await expect(page.getByTestId("kind-select-trigger")).toHaveCount(0);

		// The Base URL field is HIDDEN for managed-cloud providers
		// (vendor-fixed endpoint, users should not edit it). The
		// catalog default is still sent in the request payload —
		// verified by intercepting the test-credentials call below.
		await expect(
			page.getByRole("textbox", { name: "Base URL" }),
		).toHaveCount(0);

		// Fill the form and intercept the test-credentials request to
		// confirm the hidden baseUrl is correctly carried through as
		// https://ollama.com.
		await page.getByRole("textbox", { name: "Display Name" }).fill("Cloud E2E");
		await page.getByRole("textbox", { name: "API Key" }).fill("test-key-e2e");

		const testRequestPromise = page.waitForRequest(
			(req) =>
				req.url().includes("/api/providers/test-credentials") &&
				req.method() === "POST",
			{ timeout: 10_000 },
		);
		await page.getByTestId("test-button").click();
		const testRequest = await testRequestPromise;
		const body = testRequest.postDataJSON() as {
			providerKind: string;
			config: { baseUrl?: string };
		};
		expect(body.providerKind).toBe("ollama-cloud");
		expect(body.config.baseUrl).toBe("https://ollama.com");
	});
});
