/**
 * Navigation configuration — declarative, easy to extend.
 *
 * Adding a new menu item:
 * 1. Add entry to `main` or `secondary` array
 * 2. Add i18n key to locales/en.json and locales/es.json
 * 3. Icon must be registered in useNavigation.ts iconRegistry
 */
export interface NavSubItem {
	titleKey: string;
	url: string;
}

export interface NavItem {
	titleKey: string;
	url: string;
	icon: string;
	items?: NavSubItem[];
	isActive?: boolean;
}

export interface NavSection {
	items: NavItem[];
}

export const navigationConfig = {
	main: [
		{
			titleKey: "nav.home",
			url: "/",
			icon: "Home",
			isActive: true,
		},
		{
			titleKey: "nav.apiKeys",
			url: "/api-keys",
			icon: "Key",
			items: [
				{ titleKey: "nav.apiKeysList", url: "/api-keys" },
				{ titleKey: "nav.apiKeysCreate", url: "/api-keys/new" },
			],
		},
		{
			titleKey: "nav.endpoints",
			url: "/endpoints",
			icon: "ListOrdered",
		},
		{
			titleKey: "nav.providers",
			url: "/providers",
			icon: "Globe",
			items: [
				{ titleKey: "nav.providersCatalog", url: "/providers" },
				{ titleKey: "nav.providersQuota", url: "/providers/quota" },
			],
		},
		{
			titleKey: "nav.combos",
			url: "/combos",
			icon: "CircleDot",
		},
		{
			titleKey: "nav.modelAliases",
			url: "/model-aliases",
			icon: "Tags",
		},
		{
			titleKey: "nav.usage",
			url: "/usage",
			icon: "ChartNoAxesCombined",
		},
		{
			titleKey: "nav.rateLimits",
			url: "/rate-limits",
			icon: "Gauge",
		},
		{
			titleKey: "nav.cache",
			url: "/cache",
			icon: "Database",
		},
		{
			titleKey: "nav.telemetry",
			url: "/telemetry",
			icon: "Activity",
		},
		{
			titleKey: "nav.resilience",
			url: "/resilience",
			icon: "ShieldCheck",
		},
		{
			titleKey: "nav.settings",
			url: "/settings",
			icon: "Settings",
		},
	] satisfies NavItem[],

	secondary: [
		{ titleKey: "nav.support", url: "#", icon: "LifeBuoy" },
		{
			titleKey: "nav.issues",
			url: "https://github.com/dallay/cortex/issues",
			icon: "FileSliders",
		},
		{ titleKey: "nav.feedback", url: "#", icon: "Send" },
	] satisfies NavItem[],
} as const;
