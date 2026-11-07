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
			return {
				...presentation,
				kind: descriptor.id as ProviderKind,
				displayNameKey: presentation.displayNameKey || "",
				label: descriptor.label,
				authTypes: descriptor.authKinds.flatMap<AuthType>((kind) =>
					kind === "oauth"
						? ["oauth"]
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
