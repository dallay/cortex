<script setup lang="ts">
import {onMounted, ref} from "vue";
import {useI18n} from "vue-i18n";
import EmptyState from "@/components/EmptyState.vue";
import ErrorBanner from "@/components/ErrorBanner.vue";
import LoadingState from "@/components/LoadingState.vue";
import PageHeader from "@/components/PageHeader.vue";
import {Button} from "@/components/ui/button";
import {Input} from "@/components/ui/input";
import {NativeSelect, NativeSelectOption} from "@/components/ui/native-select";
import {useModelAliases} from "@/composables/useModelAliases";
import {useProviders} from "@/composables/useProviders";
import {validateModelAlias} from "@/lib/modelAliasValidation";

const {t} = useI18n();
const {aliases, loading, saving, error, fetch, create, remove} = useModelAliases();
const {providers, fetch: fetchProviders} = useProviders();
const alias = ref("");
const canonical = ref("");
const providerId = ref("");
const issues = ref<string[]>([]);
const operationError = ref<string | null>(null);

async function submit() {
	issues.value = validateModelAlias({alias: alias.value, canonical: canonical.value}).map((issue) => issue.code);
	operationError.value = null;
	if (issues.value.length > 0) return;
	const success = await create({
		alias: alias.value.trim(),
		canonical: canonical.value.trim(),
		providerId: providerId.value || null,
	});
	if (success) {
		alias.value = "";
		canonical.value = "";
		providerId.value = "";
	} else {
		operationError.value = error.value;
	}
}

async function deleteAlias(value: string) {
	if (!globalThis.confirm(t("modelAliases.deleteConfirm", {alias: value}))) return;
	operationError.value = null;
	if (!(await remove(value))) operationError.value = error.value;
}

onMounted(() => void Promise.all([fetch(), fetchProviders()]));
</script>

<template>
	<div class="space-y-6">
		<PageHeader :title="t('modelAliases.title')" :description="t('modelAliases.description')" />
		<ErrorBanner v-if="error && !operationError" :message="error">
			<Button type="button" variant="outline" @click="fetch">{{ t("combos.retry") }}</Button>
		</ErrorBanner>
		<ErrorBanner v-if="operationError" :message="operationError" />
		<LoadingState v-if="loading && aliases.length === 0" :message="t('common.loading')" />
		<EmptyState v-else-if="aliases.length === 0" :title="t('modelAliases.empty')" />
		<div v-else class="overflow-x-auto rounded-lg border">
			<table class="w-full text-sm">
				<thead><tr class="border-b text-left"><th class="p-3">{{ t("modelAliases.alias") }}</th><th class="p-3">{{ t("modelAliases.canonical") }}</th><th class="p-3">{{ t("modelAliases.provider") }}</th><th class="p-3">{{ t("common.actions") }}</th></tr></thead>
				<tbody>
					<tr v-for="item in aliases" :key="item.alias" class="border-b last:border-0">
						<td class="p-3 font-mono">{{ item.alias }}</td><td class="p-3">{{ item.canonical }}</td><td class="p-3">{{ item.providerId ?? "—" }}</td>
						<td class="p-3"><Button :data-testid="`model-alias-delete-${item.alias}`" type="button" variant="destructive" size="sm" @click="deleteAlias(item.alias)">{{ t("common.delete") }}</Button></td>
					</tr>
				</tbody>
			</table>
		</div>
		<form data-testid="model-alias-form" class="grid gap-3 rounded-lg border p-4 md:grid-cols-4" @submit.prevent="submit">
			<div><label for="model-alias" class="text-sm font-medium">{{ t("modelAliases.alias") }}</label><Input id="model-alias" data-testid="model-alias-input" v-model="alias" :placeholder="t('modelAliases.aliasPlaceholder')" /> <p v-if="issues.includes('alias_required')" class="text-xs text-destructive">{{ t("modelAliases.errors.alias_required") }}</p><p v-if="issues.includes('reserved_combo_prefix')" class="text-xs text-destructive">{{ t("modelAliases.errors.reserved_combo_prefix") }}</p></div>
			<div><label for="model-canonical" class="text-sm font-medium">{{ t("modelAliases.canonical") }}</label><Input id="model-canonical" data-testid="model-alias-canonical" v-model="canonical" :placeholder="t('modelAliases.canonicalPlaceholder')" /><p v-if="issues.includes('canonical_required')" class="text-xs text-destructive">{{ t("modelAliases.errors.canonical_required") }}</p></div>
			<div><label for="model-provider" class="text-sm font-medium">{{ t("modelAliases.provider") }}</label><NativeSelect id="model-provider" v-model="providerId"><NativeSelectOption value="">{{ t("modelAliases.providerAny") }}</NativeSelectOption><NativeSelectOption v-for="provider in providers" :key="provider.id" :value="provider.providerRuntimeId">{{ provider.name }}</NativeSelectOption></NativeSelect></div>
			<div class="flex items-end"><Button data-testid="model-alias-create" type="submit" :disabled="saving">{{ t("modelAliases.create") }}</Button></div>
		</form>
	</div>
</template>
