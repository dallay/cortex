<script lang="ts" setup>
/**
 * ProviderCategorySection — collapsible section of the catalog view
 * grouping provider cards by category (API Key, OAuth, Local).
 *
 * Renders a section header (title + description) and a responsive grid
 * of `ProviderCatalogCard` items. If the section has no items (e.g. the
 * active category filter excludes it), the section renders nothing —
 * the parent decides whether to show a single section at a time or all
 * three.
 */
import {useI18n} from "vue-i18n";
import type {ProviderCatalogItem} from "@/composables/useProviderCatalog";
import type {CategoryDescriptor} from "@/config/providerCatalog";
import type {CodexAccountsResponse} from "@/lib/api";
import ProviderCatalogCard from "./ProviderCatalogCard.vue";

defineProps<{
  category: CategoryDescriptor;
  codexAccounts?: CodexAccountsResponse | null;
  codexAccountsLoading?: boolean;
  items: readonly ProviderCatalogItem[];
}>();

const emit = defineEmits<{
  add: [kind: ProviderCatalogItem["kind"]];
}>();

const {t} = useI18n();
</script>

<template>
  <section v-if="items.length > 0" :data-testid="`catalog-section-${category.kind}`"
           class="space-y-3">
    <div>
      <h2 class="text-lg font-semibold tracking-tight">
        {{ t(category.displayNameKey) }}
      </h2>
      <p class="text-sm text-muted-foreground">
        {{ t(category.descriptionKey) }}
      </p>
    </div>
    <div class="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
      <ProviderCatalogCard
        v-for="item in items"
        :key="item.kind"
        :codex-accounts="codexAccounts"
        :codex-accounts-loading="codexAccountsLoading"
        :item="item"
        @add="kind => emit('add', kind)"
      />
    </div>
  </section>
</template>
