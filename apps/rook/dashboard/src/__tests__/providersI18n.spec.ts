/**
 * i18n smoke tests for the providers namespace.
 *
 * Regression guard for the bug where category keys
 * (`providers.category.apiKey.description` etc.) were stored as flat
 * keys with literal dots in the property name, so vue-i18n tried to
 * walk the path `category.apiKey.description` and fell back to the
 * raw key string (displaying `providers.category.apiKey.description`
 * on the page). These tests assert that every key referenced by
 * `providerCatalog.ts` actually resolves to a non-key string.
 */
import { describe, expect, it } from "vitest";
import { createI18n } from "vue-i18n";
import { CATEGORIES, PROVIDER_KINDS } from "@/config/providerCatalog";
import en from "@/locales/en.json";
import es from "@/locales/es.json";

function makeI18n(locale: "en" | "es") {
	return createI18n({
		legacy: false,
		locale,
		fallbackLocale: "en",
		messages: { en, es },
		// When a key is missing, return an empty string so the test can
		// distinguish "missing" from "found the literal key".
		missing: () => "",
	});
}

describe("providers i18n keys", () => {
	for (const locale of ["en", "es"] as const) {
		describe(`locale: ${locale}`, () => {
			const i18n = makeI18n(locale);
			const t = i18n.global.t as (key: string) => string;

			it("resolves every category display name key", () => {
				for (const c of CATEGORIES) {
					const resolved = t(c.displayNameKey);
					expect(resolved, c.displayNameKey).not.toBe("");
					// The buggy flat form would resolve to the key itself.
					expect(resolved, c.displayNameKey).not.toBe(c.displayNameKey);
				}
			});

			it("resolves every category description key", () => {
				for (const c of CATEGORIES) {
					const resolved = t(c.descriptionKey);
					expect(resolved, c.descriptionKey).not.toBe("");
					expect(resolved, c.descriptionKey).not.toBe(c.descriptionKey);
				}
			});

			it("resolves every provider kind display name key", () => {
				for (const k of PROVIDER_KINDS) {
					const resolved = t(k.displayNameKey);
					expect(resolved, k.displayNameKey).not.toBe("");
					expect(resolved, k.displayNameKey).not.toBe(k.displayNameKey);
				}
			});

			it("resolves every provider kind description key", () => {
				for (const k of PROVIDER_KINDS) {
					const resolved = t(k.descriptionKey);
					expect(resolved, k.descriptionKey).not.toBe("");
					expect(resolved, k.descriptionKey).not.toBe(k.descriptionKey);
				}
			});
		});
	}
});

describe("regression — literal-dot keys no longer leak into the UI", () => {
	for (const locale of ["en", "es"] as const) {
		const i18n = makeI18n(locale);
		const t = i18n.global.t as (key: string) => string;

		it(`[${locale}] providers.category.apiKey.description resolves to a sentence`, () => {
			const resolved = t("providers.category.apiKey.description");
			expect(resolved.length, "should be more than 5 chars").toBeGreaterThan(5);
			// Should not be the raw key.
			expect(resolved).not.toBe("providers.category.apiKey.description");
			// Should not be empty.
			expect(resolved).not.toBe("");
		});

		it(`[${locale}] providers.kind.openai.description resolves to a sentence`, () => {
			const resolved = t("providers.kind.openai.description");
			expect(resolved.length).toBeGreaterThan(5);
			expect(resolved).not.toBe("providers.kind.openai.description");
			expect(resolved).not.toBe("");
		});

		it(`[${locale}] providers.kind.ollamaCloud.description resolves`, () => {
			const resolved = t("providers.kind.ollamaCloud.description");
			expect(resolved).not.toBe("providers.kind.ollamaCloud.description");
			expect(resolved).not.toBe("");
		});

		it(`[${locale}] providers.kind.ollamaCloud.name resolves to a real name`, () => {
			const resolved = t("providers.kind.ollamaCloud.name");
			// The bug fallback returned the raw key. The real value is
			// either 'Ollama Cloud' (en) or 'Ollama Cloud' (es).
			expect(resolved).toBe("Ollama Cloud");
		});
	}
});
