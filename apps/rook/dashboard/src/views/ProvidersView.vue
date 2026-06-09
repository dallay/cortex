<script setup lang="ts">
/**
 * ProvidersView — catalog of all supported providers.
 *
 * Replaces the legacy two-tab list/quotes layout. The "Quotes" tab
 * moves to its own dedicated route (`/providers/quota` → Phase 6),
 * keeping this view focused on provider discovery + connection setup.
 *
 * Renders a search + category filter at the top, then a 3-section grid
 * (API Key / OAuth / Local) populated from `useProviderCatalog`. Click
 * "Add" on a card to open the AddProviderDialog pre-scoped to that
 * provider kind. Click the card body to drill into the per-kind
 * details view at `/providers/:kind`.
 */

import {SearchX} from "@lucide/vue";
import {computed, onMounted, ref} from "vue";
import {useI18n} from "vue-i18n";
import AddProviderDialog from "@/components/AddProviderDialog.vue";
import EmptyState from "@/components/EmptyState.vue";
import ErrorBanner from "@/components/ErrorBanner.vue";
import LoadingState from "@/components/LoadingState.vue";
import PageHeader from "@/components/PageHeader.vue";
import ProviderCatalogFilter, {
  type CategoryFilter,
} from "@/components/ProviderCatalogFilter.vue";
import ProviderCategorySection from "@/components/ProviderCategorySection.vue";
import {Button} from "@/components/ui/button";
import {useAvailableModels} from "@/composables/useAvailableModels";
import {useProviderCatalog} from "@/composables/useProviderCatalog";
import {useProviders} from "@/composables/useProviders";
import {CATEGORIES, type ProviderKind} from "@/config/providerCatalog";

const {t} = useI18n();

const {fetch, loading, error} = useProviders();
const {fetch: fetchModels} = useAvailableModels();
const {items, byCategory} = useProviderCatalog();

const searchQuery = ref("");
const activeCategory = ref<CategoryFilter>("all");

onMounted(() => {
  fetch();
  fetchModels();
});

const filteredByCategory = computed(() => {
  const query = searchQuery.value.trim().toLowerCase();
  const matches = (kind: ProviderKind, displayNameKey: string) =>
    query === "" ||
    kind.toLowerCase().includes(query) ||
    displayNameKey.toLowerCase().includes(query);
  return CATEGORIES.map((category) => ({
    category,
    items: byCategory(category.kind).value.filter((item) =>
      matches(item.kind, item.displayNameKey),
    ),
  }));
});

const totalMatches = computed(() =>
  filteredByCategory.value.reduce((sum, entry) => sum + entry.items.length, 0),
);

const isFiltering = computed(
  () => searchQuery.value.trim() !== "" || activeCategory.value !== "all",
);

const visibleSections = computed(() => {
  if (activeCategory.value === "all") {
    return filteredByCategory.value;
  }
  return filteredByCategory.value.filter(
    (entry) => entry.category.kind === activeCategory.value,
  );
});

// AddProviderDialog state
const addDialogOpen = ref(false);
const addDialogKind = ref<ProviderKind | undefined>(undefined);

function openAddDialog(kind: ProviderKind) {
  addDialogKind.value = kind;
  addDialogOpen.value = true;
}

function onSaved() {
  addDialogOpen.value = false;
}

function onDeleted() {
  addDialogOpen.value = false;
}
</script>

<template>
  <div class="space-y-6">
    <PageHeader
      :title="t('providers.catalog.title')"
      :description="t('providers.catalog.subtitle')"
    />

    <ErrorBanner v-if="error" :message="error" @retry="fetch" />

    <ProviderCatalogFilter
      v-model:search-query="searchQuery"
      v-model:active-category="activeCategory"
    />

    <LoadingState v-if="loading && items.length === 0" />

    <template v-else>
      <ProviderCategorySection
        v-for="entry in visibleSections"
        :key="entry.category.kind"
        :category="entry.category"
        :items="entry.items"
        @add="openAddDialog"
      />

      <EmptyState
        v-if="totalMatches === 0"
        :title="t('common.noResults')"
        :description="t('providers.catalog.noResults')"
        :icon="SearchX"
      >
        <template #actions>
          <Button
            v-if="isFiltering"
            variant="outline"
            size="sm"
            @click="searchQuery = ''; activeCategory = 'all'"
          >
            {{ t('providers.catalog.filterAll') }}
          </Button>
        </template>
      </EmptyState>
    </template>

    <AddProviderDialog
      v-model:open="addDialogOpen"
      :provider-kind="addDialogKind"
      mode="create"
      @saved="onSaved"
      @deleted="onDeleted"
    />
  </div>
</template>
