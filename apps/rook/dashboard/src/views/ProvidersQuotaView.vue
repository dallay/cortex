<script setup lang="ts">
import {Fuel} from "@lucide/vue";
import {computed, onMounted} from "vue";
import {useI18n} from "vue-i18n";
import {Alert, AlertDescription, AlertTitle} from "@/components/ui/alert";
import {Badge} from "@/components/ui/badge";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {PROVIDER_KINDS} from "@/config/providerCatalog";
import {useProvidersQuota} from "@/composables/useProvidersQuota";

const {t, d, n} = useI18n();
const {items, generatedAt, loading, error, fetch} = useProvidersQuota();

onMounted(() => {
  fetch();
});

const UNKNOWN_PROVIDER_FALLBACK = {
  kind: "unknown",
  displayNameKey: "providers.quota.unknownProvider",
  category: "api-key" as const,
  defaultBaseUrl: "",
  defaultModels: [],
  authTypes: ["apikey"] as const,
  descriptionKey: "providers.quota.unknownProvider",
};

function safeFindCatalogEntry(kind: string) {
  return (
    PROVIDER_KINDS.find((p) => p.kind === kind) ??
    ({...UNKNOWN_PROVIDER_FALLBACK, kind})
  );
}

const rows = computed(() =>
  items.value.map((item) => {
    const catalog = safeFindCatalogEntry(item.providerKind);
    const statusVariant: "default" | "destructive" | "outline" | "secondary" =
      item.warningLevel === "critical"
        ? "destructive"
        : item.warningLevel === "warning"
          ? "secondary"
          : item.warningLevel === "not_configured"
            ? "outline"
            : "default";

    const isKnown = PROVIDER_KINDS.some((p) => p.kind === item.providerKind);

    return {
      ...item,
      displayName: isKnown ? t(catalog.displayNameKey) : item.providerKind,
      statusVariant,
      usagePercent:
        item.warningThreshold && item.warningThreshold > 0
          ? Math.round((item.observedRateLimitedRatio / item.warningThreshold) * 100)
          : null,
    };
  }),
);

function formatUsd(value: number): string {
  return n(value, {style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 4});
}

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}
</script>

<template>
  <div class="space-y-6">
    <div>
      <h1 class="text-2xl font-semibold tracking-tight flex items-center gap-2">
        <Fuel class="h-6 w-6 text-primary" />
        {{ t('providers.quota.title') }}
      </h1>
      <p class="text-muted-foreground">
        {{ t('providers.quota.subtitle') }}
      </p>
      <p v-if="generatedAt" class="text-xs text-muted-foreground mt-1">
        {{ t('providers.quota.generatedAt', { date: d(new Date(generatedAt), 'short') }) }}
      </p>
    </div>

    <Alert v-if="error" variant="destructive">
      <AlertTitle>{{ t('common.error') }}</AlertTitle>
      <AlertDescription>{{ error }}</AlertDescription>
    </Alert>

    <div class="rounded-md border">
      <Table>
        <TableHeader>
          <TableRow class="bg-muted/50">
            <TableHead class="px-4 py-3 text-left text-sm font-medium">
              {{ t('providers.name') }}
            </TableHead>
            <TableHead class="px-4 py-3 text-left text-sm font-medium">
              {{ t('providers.quota.support') }}
            </TableHead>
            <TableHead class="px-4 py-3 text-right text-sm font-medium">
              {{ t('providers.quota.last24hTokens') }}
            </TableHead>
            <TableHead class="px-4 py-3 text-right text-sm font-medium">
              {{ t('providers.quota.last7dCost') }}
            </TableHead>
            <TableHead class="px-4 py-3 text-right text-sm font-medium">
              {{ t('providers.quota.rateLimited') }}
            </TableHead>
            <TableHead class="px-4 py-3 text-right text-sm font-medium">
              {{ t('providers.status') }}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          <template v-if="loading && rows.length === 0">
            <TableRow>
              <TableCell colspan="6" class="px-4 py-8 text-center text-sm text-muted-foreground">
                {{ t('common.loading') }}
              </TableCell>
            </TableRow>
          </template>
          <template v-else>
            <TableRow
              v-for="item in rows"
              :key="item.providerKind"
              class="hover:bg-muted/30 align-top"
              :data-testid="`quota-row-${item.providerKind}`"
            >
              <TableCell class="px-4 py-3">
                <div class="font-medium">{{ item.displayName }}</div>
                <div class="text-xs text-muted-foreground">
                  {{ t('providers.quota.connections', { configured: item.connectionCount, active: item.activeConnectionCount }) }}
                </div>
              </TableCell>
              <TableCell class="px-4 py-3 text-sm text-muted-foreground max-w-sm">
                <div>{{ item.note }}</div>
              </TableCell>
              <TableCell class="px-4 py-3 text-right font-mono text-xs">
                {{ item.last24h.totalTokens.toLocaleString() }}
              </TableCell>
              <TableCell class="px-4 py-3 text-right font-mono text-xs">
                {{ formatUsd(item.last7d.costUsd) }}
              </TableCell>
              <TableCell class="px-4 py-3 text-right">
                <div class="font-mono text-xs">{{ formatPercent(item.observedRateLimitedRatio) }}</div>
                <div class="text-[11px] text-muted-foreground">
                  {{ item.last7d.rateLimitedRequests }}/{{ item.last7d.requests }}
                </div>
              </TableCell>
              <TableCell class="px-4 py-3 text-right space-y-2">
                <Badge :variant="item.statusVariant">
                  {{ t(`providers.quota.level.${item.warningLevel}`) }}
                </Badge>
                <div class="text-[11px] text-muted-foreground">
                  {{ t(`providers.quota.supportValue.${item.support}`) }}
                </div>
              </TableCell>
            </TableRow>
          </template>
        </TableBody>
      </Table>
    </div>
  </div>
</template>
