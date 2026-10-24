<script setup lang="ts">
import { onMounted } from "vue";
import { useI18n } from "vue-i18n";
import ErrorBanner from "@/components/ErrorBanner.vue";
import LoadingState from "@/components/LoadingState.vue";
import PageHeader from "@/components/PageHeader.vue";
import { Button } from "@/components/ui/button";
import { formatMilliseconds, useTelemetry } from "@/composables/useTelemetry";

const { t } = useI18n();
const { providers, provider, latency, loading, error, availability, fetch, inspect } = useTelemetry();
onMounted(() => void fetch());
</script>
<template>
	<div class="space-y-6">
		<PageHeader :title="t('telemetry.title')" :description="t('telemetry.description')" />
		<div v-if="availability === 'unavailable'" role="status" class="rounded-lg border bg-muted p-4">{{ t("telemetry.unavailable") }}</div>
		<ErrorBanner v-else-if="error" :message="error" />
		<LoadingState v-if="loading" :message="t('common.loading')" />
		<div v-else-if="availability === 'available'" class="overflow-x-auto rounded-lg border"><table class="w-full text-sm"><thead><tr class="border-b text-left"><th class="p-3">{{ t("telemetry.provider") }}</th><th class="p-3">{{ t("telemetry.requests") }}</th><th class="p-3">{{ t("telemetry.success") }}</th><th class="p-3">{{ t("telemetry.failures") }}</th><th class="p-3">p50</th><th class="p-3">p95</th><th class="p-3">p99</th><th class="p-3"></th></tr></thead><tbody><tr v-for="item in providers" :key="item.provider_id" class="border-b"><td class="p-3">{{ item.provider_id }}</td><td class="p-3">{{ item.total_requests }}</td><td class="p-3">{{ item.success_count }}</td><td class="p-3">{{ item.failure_count + item.rate_limited_count }}</td><td class="p-3">{{ formatMilliseconds(item.latency_stats?.p50) }}</td><td class="p-3">{{ formatMilliseconds(item.latency_stats?.p95) }}</td><td class="p-3">{{ formatMilliseconds(item.latency_stats?.p99) }}</td><td class="p-3"><Button variant="outline" @click="inspect(item.provider_id)">{{ t("telemetry.details") }}</Button></td></tr><tr v-if="providers.length === 0"><td colspan="8" class="p-6 text-center text-muted-foreground">{{ t("telemetry.empty") }}</td></tr></tbody></table></div>
		<section v-if="provider && latency" class="grid gap-3 rounded-lg border p-4 md:grid-cols-4"><h2 class="font-medium md:col-span-4">{{ provider.provider_id }} · {{ t("telemetry.latencyDistribution") }} ({{ latency.count }} {{ t("telemetry.samples") }})</h2><p>p50: {{ formatMilliseconds(latency.p50) }}</p><p>p95: {{ formatMilliseconds(latency.p95) }}</p><p>p99: {{ formatMilliseconds(latency.p99) }}</p><p>{{ t("telemetry.minMax", { min: formatMilliseconds(latency.min), max: formatMilliseconds(latency.max) }) }}</p></section>
	</div>
</template>
