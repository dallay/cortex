import { computed, ref } from "vue";
import {
	type AuthType,
	findCatalogEntry,
	type ProviderKind,
} from "@/config/providerCatalog";
import { type ProviderDescriptorResponse, useApi } from "@/lib/api";

const descriptors = ref<ProviderDescriptorResponse[]>([]);
const loading = ref(false);
const error = ref<string | null>(null);

export function useProviderDescriptors() {
	async function fetch() {
		loading.value = true;
		error.value = null;
		try {
			descriptors.value = await useApi().getProviderDescriptors();
		} catch (cause) {
			error.value =
				cause instanceof Error
					? cause.message
					: "Failed to fetch provider descriptors";
		} finally {
			loading.value = false;
		}
	}

	const catalog = computed(() =>
		descriptors.value.map((descriptor) => {
			const presentation = findCatalogEntry(descriptor.id);
			// Prefer the backend category; fall back to the static catalog entry.
			const category = (descriptor.category ||
				presentation.category) as import("@/config/providerCatalog").CategoryKind;
			return {
				...presentation,
				kind: descriptor.id as ProviderKind,
				category,
				displayNameKey: presentation.displayNameKey || "",
				label: descriptor.label,
				supportsOAuth: descriptor.supportsOAuth,
				authTypes: descriptor.authKinds.flatMap<AuthType>((kind) =>
					kind === "oauth"
						? descriptor.supportsOAuth
							? ["oauth"]
							: []
						: kind === "apiKey" || kind === "importedToken"
							? ["apikey"]
							: [],
				),
				iconFile: presentation.iconFile,
				iconKey: descriptor.iconKey,
			};
		}),
	);

	return { descriptors, catalog, loading, error, fetch };
}
