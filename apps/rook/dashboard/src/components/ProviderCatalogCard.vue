<script lang="ts" setup>
/**
 * ProviderCatalogCard — single-card representation of a `ProviderKind` in
 * the catalog view.
 *
 * Two click targets:
 *   - Main card body → navigates to `/providers/:kind` (ProviderDetailsView)
 *   - "Add" button → emits `add` so the parent can open AddProviderDialog
 *     scoped to this kind. Click handler stops propagation so the card
 *     click does not also fire.
 *
 * Status badges reflect `hasActiveConnections` for model providers and the
 * linked-account state for Codex. Codex accounts are not model connections.
 * The card is purely presentational — all state-changing actions live in the
 * details view.
 *
 * The provider icon is rendered by `ProviderIcon.vue` — a branded
 * `<img src="/providers/<iconFile>">` with a Lucide `Server` fallback.
 * Catalog cards are below the fold by default so we use `loading="lazy"`.
 */

import {Plus} from "@lucide/vue";
import {computed} from "vue";
import {useI18n} from "vue-i18n";
import ProviderIcon from "@/components/ProviderIcon.vue";
import {Badge} from "@/components/ui/badge";
import {Button} from "@/components/ui/button";
import {Card, CardContent, CardHeader, CardTitle} from "@/components/ui/card";
import type {ProviderCatalogItem} from "@/composables/useProviderCatalog";
import type {CodexAccountsResponse} from "@/lib/api";

const props = defineProps<{
  codexAccounts?: CodexAccountsResponse | null;
  codexAccountsLoading?: boolean;
  item: ProviderCatalogItem;
}>();

const emit = defineEmits<{
  add: [kind: ProviderCatalogItem["kind"]];
}>();

const {t} = useI18n();

const name = computed(() => props.item.displayNameKey ? t(props.item.displayNameKey) : props.item.label);
const description = computed(() => props.item.descriptionKey ? t(props.item.descriptionKey) : props.item.label);
const isCodex = computed(() => props.item.kind === "openai-codex");
const codexAccountCount = computed(() =>
  isCodex.value && props.codexAccounts?.enabled
    ? props.codexAccounts.accounts.length
    : null,
);

const statusLabel = computed(() => {
  if (isCodex.value) {
    if (props.codexAccountsLoading) {
      return t("providers.catalog.codexChecking");
    }
    if (codexAccountCount.value === null) {
      return t("providers.catalog.codexUnavailable");
    }
    return codexAccountCount.value > 0
      ? t("providers.catalog.codexLinked")
      : t("providers.catalog.codexNotLinked");
  }
  return props.item.hasActiveConnections
    ? t("providers.catalog.active")
    : t("providers.catalog.notConfigured");
});

const statusVariant = computed(() => {
  if (isCodex.value) return "secondary";
  return props.item.hasActiveConnections ? "default" : "secondary";
});

const actionLabel = computed(() => {
  if (!isCodex.value) return t("providers.catalog.addProvider");
  return codexAccountCount.value !== null && codexAccountCount.value > 0
    ? t("providers.catalog.manageAccounts")
    : t("providers.catalog.linkAccount");
});

const detailLink = computed(() => `/providers/${props.item.kind}`);
</script>

<template>
  <Card
    :data-testid="`provider-card-${item.kind}`"
    class="transition-colors hover:bg-muted/30"
  >
    <RouterLink
      :data-testid="`provider-card-link-${item.kind}`"
      :to="detailLink"
      class="block"
    >
      <CardHeader class="flex flex-row items-start justify-between gap-3 space-y-0 pb-2">
        <div class="flex items-start gap-3 min-w-0">
          <div class="rounded-lg bg-primary/10 p-2 shrink-0">
            <ProviderIcon
              :decorative="true"
              :height="20"
              :icon-key="item.iconKey"
              :kind="item.kind"
              :width="20"
              class="text-primary"
              loading="lazy"
            />
          </div>
          <div class="min-w-0">
            <CardTitle class="text-base">{{ name }}</CardTitle>
            <p class="text-xs text-muted-foreground mt-1 line-clamp-2">
              {{ description }}
            </p>
          </div>
        </div>
        <Badge :variant="statusVariant" class="shrink-0">{{ statusLabel }}</Badge>
      </CardHeader>
      <CardContent>
        <span v-if="isCodex && !codexAccountsLoading && codexAccountCount === 0" class="block text-xs text-muted-foreground">
          {{ t('providers.catalog.codexAccountsEmpty') }}
        </span>
        <span v-else-if="isCodex && !codexAccountsLoading && codexAccountCount !== null" class="block text-xs text-muted-foreground">
          {{ t('providers.catalog.codexAccountsCount', codexAccountCount) }}
        </span>
        <span v-else-if="isCodex && !codexAccountsLoading" class="block text-xs text-muted-foreground">
          {{ t('providers.catalog.codexAccountsUnavailable') }}
        </span>
        <span v-if="!isCodex" class="block text-xs text-muted-foreground">
          {{ t('providers.catalog.connectionsCount', item.connectionCount) }}
        </span>
      </CardContent>
    </RouterLink>
    <div class="px-6 pb-6 pt-0 flex items-center gap-2">
      <Button
        :data-testid="`provider-card-add-${item.kind}`"
        class="flex-1"
        size="sm"
        variant="outline"
        @click.stop.prevent="emit('add', item.kind)"
      >
        <Plus class="h-4 w-4 mr-1"/>
        {{ actionLabel }}
      </Button>
    </div>
  </Card>
</template>
