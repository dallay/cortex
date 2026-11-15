<script setup lang="ts">
/**
 * ProvidersView — catalog of all supported providers.
 *
 * Replaces the legacy two-tab list/quotes layout. The "Quotes" tab
 * moves to its own dedicated route (`/providers/quota` → Phase 6),
 * keeping this view focused on provider discovery + connection setup.
 *
 * Renders a search + category filter at the top, then 7 sections
 * (Compatible / OAuth / IDE / API Key / Free / Local / Other) populated
 * from `useProviderCatalog`. Click
 * "Add" on a card to open the AddProviderDialog pre-scoped to that
 * provider kind. Click the card body to drill into the per-kind
 * details view at `/providers/:kind`.
 */

import {SearchX} from "@lucide/vue";
import {computed, onMounted, ref} from "vue";
import {useI18n} from "vue-i18n";
import AddProviderDialog from "@/components/AddProviderDialog.vue";
import CompatibleConnectionsSection from "@/components/CompatibleConnectionsSection.vue";
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

const {providers, update, test, fetch, loading, error} = useProviders();
const {fetch: fetchModels} = useAvailableModels();
const {items, byCategory, fetch: fetchDescriptors, loading: descriptorsLoading, error: descriptorsError} = useProviderCatalog();

const searchQuery = ref("");
const activeCategory = ref<CategoryFilter>("all");

onMounted(() => {
  void fetch();
  void fetchModels();
  void fetchDescriptors();
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

const compatibleConnections = computed(() => {
  const query = searchQuery.value.trim().toLowerCase();
  return providers.value.filter((connection) =>
    ["openai-compatible", "anthropic-compatible"].includes(connection.providerKind) &&
    (query === "" || connection.name.toLowerCase().includes(query) || connection.providerKind.includes(query)),
  );
});

const showCompatible = computed(() =>
  (activeCategory.value === "all" || activeCategory.value === "compatible") &&
  (searchQuery.value.trim() === "" || compatibleConnections.value.length > 0),
);

const totalMatches = computed(() =>
  visibleSections.value.reduce((sum, entry) => sum + entry.items.length, 0) +
  (showCompatible.value ? compatibleConnections.value.length : 0),
);

const isFiltering = computed(
  () => searchQuery.value.trim() !== "" || activeCategory.value !== "all",
);

const visibleSections = computed(() => {
  if (activeCategory.value === "all") {
    return filteredByCategory.value.filter((entry) => entry.category.kind !== "compatible");
  }
  return filteredByCategory.value.filter(
    (entry) => entry.category.kind === activeCategory.value && entry.category.kind !== "compatible",
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
  fetch();
}

function onDeleted() {
  addDialogOpen.value = false;
  fetch();
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

    <LoadingState v-if="(descriptorsLoading || loading) && items.length === 0" />
    <template v-else>
      <ErrorBanner v-if="descriptorsError" :message="descriptorsError" @retry="fetchDescriptors" />

      <CompatibleConnectionsSection
        v-if="showCompatible"
        :providers="compatibleConnections"
        :update="update"
        :test="test"
        :refresh="fetch"
        @add="openAddDialog"
      />

      <ProviderCategorySection
        v-for="entry in visibleSections"
        :key="entry.category.kind"
        :category="entry.category"
        :items="entry.items"
        @add="openAddDialog"
      />

      <EmptyState
        v-if="totalMatches === 0 && !descriptorsError"
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
