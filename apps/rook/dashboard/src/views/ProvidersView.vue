<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { Globe, XCircle, AlertCircle, Activity, RefreshCw } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { useProviders } from '@/composables/useProviders'
import PageHeader from '@/components/PageHeader.vue'
import EmptyState from '@/components/EmptyState.vue'
import ErrorBanner from '@/components/ErrorBanner.vue'
import LoadingState from '@/components/LoadingState.vue'
import DataTable from '@/components/DataTable.vue'

const { t } = useI18n()

const activeTab = ref<'list' | 'quotes'>('list')
const { providers, loading, error, fetch, test } = useProviders()

onMounted(() => {
  fetch()
})

// Mock quotes data - will be replaced when backend has pricing endpoint
const quotes = ref([
  { id: '1', provider: 'OpenAI', model: 'gpt-4o', inputCost: 0.03, outputCost: 0.06, updated: '2024-01-20' },
  { id: '2', provider: 'Anthropic', model: 'claude-opus-4-5', inputCost: 0.015, outputCost: 0.075, updated: '2024-01-20' },
])

async function handleTest(id: string) {
  await test(id)
  await fetch()
}
</script>

<template>
  <div class="space-y-6">
    <PageHeader
      :title="t('nav.providers')"
      :description="t('providers.description')"
    />

    <!-- Error State -->
    <ErrorBanner v-if="error" :error="error" @retry="fetch">
      <template #default>
        <Button variant="ghost" size="sm" @click="fetch">
          <RefreshCw class="h-4 w-4 mr-1" />
          Retry
        </Button>
      </template>
    </ErrorBanner>

    <!-- Tabs -->
    <div class="flex gap-1 border-b">
      <button
        @click="activeTab = 'list'"
        class="px-4 py-2 text-sm font-medium transition-colors relative"
        :class="activeTab === 'list' ? 'text-foreground' : 'text-muted-foreground hover:text-foreground'"
      >
        {{ t('nav.providersList') }}
        <div
          v-if="activeTab === 'list'"
          class="absolute bottom-0 left-0 right-0 h-0.5 bg-primary"
        />
      </button>
      <button
        @click="activeTab = 'quotes'"
        class="px-4 py-2 text-sm font-medium transition-colors relative"
        :class="activeTab === 'quotes' ? 'text-foreground' : 'text-muted-foreground hover:text-foreground'"
      >
        {{ t('nav.providersQuotes') }}
        <div
          v-if="activeTab === 'quotes'"
          class="absolute bottom-0 left-0 right-0 h-0.5 bg-primary"
        />
      </button>
    </div>

    <!-- Loading State -->
    <LoadingState v-if="loading && providers.length === 0" />

    <!-- List Tab -->
    <div v-if="activeTab === 'list'" class="space-y-4">
      <!-- Empty State -->
      <EmptyState v-if="!loading && providers.length === 0">
        <template #icon>
          <Globe class="h-12 w-12 mx-auto text-muted-foreground" />
        </template>
        <template #title><h3 class="font-medium mb-2">{{ t('providers.empty') }}</h3></template>
        <template #description><p class="text-sm text-muted-foreground mb-4">{{ t('providers.emptyDescription') }}</p></template>
        <template #default>
          <Button>{{ t('providers.add') }}</Button>
        </template>
      </EmptyState>

      <!-- Provider List -->
      <DataTable v-if="providers.length > 0">
        <template #header>
          <tr class="border-b bg-muted/50">
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('providers.name') }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('providers.kind') }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('providers.status') }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('providers.latency') }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('providers.priority') }}</th>
            <th class="px-4 py-3 text-right text-sm font-medium">{{ t('common.actions') }}</th>
          </tr>
        </template>
        <template #body>
          <tr v-for="provider in providers" :key="provider.id" class="hover:bg-muted/30">
            <td class="px-4 py-3">
              <div class="flex items-center gap-2">
                <Globe class="h-4 w-4 text-muted-foreground" />
                <span class="font-medium">{{ provider.name }}</span>
              </div>
            </td>
            <td class="px-4 py-3">
              <span class="text-sm font-mono text-muted-foreground">{{ provider.providerKind }}</span>
            </td>
            <td class="px-4 py-3">
              <div class="flex items-center gap-2">
                <AlertCircle :class="['h-4 w-4', provider.testStatus.status === 'active' ? 'text-green-500' : provider.testStatus.status === 'unhealthy' ? 'text-destructive' : 'text-yellow-500']" />
                <span class="text-sm capitalize">{{ provider.testStatus.status }}</span>
              </div>
              <p v-if="provider.testStatus.error" class="text-xs text-destructive mt-0.5 max-w-[200px] truncate">
                {{ provider.testStatus.error }}
              </p>
            </td>
            <td class="px-4 py-3">
              <div v-if="provider.testStatus.latencyMs" class="flex items-center gap-1 text-sm text-muted-foreground">
                <Activity class="h-3 w-3" />
                {{ provider.testStatus.latencyMs }}ms
              </div>
              <span v-else class="text-sm text-muted-foreground">—</span>
            </td>
            <td class="px-4 py-3">
              <span class="text-sm text-muted-foreground">{{ provider.priority }}</span>
            </td>
            <td class="px-4 py-3 text-right">
              <div class="flex items-center justify-end gap-2">
                <Button variant="ghost" size="sm" @click="handleTest(provider.id)">
                  <RefreshCw class="h-4 w-4" />
                </Button>
                <Button variant="ghost" size="sm" class="text-destructive hover:text-destructive">
                  <XCircle class="h-4 w-4" />
                </Button>
              </div>
            </td>
          </tr>
        </template>
      </DataTable>
    </div>

    <!-- Quotes Tab -->
    <div v-if="activeTab === 'quotes'" class="space-y-4">
      <DataTable>
        <template #header>
          <tr class="border-b bg-muted/50">
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('providers.quotes.provider') }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('providers.quotes.model') }}</th>
            <th class="px-4 py-3 text-right text-sm font-medium">{{ t('providers.quotes.inputCost') }}</th>
            <th class="px-4 py-3 text-right text-sm font-medium">{{ t('providers.quotes.outputCost') }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('providers.quotes.updated') }}</th>
          </tr>
        </template>
        <template #body>
          <tr v-for="quote in quotes" :key="quote.id" class="hover:bg-muted/30">
            <td class="px-4 py-3 text-sm font-medium">{{ quote.provider }}</td>
            <td class="px-4 py-3 text-sm text-muted-foreground font-mono">{{ quote.model }}</td>
            <td class="px-4 py-3 text-sm text-right">${{ quote.inputCost }}/1M</td>
            <td class="px-4 py-3 text-sm text-right">${{ quote.outputCost }}/1M</td>
            <td class="px-4 py-3 text-sm text-muted-foreground">{{ quote.updated }}</td>
          </tr>
        </template>
      </DataTable>
    </div>
  </div>
</template>
