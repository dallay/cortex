<script setup lang="ts">
/**
 * ProviderCatalogFilter — search input + category filter chips.
 *
 * Pure controlled component. Emits updates on every keystroke / chip
 * change. Parent owns the filter state and derives the visible list.
 *
 * Categories are derived from `CATEGORIES` plus an "All" sentinel
 * (rendered first). The filter selection is represented as a
 * `CategoryKind | 'all'` discriminated string so the parent can
 * early-out when `'all'` is selected.
 */

import {Search} from "@lucide/vue";
import {computed} from "vue";
import {useI18n} from "vue-i18n";
import {Input} from "@/components/ui/input";
import {CATEGORIES, type CategoryKind} from "@/config/providerCatalog";

export type CategoryFilter = CategoryKind | "all";

const props = defineProps<{
  searchQuery: string;
  activeCategory: CategoryFilter;
}>();

const emit = defineEmits<{
  "update:searchQuery": [value: string];
  "update:activeCategory": [value: CategoryFilter];
}>();

const {t} = useI18n();

const chips = computed(() => [
  {value: "all" as const, label: t("providers.catalog.filterAll")},
  ...CATEGORIES.map((c) => ({value: c.kind, label: t(c.displayNameKey)})),
]);

function onSearchInput(event: Event) {
  const target = event.target as HTMLInputElement;
  emit("update:searchQuery", target.value);
}
</script>

<template>
  <div class="space-y-3">
    <div class="relative">
      <Search class="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-muted-foreground" />
      <Input
        :model-value="searchQuery"
        :placeholder="t('providers.catalog.searchPlaceholder')"
        class="pl-9"
        :data-testid="'catalog-search'"
        @input="onSearchInput"
      />
    </div>
    <div class="flex flex-wrap gap-2" role="tablist">
      <button
        v-for="chip in chips"
        :key="chip.value"
        type="button"
        role="tab"
        :aria-selected="activeCategory === chip.value"
        :data-testid="`catalog-filter-${chip.value}`"
        :class="[
          'px-3 py-1 text-xs font-medium rounded-full border transition-colors',
          activeCategory === chip.value
            ? 'bg-primary text-primary-foreground border-primary'
            : 'bg-background text-muted-foreground hover:bg-muted/50',
        ]"
        @click="emit('update:activeCategory', chip.value)"
      >
        {{ chip.label }}
      </button>
    </div>
  </div>
</template>
