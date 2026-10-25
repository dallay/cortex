<script setup lang="ts">
import {Fuel, RefreshCw} from "@lucide/vue";
import {computed, onMounted} from "vue";
import {useI18n} from "vue-i18n";
import {Alert, AlertDescription, AlertTitle} from "@/components/ui/alert";
import {Badge} from "@/components/ui/badge";
import {Button} from "@/components/ui/button";
import {Table, TableBody, TableCell, TableHead, TableHeader, TableRow} from "@/components/ui/table";
import {useProvidersQuota} from "@/composables/useProvidersQuota";
import {PROVIDER_KINDS} from "@/config/providerCatalog";

const {t, d, n} = useI18n();
const {items, generatedAt, loading, error, stale, fetch} = useProvidersQuota();
const cellCount = 6;
onMounted(() => void fetch());

function statusVariant(level: (typeof items.value)[number]["localWarningLevel"]) {
  switch (level) {
    case "critical":
      return "destructive" as const;
    case "warning":
      return "secondary" as const;
    case "not_configured":
      return "outline" as const;
    default:
      return "default" as const;
  }
}

const rows = computed(() => items.value.map((item) => {
  const catalog = PROVIDER_KINDS.find((provider) => provider.kind === item.providerKind);
  return {
    ...item,
    displayName: catalog ? t(catalog.displayNameKey) : item.providerKind,
    statusVariant: statusVariant(item.localWarningLevel),
  };
}));
function formatUsd(value: number | null): string {
  return value === null ? t("providers.quota.unknownCost") : n(value, {style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 4});
}
function availabilityLabel(value: "available" | "no_data" | "unavailable"): string {
  return t(`providers.quota.availability.${value}`);
}
function last24hLabel(item: (typeof items.value)[number]): string {
  const availability = item.last24h.availability;
  if (availability === "unavailable") return t("providers.quota.unavailable");
  if (availability === "no_data") return availabilityLabel("no_data");
  return n(item.last24h.totalTokens ?? 0);
}
function formatPercent(value: number | null): string {
  return value === null ? t("providers.quota.unavailable") : `${(value * 100).toFixed(1)}%`;
}
</script>

<template>
  <div class="space-y-6">
    <div class="flex items-start justify-between gap-4">
      <div>
        <h1 class="flex items-center gap-2 text-2xl font-semibold tracking-tight"><Fuel class="h-6 w-6 text-primary" />{{ t('providers.quota.title') }}</h1>
        <p class="text-muted-foreground">{{ t('providers.quota.subtitle') }}</p>
        <p class="mt-1 text-xs text-muted-foreground">{{ t('providers.quota.coverage') }}</p>
        <p v-if="generatedAt" class="mt-1 text-xs text-muted-foreground">{{ t('providers.quota.generatedAt', {date: d(new Date(generatedAt), 'short')}) }}</p>
      </div>
      <Button variant="outline" :disabled="loading" @click="fetch"><RefreshCw class="mr-2 h-4 w-4" />{{ t('providers.quota.refresh') }}</Button>
    </div>

    <Alert v-if="error" :variant="stale ? 'default' : 'destructive'" data-testid="quota-refresh-state">
      <AlertTitle>{{ stale ? t('providers.quota.stale') : t('common.error') }}</AlertTitle>
      <AlertDescription>{{ stale ? t('providers.quota.staleDescription') : error }}</AlertDescription>
    </Alert>

    <div class="rounded-md border">
      <Table>
        <TableHeader><TableRow class="bg-muted/50">
          <TableHead>{{ t('providers.name') }}</TableHead><TableHead>{{ t('providers.quota.coverageHeading') }}</TableHead>
          <TableHead class="text-right">{{ t('providers.quota.last24hTokens') }}</TableHead><TableHead class="text-right">{{ t('providers.quota.last7dCost') }}</TableHead>
          <TableHead class="text-right">{{ t('providers.quota.rateLimited') }}</TableHead><TableHead class="text-right">{{ t('providers.status') }}</TableHead>
        </TableRow></TableHeader>
        <TableBody>
          <TableRow v-if="loading && rows.length === 0"><TableCell :colspan="cellCount" class="py-8 text-center">{{ t('common.loading') }}</TableCell></TableRow>
          <TableRow v-if="rows.some((item) => item.usageAvailability === 'unavailable')"><TableCell :colspan="cellCount" class="text-sm text-muted-foreground">{{ t('providers.quota.usageUnavailable') }}</TableCell></TableRow>
          <template v-for="item in rows" :key="item.providerKind">
          <TableRow class="align-top hover:bg-muted/30" :data-testid="`quota-row-${item.providerKind}`">
            <TableCell><div class="font-medium">{{ item.displayName }}</div><div class="text-xs text-muted-foreground">{{ t('providers.quota.connections', {configured: item.connectionCount, active: item.activeConnectionCount}) }}</div></TableCell>
            <TableCell class="max-w-sm text-sm text-muted-foreground"><div>{{ t('providers.quota.cortexCoverage') }}</div><div>{{ t(item.accountQuota.availability === 'not_applicable' ? 'providers.quota.accountNotApplicable' : 'providers.quota.accountNotProvided') }}</div><div v-for="threshold in item.localThresholds" :key="`${threshold.warning}-${threshold.error}`">{{ t('providers.quota.localThreshold', {warning: formatPercent(threshold.warning), error: formatPercent(threshold.error), count: threshold.connectionCount}) }}</div></TableCell>
            <TableCell class="text-right font-mono text-xs">{{ last24hLabel(item) }}</TableCell>
            <TableCell class="text-right font-mono text-xs"><span>{{ formatUsd(item.last7d.costAvailability === 'complete' ? item.last7d.costUsd : null) }}</span><span class="block text-[11px] text-muted-foreground">{{ t('providers.quota.estimatedCost') }}</span><span v-if="item.last7d.costAvailability === 'partial'" class="block text-[11px] text-muted-foreground">{{ t('providers.quota.partialCost') }}</span></TableCell>
            <TableCell class="text-right"><div class="font-mono text-xs">{{ formatPercent(item.rateLimitObservation.ratio) }}</div><div class="text-[11px] text-muted-foreground">{{ item.rateLimitObservation.rateLimitedRequests === null ? '—' : n(item.rateLimitObservation.rateLimitedRequests) }}/{{ item.rateLimitObservation.requests === null ? '—' : n(item.rateLimitObservation.requests) }}</div></TableCell>
            <TableCell class="space-y-2 text-right"><Badge :variant="item.statusVariant">{{ t(`providers.quota.level.${item.localWarningLevel}`) }}</Badge><div class="text-[11px] text-muted-foreground">{{ t('providers.quota.observedRateLimits') }}</div></TableCell>
          </TableRow>
          <TableRow>
            <TableCell :colspan="cellCount" class="p-0"><section :data-testid="`quota-trend-${item.providerKind}`" class="grid grid-cols-2 gap-2 px-4 pb-3 sm:grid-cols-4 lg:grid-cols-7" :aria-label="t('providers.quota.trendLabel', {provider: item.displayName})"><div v-for="point in item.trend" :key="point.date" data-testid="quota-trend-day" class="min-w-0 border-t-2 border-primary/60 pt-1 text-center text-[10px] text-muted-foreground"><span class="block">{{ point.date }}</span><span class="block font-mono">{{ point.requests === null ? '—' : n(point.requests) }} {{ t('providers.quota.requests') }}</span><span class="block font-mono">{{ point.totalTokens === null ? '—' : n(point.totalTokens) }} {{ t('providers.quota.tokens') }}</span><span v-if="point.costUsd !== null" class="block font-mono">{{ t('providers.quota.trendCost') }}: {{ formatUsd(point.costUsd) }}</span><span v-else class="block">{{ t('providers.quota.trendCost') }}: {{ formatUsd(null) }}</span><span v-if="point.costAvailability === 'partial'" class="block">{{ t('providers.quota.partialCost') }}</span><span v-else-if="point.costAvailability === 'unknown'" class="block">{{ t('providers.quota.unknownCost') }}</span></div></section></TableCell>
          </TableRow>
          </template>
          <TableRow v-if="!loading && rows.length === 0"><TableCell :colspan="cellCount" class="py-8 text-center text-muted-foreground">{{ error || t('providers.quota.empty') }}</TableCell></TableRow>
        </TableBody>
      </Table>
    </div>
  </div>
</template>
