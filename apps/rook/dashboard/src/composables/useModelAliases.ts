import { ref } from "vue";
import type { CreateModelAliasRequest, ModelAliasResponse } from "@/lib/api";
import { useApi } from "@/lib/api";

export function useModelAliases() {
	const api = useApi();
	const aliases = ref<ModelAliasResponse[]>([]);
	const loading = ref(false);
	const saving = ref(false);
	const error = ref<string | null>(null);

	async function fetch(): Promise<void> {
		loading.value = true;
		error.value = null;
		try {
			aliases.value = await api.getModelAliases();
		} catch (cause) {
			error.value =
				cause instanceof Error ? cause.message : "Failed to load model aliases";
		} finally {
			loading.value = false;
		}
	}

	async function create(data: CreateModelAliasRequest): Promise<boolean> {
		saving.value = true;
		error.value = null;
		try {
			await api.createModelAlias(data);
			await fetch();
			return true;
		} catch (cause) {
			error.value =
				cause instanceof Error ? cause.message : "Failed to create model alias";
			return false;
		} finally {
			saving.value = false;
		}
	}

	async function remove(alias: string): Promise<boolean> {
		saving.value = true;
		error.value = null;
		try {
			await api.deleteModelAlias(alias);
			aliases.value = aliases.value.filter((item) => item.alias !== alias);
			return true;
		} catch (cause) {
			error.value =
				cause instanceof Error ? cause.message : "Failed to delete model alias";
			return false;
		} finally {
			saving.value = false;
		}
	}

	return { aliases, loading, saving, error, fetch, create, remove };
}
