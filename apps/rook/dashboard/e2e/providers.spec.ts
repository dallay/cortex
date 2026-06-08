import { expect, test } from "@playwright/test";

// ---------------------------------------------------------------------------
// Helper — login once per describe block
// ---------------------------------------------------------------------------

async function loginIfNeeded(page: import("@playwright/test").Page) {
  await page.goto("/");
  const loginForm = page.locator("form");
  if (await loginForm.isVisible()) {
    await page.getByRole("textbox", {name: "Password"}).fill("S3cr3tP@ssw0rd*123");
    await page.getByRole("button", {name: "Sign in"}).click();
    await expect(page).toHaveURL("/");
  }
}

// ---------------------------------------------------------------------------
// Provider Catalog — Ollama Cloud card (existing tests, kept intact)
// ---------------------------------------------------------------------------

test.describe("Provider Catalog — Ollama Cloud card", () => {
	test.beforeEach(async ({ page }) => {
    await loginIfNeeded(page);
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

// ---------------------------------------------------------------------------
// Provider Detail Polish — new tests for the drill-in fix and title link
// ---------------------------------------------------------------------------

test.describe("Provider Detail Polish", () => {
  test.beforeEach(async ({page}) => {
    await loginIfNeeded(page);
  });

  test("clicking the Ollama Cloud card navigates to /providers/ollama-cloud", async ({
                                                                                       page,
                                                                                     }) => {
    await page.goto("/providers");
    // Click the card body link (not the Add button)
    await page.getByTestId("provider-card-link-ollama-cloud").click();
    await expect(page).toHaveURL(/\/providers\/ollama-cloud$/);
  });

  test("detail header title link has correct href, target, rel, and aria-label", async ({
                                                                                          page,
                                                                                        }) => {
    await page.goto("/providers/ollama-cloud");
    const link = page.getByRole("link", {name: /Ollama Cloud — opens in new tab/i});
    await expect(link).toBeVisible();
    await expect(link).toHaveAttribute("href", "https://ollama.com/cloud");
    await expect(link).toHaveAttribute("target", "_blank");
    await expect(link).toHaveAttribute("rel", /noopener/);
    await expect(link).toHaveAttribute("rel", /noreferrer/);
  });

  test("invalid kind /providers/foo redirects to /providers", async ({
                                                                       page,
                                                                     }) => {
    await page.goto("/providers/foo");
    await expect(page).toHaveURL(/\/providers$/);
  });
});
