<script setup lang="ts">
import { computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";
import ErrorBanner from "@/components/ErrorBanner.vue";
import LoadingState from "@/components/LoadingState.vue";
import PageHeader from "@/components/PageHeader.vue";
import { Button } from "@/components/ui/button";
import { useCache } from "@/composables/useCache";
import { cacheStatus } from "@/lib/cacheStatus";

const { t } = useI18n();
const { stats, signatures, selectedSignature, loading, error, fetch, inspect, clear, remove } = useCache();
const hitRate = computed(() => stats.value && stats.value.hits + stats.value.misses > 0 ? stats.value.hits / (stats.value.hits + stats.value.misses) : null);
const state = computed(() => stats.value ? cacheStatus(stats.value) : null);
async function clearAll() { if (globalThis.confirm(t("cache.clearConfirm"))) await clear(); }
async function deleteEntry(signature: string) { if (globalThis.confirm(t("cache.deleteConfirm"))) await remove(signature); }
onMounted(() => void fetch());
</script>
<template>
	<div class="space-y-6">
		<PageHeader :title="t('cache.title')" :description="t('cache.description')" />
		<ErrorBanner v-if="error" :message="error" />
		<LoadingState v-if="loading" :message="t('common.loading')" />
		<template v-else-if="stats">
			<div class="flex items-center justify-between"><p role="status">{{ t(`cache.state.${state}`) }}</p><Button data-testid="cache-clear" type="button" variant="destructive" :disabled="state === 'disabled'" @click="clearAll">{{ t("cache.clear") }}</Button></div>
			<div class="grid gap-4 md:grid-cols-4"><div class="rounded-lg border p-4"><p>{{ t("cache.hits") }}</p><strong>{{ stats.hits }}</strong></div><div class="rounded-lg border p-4"><p>{{ t("cache.misses") }}</p><strong>{{ stats.misses }}</strong></div><div class="rounded-lg border p-4"><p>{{ t("cache.hitRate") }}</p><strong>{{ hitRate == null ? t("cache.noData") : `${(hitRate * 100).toFixed(0)}%` }}</strong></div><div class="rounded-lg border p-4"><p>{{ t("cache.capacity") }}</p><strong>{{ stats.entries }} / {{ stats.max_entries || t("cache.unlimited") }}</strong></div></div>
			<section class="rounded-lg border"><h2 class="border-b p-4 font-medium">{{ t("cache.signatures") }}</h2><div v-for="entry in signatures" :key="entry.signature" class="flex flex-wrap items-center justify-between gap-3 border-b p-4 last:border-0"><div class="min-w-0"><p class="font-mono text-xs">{{ entry.signature.slice(0, 12) }}…{{ entry.signature.slice(-8) }}</p><p class="text-sm">{{ entry.request_id }} · {{ entry.provider }} / {{ entry.model }}</p><p class="text-xs text-muted-foreground">{{ t("cache.messages", { count: entry.request_metadata.message_count }) }} · {{ new Date(entry.cached_at).toLocaleString() }}</p></div><div class="flex gap-2"><Button type="button" variant="outline" @click="inspect(entry.signature)">{{ t("cache.inspect") }}</Button><Button type="button" variant="destructive" @click="deleteEntry(entry.signature)">{{ t("common.delete") }}</Button></div></div><p v-if="!signatures.length" class="p-6 text-center text-muted-foreground">{{ t("cache.noSignatures") }}</p></section>
			<section v-if="selectedSignature" class="rounded-lg border p-4"><div class="flex justify-between"><h2 class="font-medium">{{ t("cache.response") }}</h2><Button variant="outline" @click="selectedSignature = null">{{ t("common.cancel") }}</Button></div><pre class="mt-3 max-h-96 overflow-auto whitespace-pre-wrap text-xs">{{ JSON.stringify(selectedSignature, null, 2) }}</pre></section>
		</template>
	</div>
</template>
