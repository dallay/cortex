import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath, URL } from "node:url";
import { codecovVitePlugin } from "@codecov/vite-plugin";
import tailwindcss from "@tailwindcss/vite";
import vue from "@vitejs/plugin-vue";
import { defineConfig, normalizePath, type Plugin } from "vite";

// Backend target for the Vite dev server proxy. Override via the API_TARGET
// env var (e.g. set it in dev/e2e/run-api-keys-e2e.sh when the backend is on
// a non-default port like 8081).
const API_TARGET = process.env.API_TARGET ?? "http://127.0.0.1:3773";

// Base path: serve dashboard from /dashboard/ prefix
// This ensures assets are generated with correct /dashboard/assets/ paths
const BASE_PATH = process.env.BASE_PATH ?? "/dashboard/";

const SIMPLE_ICONS_IMPORT = "@iconify-json/simple-icons/icons.json";
const SIMPLE_ICONS_SUBSET_ID = "\0simple-icons-provider-subset";
const dashboardRequire = createRequire(
	fileURLToPath(new URL("./package.json", import.meta.url)),
);
const SIMPLE_ICONS_PATH = dashboardRequire.resolve(SIMPLE_ICONS_IMPORT);
const PROVIDER_ICON_PATH = normalizePath(
	fileURLToPath(new URL("./src/components/ProviderIcon.vue", import.meta.url)),
);
const PROVIDER_ICON_NAMES = ["openai", "anthropic", "ollama", "googlegemini"];

function simpleIconsProviderSubset(): Plugin {
	let moduleCode: string | undefined;

	return {
		name: "simple-icons-provider-subset",
		enforce: "pre",
		resolveId(source, importer) {
			if (
				source !== SIMPLE_ICONS_IMPORT ||
				importer?.split("?")[0] !== PROVIDER_ICON_PATH
			) {
				return null;
			}

			return SIMPLE_ICONS_SUBSET_ID;
		},
		load(id) {
			if (id !== SIMPLE_ICONS_SUBSET_ID) return null;
			if (!moduleCode) {
				const iconSet = JSON.parse(readFileSync(SIMPLE_ICONS_PATH, "utf8")) as {
					prefix: string;
					width?: number;
					height?: number;
					icons: Record<string, unknown>;
				};
				const icons = Object.fromEntries(
					PROVIDER_ICON_NAMES.map((name) => {
						const icon = iconSet.icons[name];
						if (!icon) {
							throw new Error(`Simple Icons catalog is missing ${name}`);
						}
						return [name, icon];
					}),
				);
				moduleCode = `export default ${JSON.stringify({
					prefix: iconSet.prefix,
					width: iconSet.width,
					height: iconSet.height,
					icons,
				})};`;
			}
			return moduleCode;
		},
	};
}

export default defineConfig({
	base: BASE_PATH,
	plugins: [
		simpleIconsProviderSubset(),
		vue(),
		tailwindcss(),
		// Put the Codecov vite plugin after all other plugins
		codecovVitePlugin({
			enableBundleAnalysis: process.env.CODECOV_BUNDLE_ANALYSIS === "true",
			bundleName: "rook-dashboard",
			uploadToken: process.env.CODECOV_TOKEN || undefined,
			gitService: "github",
		}),
	],
	resolve: {
		alias: {
			"@": fileURLToPath(new URL("./src", import.meta.url)),
		},
	},
	test: {
		environment: "jsdom",
		// transform these packages so vitest can handle their ESM exports
		server: {
			deps: {
				inline: [
					"class-variance-authority",
					"clsx",
					"tailwind-merge",
					"reka-ui",
				],
			},
		},
	},
	server: {
		port: 4747,
		proxy: {
			"/api/": {
				target: API_TARGET,
				changeOrigin: true,
			},
			"/health": {
				target: API_TARGET,
				changeOrigin: true,
			},
			"/login": {
				target: API_TARGET,
				changeOrigin: true,
				bypass(req) {
					// Browser navigation sends Accept: text/html — serve the SPA so
					// Vue Router handles the /login route. XHR/fetch calls (CSRF token
					// retrieval) send Accept: */* and must reach the backend.
					const accept = req.headers.accept ?? "";
					if (req.method === "GET" && accept.includes("text/html")) {
						return "/index.html";
					}
					return null;
				},
			},
			"/logout": {
				target: API_TARGET,
				changeOrigin: true,
			},
		},
	},
	build: {
		outDir: "dist",
		emptyOutDir: true,
	},
});
