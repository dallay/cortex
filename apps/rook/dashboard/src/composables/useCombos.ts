/**
 * Combos composable — CRUD for fallback chains of provider/model steps.
 *
 * Mirrors the shape of `useApiKeys`: a reactive list plus `loading`, `error`
 * and per-operation helpers that never throw. Callers decide what to show
 * when an operation returns `null` / `false`.
 *
 * `create` refreshes the list because the backend assigns the id and
 * timestamps; `update` and `remove` patch the local list to avoid an extra
 * round-trip.
 */
import { ref } from "vue";
import {
	type ComboAliasResponse,
	type ComboResponse,
	type CreateComboRequest,
	type UpdateComboRequest,
	useApi,
} from "@/lib/api";

export function useCombos() {
	const api = useApi();
	const combos = ref<ComboResponse[]>([]);
	const aliasesByComboId = ref<Record<string, ComboAliasResponse[]>>({});
	const aliasError = ref<Record<string, string | null>>({});
	const loading = ref(false);
	const saving = ref(false);
	const error = ref<string | null>(null);

	async function fetch(): Promise<void> {
		loading.value = true;
		error.value = null;
		try {
			const response = await api.getCombos();
			combos.value = response.combos;
		} catch (e) {
			error.value = e instanceof Error ? e.message : "Failed to fetch combos";
			console.error("[useCombos]", e);
		} finally {
			loading.value = false;
		}
	}

	async function create(
		data: CreateComboRequest,
	): Promise<ComboResponse | null> {
		saving.value = true;
		error.value = null;
		try {
			const created = await api.createCombo(data);
			await fetch();
			return created;
		} catch (e) {
			error.value = e instanceof Error ? e.message : "Failed to create combo";
			return null;
		} finally {
			saving.value = false;
		}
	}

	async function update(
		id: string,
		data: UpdateComboRequest,
	): Promise<ComboResponse | null> {
		saving.value = true;
		error.value = null;
		try {
			const updated = await api.updateCombo(id, data);
			const index = combos.value.findIndex((combo) => combo.id === id);
			if (index === -1) {
				combos.value.push(updated);
			} else {
				combos.value[index] = updated;
			}
			return updated;
		} catch (e) {
			error.value = e instanceof Error ? e.message : "Failed to update combo";
			return null;
		} finally {
			saving.value = false;
		}
	}

	async function fetchAliases(comboId: string): Promise<void> {
		aliasError.value[comboId] = null;
		try {
			aliasesByComboId.value[comboId] = await api.getComboAliases(comboId);
		} catch (e) {
			aliasError.value[comboId] =
				e instanceof Error ? e.message : "Failed to fetch combo aliases";
		}
	}

	async function createAlias(
		comboId: string,
		alias: string,
	): Promise<ComboAliasResponse | null> {
		aliasError.value[comboId] = null;
		try {
			const created = await api.createComboAlias(comboId, alias);
			const aliases = aliasesByComboId.value[comboId] ?? [];
			aliasesByComboId.value[comboId] = [...aliases, created];
			return created;
		} catch (e) {
			aliasError.value[comboId] =
				e instanceof Error ? e.message : "Failed to create combo alias";
			return null;
		}
	}

	async function removeAlias(comboId: string, alias: string): Promise<boolean> {
		aliasError.value[comboId] = null;
		try {
			await api.deleteComboAlias(comboId, alias);
			aliasesByComboId.value[comboId] = (
				aliasesByComboId.value[comboId] ?? []
			).filter((item) => item.alias !== alias);
			return true;
		} catch (e) {
			aliasError.value[comboId] =
				e instanceof Error ? e.message : "Failed to delete combo alias";
			return false;
		}
	}

	async function remove(id: string): Promise<boolean> {
		saving.value = true;
		error.value = null;
		try {
			await api.deleteCombo(id);
			combos.value = combos.value.filter((combo) => combo.id !== id);
			return true;
		} catch (e) {
			error.value = e instanceof Error ? e.message : "Failed to delete combo";
			return false;
		} finally {
			saving.value = false;
		}
	}

	return {
		combos,
		aliasesByComboId,
		aliasError,
		loading,
		saving,
		error,
		fetch,
		create,
		update,
		remove,
		fetchAliases,
		createAlias,
		removeAlias,
	};
}
