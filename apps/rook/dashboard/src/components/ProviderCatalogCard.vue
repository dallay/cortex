<script setup lang="ts">
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
 * Status badge reflects `hasActiveConnections` (live data from
 * `useProviderCatalog`). The card is purely presentational — all
 * state-changing actions live in the details view.
 *
 * The provider icon is rendered by `ProviderIcon.vue` — a branded
 * `<img src="/providers/<iconFile>">` with a Lucide `Server` fallback.
 * Catalog cards are below the fold by default so we use `loading="lazy"`.
 */
import {computed} from 'vue'
import { useI18n } from 'vue-i18n'
import { Plus } from '@lucide/vue'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import type { ProviderCatalogItem } from '@/composables/useProviderCatalog'
import ProviderIcon from '@/components/ProviderIcon.vue'

const props = defineProps<{
  item: ProviderCatalogItem
}>()

const emit = defineEmits<{
  add: [kind: ProviderCatalogItem['kind']]
}>()

const { t } = useI18n()

const name = computed(() => t(props.item.displayNameKey))
const description = computed(() => t(props.item.descriptionKey))

const statusLabel = computed(() =>
  props.item.hasActiveConnections
    ? t('providers.catalog.active')
    : t('providers.catalog.notConfigured'),
)

const statusVariant = computed(() =>
  props.item.hasActiveConnections ? 'default' : 'secondary',
)

const detailLink = computed(() => `/providers/${props.item.kind}`)
</script>

<template>
  <Card
    class="transition-colors hover:bg-muted/30"
    :data-testid="`provider-card-${item.kind}`"
  >
    <RouterLink
      :to="detailLink"
      class="block"
      :data-testid="`provider-card-link-${item.kind}`"
    >
      <CardHeader class="flex flex-row items-start justify-between gap-3 space-y-0 pb-2">
        <div class="flex items-start gap-3 min-w-0">
          <div class="rounded-lg bg-primary/10 p-2 shrink-0">
            <ProviderIcon
              :kind="item.kind"
              loading="lazy"
              :width="20"
              :height="20"
              :decorative="true"
              class="text-primary"
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
        <span class="text-xs text-muted-foreground">
          {{ t('providers.catalog.connectionsCount', item.connectionCount) }}
        </span>
      </CardContent>
    </RouterLink>
    <div class="px-6 pb-6 pt-0 flex items-center gap-2">
      <Button
        variant="outline"
        size="sm"
        class="flex-1"
        :data-testid="`provider-card-add-${item.kind}`"
        @click.stop.prevent="emit('add', item.kind)"
      >
        <Plus class="h-4 w-4 mr-1" />
        {{ t('providers.catalog.addProvider') }}
      </Button>
    </div>
  </Card>
</template>
