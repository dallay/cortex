<script setup lang="ts">
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
import { useI18n } from 'vue-i18n'
import ProviderCatalogCard from './ProviderCatalogCard.vue'
import type { CategoryDescriptor } from '@/config/providerCatalog'
import type { ProviderCatalogItem } from '@/composables/useProviderCatalog'

defineProps<{
  category: CategoryDescriptor
  items: readonly ProviderCatalogItem[]
}>()

const emit = defineEmits<{
  add: [kind: ProviderCatalogItem['kind']]
}>()

const { t } = useI18n()
</script>

<template>
  <section v-if="items.length > 0" class="space-y-3" :data-testid="`catalog-section-${category.kind}`">
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
        :item="item"
        @add="kind => emit('add', kind)"
      />
    </div>
  </section>
</template>
