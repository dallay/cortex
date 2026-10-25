<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import ErrorBanner from "@/components/ErrorBanner.vue";
import LoadingState from "@/components/LoadingState.vue";
import PageHeader from "@/components/PageHeader.vue";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useUsage } from "@/composables/useUsage";

const { t } = useI18n();
const { entries, total, summary, cost, costStatus, loading, error, availability, query, fetch } = useUsage();
const provider = ref("");
const model = ref("");
const apiKeyId = ref("");
const start = ref("");
const end = ref("");
const pageSize = 25;
const pageCount = computed(() => Math.max(1, Math.ceil(total.value / pageSize)));
const costValue = computed(() => {
	if (costStatus.value === "no_data" || costStatus.value === "unknown") return null;
	return cost.value?.total_cost_usd ?? null;
});

function applyFilters() {
	void fetch({ provider: provider.value || undefined, model: model.value || undefined, apiKeyId: apiKeyId.value || undefined, start: start.value ? new Date(start.value).toISOString() : undefined, end: end.value ? new Date(end.value).toISOString() : undefined, offset: 0, limit: pageSize });
}
function changePage(offset: number) {
	void fetch({ ...query.value, offset, limit: pageSize });
}
function currency(value: number | null | undefined) {
	return value == null ? t("usage.noData") : new Intl.NumberFormat(undefined, { style: "currency", currency: "USD", maximumFractionDigits: 4 }).format(value);
}
onMounted(() => void fetch());
</script>

<template>
	<div class="space-y-6">
		<PageHeader :title="t('usage.title')" :description="t('usage.description')" />
		<div v-if="availability === 'unavailable'" class="rounded-lg border bg-muted p-4" role="status">{{ t("usage.unavailable") }}</div>
		<ErrorBanner v-else-if="error" :message="error" />
		<div v-if="summary && availability === 'available'" class="grid gap-4 md:grid-cols-4">
			<div class="rounded-lg border p-4"><p class="text-sm text-muted-foreground">{{ t("usage.requests") }}</p><p class="text-2xl font-semibold">{{ summary.total_requests }}</p></div>
			<div class="rounded-lg border p-4"><p class="text-sm text-muted-foreground">{{ t("usage.tokens") }}</p><p class="text-2xl font-semibold">{{ (summary.total_prompt_tokens + summary.total_completion_tokens).toLocaleString() }}</p></div>
			<div class="rounded-lg border p-4"><p class="text-sm text-muted-foreground">{{ t("usage.averageLatency") }}</p><p class="text-2xl font-semibold">{{ summary.avg_latency_ms }} ms</p></div>
			<div class="rounded-lg border p-4"><p class="text-sm text-muted-foreground">{{ t("usage.totalCost") }}</p><p class="text-2xl font-semibold">{{ currency(costValue) }}</p><p v-if="costStatus === 'partial'" class="text-xs text-muted-foreground">{{ t("usage.partialCost", { count: cost?.unknown_cost_entries }) }}</p><p v-else-if="costStatus === 'unknown'" class="text-xs text-muted-foreground">{{ t("usage.unknownCost") }}</p></div>
		</div>
		<div class="rounded-lg border p-4"><h2 class="mb-3 font-medium">{{ t("usage.filters") }}</h2><form class="grid gap-3 md:grid-cols-5" @submit.prevent="applyFilters"><Input v-model="provider" :placeholder="t('usage.provider')" /><Input v-model="model" :placeholder="t('usage.model')" /><Input v-model="apiKeyId" :placeholder="t('usage.apiKey')" /><label class="text-xs">{{ t("usage.start") }}<input v-model="start" type="datetime-local" class="h-9 w-full rounded-md border bg-transparent px-3 text-sm" /></label><label class="text-xs">{{ t("usage.end") }}<input v-model="end" type="datetime-local" class="h-9 w-full rounded-md border bg-transparent px-3 text-sm" /></label><Button type="submit" class="md:col-span-5 md:justify-self-end">{{ t("common.search") }}</Button></form></div>
		<LoadingState v-if="loading" :message="t('common.loading')" />
		<div v-else-if="availability === 'available'" class="space-y-3">
			<div class="overflow-x-auto rounded-lg border"><table class="w-full text-sm"><thead><tr class="border-b text-left"><th class="p-3">{{ t("usage.time") }}</th><th class="p-3">{{ t("usage.provider") }}</th><th class="p-3">{{ t("usage.model") }}</th><th class="p-3">{{ t("usage.status") }}</th><th class="p-3">{{ t("usage.tokens") }}</th><th class="p-3">{{ t("usage.cost") }}</th><th class="p-3">{{ t("usage.latency") }}</th></tr></thead><tbody><tr v-for="entry in entries" :key="entry.request_id" class="border-b last:border-0"><td class="p-3">{{ new Date(entry.timestamp).toLocaleString() }}</td><td class="p-3">{{ entry.provider }}</td><td class="p-3">{{ entry.model }}</td><td class="p-3">{{ entry.status }}</td><td class="p-3">{{ ((entry.tokens_prompt ?? 0) + (entry.tokens_completion ?? 0)).toLocaleString() }}</td><td class="p-3">{{ currency(entry.cost_usd) }}</td><td class="p-3">{{ entry.latency_ms }} ms</td></tr><tr v-if="entries.length === 0"><td colspan="7" class="p-8 text-center text-muted-foreground">{{ t("usage.noData") }}</td></tr></tbody></table></div>
			<div class="flex items-center justify-between text-sm"><span>{{ t("usage.totalEntries", { count: total }) }}</span><div class="flex gap-2"><Button type="button" variant="outline" :disabled="(query.offset ?? 0) === 0" @click="changePage(Math.max(0, (query.offset ?? 0) - pageSize))">{{ t("common.previous") }}</Button><span class="px-2 py-2">{{ t("usage.page", { current: Math.floor((query.offset ?? 0) / pageSize) + 1, total: pageCount }) }}</span><Button type="button" variant="outline" :disabled="(query.offset ?? 0) + pageSize >= total" @click="changePage((query.offset ?? 0) + pageSize)">{{ t("common.next") }}</Button></div></div>
		</div>
		<div v-if="cost && costStatus !== 'no_data'" class="grid gap-4 md:grid-cols-2"><section class="rounded-lg border p-4"><h2 class="mb-3 font-medium">{{ t("usage.byProvider") }}</h2><p v-for="[name, value] in Object.entries(cost.by_provider)" :key="name" class="flex justify-between border-b py-2 last:border-0"><span>{{ name }}</span><span>{{ currency(value) }}</span></p><p v-if="!Object.keys(cost.by_provider).length" class="text-sm text-muted-foreground">{{ t("usage.noData") }}</p></section><section class="rounded-lg border p-4"><h2 class="mb-3 font-medium">{{ t("usage.byModel") }}</h2><p v-for="[name, value] in Object.entries(cost.by_model)" :key="name" class="flex justify-between border-b py-2 last:border-0"><span>{{ name }}</span><span>{{ currency(value) }}</span></p><p v-if="!Object.keys(cost.by_model).length" class="text-sm text-muted-foreground">{{ t("usage.noData") }}</p></section></div>
	</div>
</template>
