<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Copy, Check, Settings, Globe } from '@lucide/vue'
import { useBaseUrl } from '@/composables/useBaseUrl'
import { endpointsConfig } from '@/config/endpoints'

const { t } = useI18n()
const { fullBaseUrl, isOverridden, setOverride, clearOverride } = useBaseUrl()

const copied = ref<string | null>(null)

async function copyToClipboard(text: string, id: string) {
  try {
    await navigator.clipboard.writeText(text)
    copied.value = id
    setTimeout(() => {
      copied.value = null
    }, 2000)
  } catch (err) {
    console.error('Failed to copy:', err)
  }
}

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
    <!-- Page Header -->
    <div>
      <h1 class="text-2xl font-semibold tracking-tight">
        {{ t('endpoints.title') }}
      </h1>
      <p class="text-muted-foreground">
        {{ t('endpoints.description') }}
      </p>
    </div>

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
          <button
            @click="copyToClipboard(fullBaseUrl, 'base')"
            class="ml-2 shrink-0 text-muted-foreground hover:text-foreground"
          >
            <Check v-if="copied === 'base'" class="h-4 w-4 text-green-500" />
            <Copy v-else class="h-4 w-4" />
          </button>
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
              <span
                class="shrink-0 rounded bg-primary px-2 py-0.5 text-xs font-medium text-primary-foreground"
                :class="{
                  'bg-green-500': endpoint.method === 'GET',
                  'bg-blue-500': endpoint.method === 'POST',
                }"
              >
                {{ endpoint.method }}
              </span>
              <span class="font-mono text-sm truncate">{{ endpoint.path }}</span>
            </div>
            <div class="flex items-center gap-3 shrink-0">
              <span class="text-sm text-muted-foreground hidden sm:block">
                {{ t(endpoint.descriptionKey) }}
              </span>
              <button
                @click="copyToClipboard(fullBaseUrl + endpoint.path, endpoint.path)"
                class="text-muted-foreground hover:text-foreground"
              >
                <Check
                  v-if="copied === endpoint.path"
                  class="h-4 w-4 text-green-500"
                />
                <Copy v-else class="h-4 w-4" />
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  </div>
</template>
