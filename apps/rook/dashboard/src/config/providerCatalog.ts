/**
 * Provider catalog — static metadata for every supported `ProviderKind`.
 *
 * This module is the **single source of truth** for the TS-level
 * `ProviderKind` and `AuthType` unions (mirroring the backend
 * `crates/shared-kernel` and `crates/rook-core` enums). The kind set
 * is stable (5 entries); revisit the API at the 6th kind. See
 * `openspec/changes/providers-ui-3-screen-refactor/design.md` §D3.
 *
 * The catalog is consumed by:
 *   - `useProviderCatalog()` — derived composable that joins static
 *     metadata with the live connection list (`useProviders`) and the
 *     available-models list (`useAvailableModels`).
 *   - Future: `ProviderCatalogCard` / `ProviderDetailsView` (Phase 4/5).
 *
 * NOTE: The `logoIconName` field is a **string** referencing a lucide
 * icon by its PascalCase name (e.g. `"Cpu"`). Icon resolution is the
 * responsibility of the rendering component (Phase 4). The icon does
 * NOT need to be registered in `useNavigation`'s registry — that
 * registry is exclusive to the sidebar nav.
 */

/** Backend `ProviderKind` enum (6 values). Ollama Cloud shares the
 *  same provider implementation as local Ollama — it differs only in
 *  the base URL (`https://ollama.com`) and the required Bearer auth
 *  token. Split out at the enum level so the dashboard, route
 *  resolution, and connection list treat them as separate kinds. */
export type ProviderKind =
	| "openai"
	| "anthropic"
	| "ollama"
	| "ollama-cloud"
	| "gemini"
	| "groq";

/** Backend `AuthType` enum (2 values, stable for v1 — OAuth is gated). */
export type AuthType = "apikey" | "oauth";

/** Coarse grouping used by the catalog view (filters + section headers). */
export type CategoryKind = "api-key" | "oauth" | "local";

/** Single static entry in the provider catalog. */
export interface CatalogEntry {
	/** Stable identifier (matches backend `ProviderKind`). */
	readonly kind: ProviderKind;
	/** i18n key for the display name (e.g. `'providers.kind.openai.name'`). */
	readonly displayNameKey: string;
	/** Grouping for the catalog view. */
	readonly category: CategoryKind;
	/** Default base URL (overridable in the connection form). */
	readonly defaultBaseUrl: string;
	/**
	 * Whether the user can override `defaultBaseUrl` in the connection
	 * form. `false` for managed-cloud providers whose endpoint is
	 * fixed by the vendor (Ollama Cloud). The base URL is still sent
	 * to the backend — the field is just hidden, not removed from
	 * the request payload — so behavior is unchanged if a deployer
	 * ever needs to override (e.g. enterprise proxy).
	 */
	readonly baseUrlEditable?: boolean;
	/** Lucide icon name (PascalCase, e.g. `"Cpu"`). Resolved by the renderer. */
	readonly logoIconName: string;
	/** Auth types this provider supports. */
	readonly authTypes: readonly AuthType[];
	/** i18n key for the description shown on the catalog card. */
	readonly descriptionKey: string;
	/** Optional link to the provider's documentation. */
	readonly docsUrl?: string;
	/**
	 * Default models shown in the "Suggested models" section of the
	 * connection modal. Empty for self-hosted providers (e.g. Ollama
	 * pulls from the local server on demand).
	 */
	readonly defaultModels: readonly string[];
}

/**
 * The full provider catalog. Order is intentional: API-key cloud
 * providers first (most common), then local. This order is also the
 * default render order in the catalog view.
 */
export const PROVIDER_KINDS: readonly CatalogEntry[] = [
	{
		kind: "openai",
		displayNameKey: "providers.kind.openai.name",
		category: "api-key",
		defaultBaseUrl: "https://api.openai.com/v1",
		logoIconName: "Cpu",
		authTypes: ["apikey"],
		descriptionKey: "providers.kind.openai.description",
		docsUrl: "https://platform.openai.com/docs",
		defaultModels: [
			"gpt-4o",
			"gpt-4o-mini",
			"gpt-4-turbo",
			"o1-preview",
			"o1-mini",
		],
	},
	{
		kind: "anthropic",
		displayNameKey: "providers.kind.anthropic.name",
		category: "api-key",
		defaultBaseUrl: "https://api.anthropic.com",
		logoIconName: "Sparkles",
		authTypes: ["apikey"],
		descriptionKey: "providers.kind.anthropic.description",
		docsUrl: "https://docs.anthropic.com",
		defaultModels: [
			"claude-3-5-sonnet-latest",
			"claude-3-5-haiku-latest",
			"claude-3-opus-latest",
		],
	},
	{
		kind: "gemini",
		displayNameKey: "providers.kind.gemini.name",
		category: "api-key",
		defaultBaseUrl: "https://generativelanguage.googleapis.com",
		logoIconName: "Brain",
		authTypes: ["apikey"],
		descriptionKey: "providers.kind.gemini.description",
		docsUrl: "https://ai.google.dev/gemini-api/docs",
		defaultModels: [
			"gemini-2.0-flash-exp",
			"gemini-1.5-pro",
			"gemini-1.5-flash",
		],
	},
	{
		kind: "groq",
		displayNameKey: "providers.kind.groq.name",
		category: "api-key",
		defaultBaseUrl: "https://api.groq.com/openai/v1",
		logoIconName: "Zap",
		authTypes: ["apikey"],
		descriptionKey: "providers.kind.groq.description",
		docsUrl: "https://console.groq.com/docs",
		defaultModels: [
			"llama-3.3-70b-versatile",
			"llama-3.1-8b-instant",
			"mixtral-8x7b-32768",
		],
	},
	{
		kind: "ollama",
		displayNameKey: "providers.kind.ollama.name",
		category: "local",
		defaultBaseUrl: "http://localhost:11434",
		logoIconName: "Server",
		authTypes: ["apikey"],
		descriptionKey: "providers.kind.ollama.description",
		docsUrl: "https://github.com/ollama/ollama/blob/main/docs/api.md",
		defaultModels: [],
	},
	{
		kind: "ollama-cloud",
		displayNameKey: "providers.kind.ollamaCloud.name",
		category: "api-key",
		defaultBaseUrl: "https://ollama.com",
		// Managed endpoint — the URL is vendor-fixed, hiding the field
		// prevents users from pointing at a wrong host. If a deployer
		// needs to route through a proxy, they can edit the entry
		// directly in the catalog (or extend with a per-tenant
		// override in a future iteration).
		baseUrlEditable: false,
		logoIconName: "Cloud",
		authTypes: ["apikey"],
		descriptionKey: "providers.kind.ollamaCloud.description",
		docsUrl: "https://docs.ollama.com/api-reference/chat.md",
		// Ollama Cloud's model library is dynamic — the user must set
		// models per connection. Surfacing a default list here would
		// suggest models the cloud might not serve.
		defaultModels: [],
	},
] as const;

/** Catalog filter categories — the 3 grouping buckets used by the view. */
export interface CategoryDescriptor {
	readonly kind: CategoryKind;
	readonly displayNameKey: string;
	readonly descriptionKey: string;
}

export const CATEGORIES: readonly CategoryDescriptor[] = [
	{
		kind: "api-key",
		displayNameKey: "providers.category.apiKey.name",
		descriptionKey: "providers.category.apiKey.description",
	},
	{
		kind: "oauth",
		displayNameKey: "providers.category.oauth.name",
		descriptionKey: "providers.category.oauth.description",
	},
	{
		kind: "local",
		displayNameKey: "providers.category.local.name",
		descriptionKey: "providers.category.local.description",
	},
] as const;

/**
 * Lookup helper — returns the catalog entry for a given `ProviderKind`.
 *
 * @throws if the kind is not in `PROVIDER_KINDS`. This is treated as
 * a caller bug (the kind is either mistyped or a backend enum value
 * the frontend hasn't been updated to recognize).
 */
export function findCatalogEntry(kind: ProviderKind): CatalogEntry {
	const entry = PROVIDER_KINDS.find((p) => p.kind === kind);
	if (!entry) {
		throw new Error(`Unknown provider kind: ${kind}`);
	}
	return entry;
}
