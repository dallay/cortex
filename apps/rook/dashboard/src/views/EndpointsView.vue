<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { Settings, Globe } from '@lucide/vue'
import { useBaseUrl } from '@/composables/useBaseUrl'
import { endpointsConfig } from '@/config/endpoints'
import PageHeader from '@/components/PageHeader.vue'
import MethodBadge from '@/components/MethodBadge.vue'
import CopyButton from '@/components/CopyButton.vue'

const { t } = useI18n()
const { fullBaseUrl, isOverridden, setOverride, clearOverride } = useBaseUrl()

function handleBaseUrlEdit() {
  const newUrl = prompt('Enter custom base URL:', fullBaseUrl.value)
  if (newUrl !== null) {
    if (newUrl === '') {
      clearOverride()
    } else {
      setOverride(newUrl.replace(/\/v1$/, ''))
    }
  }
}

const categories = [
  { key: 'core', labelKey: 'endpoints.categories.core' },
  { key: 'media', labelKey: 'endpoints.categories.media' },
  { key: 'utility', labelKey: 'endpoints.categories.utility' },
] as const
</script>

<template>
  <div class="space-y-6">
    <PageHeader
      :title="t('endpoints.title')"
      :description="t('endpoints.description')"
    />

    <!-- Base URL Card -->
    <div class="rounded-lg border bg-card text-card-foreground shadow-sm">
      <div class="flex flex-row items-center justify-between p-4">
        <div class="flex items-center gap-2">
          <Globe class="h-4 w-4 text-muted-foreground" />
          <span class="text-sm font-medium">Base URL</span>
          <span v-if="isOverridden" class="text-xs text-muted-foreground">(custom)</span>
        </div>
        <button
          @click="handleBaseUrlEdit"
          class="inline-flex items-center gap-1 text-sm text-muted-foreground hover:text-foreground"
        >
          <Settings class="h-3 w-3" />
          Edit
        </button>
      </div>
      <div class="px-4 pb-4 pt-0">
        <div
          class="flex items-center justify-between rounded-md border bg-muted/50 px-3 py-2 font-mono text-sm"
        >
          <span class="truncate">{{ fullBaseUrl }}</span>
          <CopyButton :text="fullBaseUrl" />
        </div>
        <p v-if="isOverridden" class="mt-2 text-xs text-muted-foreground">
          Using custom URL. Leave empty and save to revert to auto-detected.
        </p>
        <p v-else class="mt-2 text-xs text-muted-foreground">
          Auto-detected from browser. Edit to set a custom domain.
        </p>
      </div>
    </div>

    <!-- Endpoint Categories -->
    <div v-for="category in categories" :key="category.key" class="space-y-3">
      <h2 class="text-sm font-semibold uppercase tracking-wide text-muted-foreground">
        {{ t(category.labelKey) }}
      </h2>
      <div class="rounded-lg border bg-card text-card-foreground shadow-sm overflow-hidden">
        <div class="divide-y">
          <div
            v-for="endpoint in endpointsConfig[category.key]"
            :key="endpoint.path"
            class="flex items-center justify-between px-4 py-3 hover:bg-muted/50"
          >
            <div class="flex items-center gap-4 min-w-0 flex-1">
              <MethodBadge :method="endpoint.method" />
              <span class="font-mono text-sm truncate">{{ endpoint.path }}</span>
            </div>
            <div class="flex items-center gap-3 shrink-0">
              <span class="text-sm text-muted-foreground hidden sm:block">
                {{ t(endpoint.descriptionKey) }}
              </span>
              <CopyButton :text="fullBaseUrl + endpoint.path" />
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
