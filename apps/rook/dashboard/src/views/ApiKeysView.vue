<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Plus, Copy, Trash2, Eye, EyeOff, Key } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

const { t } = useI18n()

// Mock data - will be replaced with API call
const apiKeys = ref([
  { id: '1', name: 'Production Key', key: 'sk-rook-prod-xxxxx', created: '2024-01-15', lastUsed: '2024-01-20' },
  { id: '2', name: 'Development Key', key: 'sk-rook-dev-xxxxx', created: '2024-01-10', lastUsed: null },
])

const showCreateForm = ref(false)
const newKeyName = ref('')
const showKeyValue = ref<string | null>(null)

async function copyToClipboard(text: string) {
  try {
    await navigator.clipboard.writeText(text)
  } catch (err) {
    console.error('Failed to copy:', err)
  }
}

function toggleKeyVisibility(id: string, key: string) {
  showKeyValue.value = showKeyValue.value === id ? null : id
}

function maskKey(key: string): string {
  return key.slice(0, 8) + '...' + key.slice(-4)
}
</script>

<template>
  <div class="space-y-6">
    <!-- Page Header -->
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-semibold tracking-tight">{{ t('nav.apiKeys') }}</h1>
        <p class="text-muted-foreground">{{ t('apiKeys.description') }}</p>
      </div>
      <Button @click="showCreateForm = !showCreateForm">
        <Plus class="h-4 w-4 mr-2" />
        {{ t('apiKeys.create') }}
      </Button>
    </div>

    <!-- Create Form -->
    <div v-if="showCreateForm" class="rounded-lg border bg-card p-4 space-y-4">
      <h3 class="font-medium">{{ t('apiKeys.createNew') }}</h3>
      <div class="flex gap-4">
        <Input
          v-model="newKeyName"
          :placeholder="t('apiKeys.namePlaceholder')"
          class="max-w-sm"
        />
        <Button>{{ t('common.create') }}</Button>
        <Button variant="ghost" @click="showCreateForm = false">{{ t('common.cancel') }}</Button>
      </div>
    </div>

    <!-- Keys List -->
    <div class="rounded-lg border">
      <table class="w-full">
        <thead>
          <tr class="border-b bg-muted/50">
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.name') }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.key') }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.created') }}</th>
            <th class="px-4 py-3 text-left text-sm font-medium">{{ t('apiKeys.lastUsed') }}</th>
            <th class="px-4 py-3 text-right text-sm font-medium">{{ t('common.actions') }}</th>
          </tr>
        </thead>
        <tbody class="divide-y">
          <tr v-for="item in apiKeys" :key="item.id" class="hover:bg-muted/30">
            <td class="px-4 py-3">
              <div class="flex items-center gap-2">
                <Key class="h-4 w-4 text-muted-foreground" />
                <span class="font-medium">{{ item.name }}</span>
              </div>
            </td>
            <td class="px-4 py-3">
              <div class="flex items-center gap-2">
                <code class="text-sm font-mono text-muted-foreground">
                  {{ showKeyValue === item.id ? item.key : maskKey(item.key) }}
                </code>
                <button
                  @click="toggleKeyVisibility(item.id, item.key)"
                  class="text-muted-foreground hover:text-foreground"
                >
                  <Eye v-if="showKeyValue !== item.id" class="h-4 w-4" />
                  <EyeOff v-else class="h-4 w-4" />
                </button>
                <button
                  @click="copyToClipboard(item.key)"
                  class="text-muted-foreground hover:text-foreground"
                >
                  <Copy class="h-4 w-4" />
                </button>
              </div>
            </td>
            <td class="px-4 py-3 text-sm text-muted-foreground">{{ item.created }}</td>
            <td class="px-4 py-3 text-sm text-muted-foreground">
              {{ item.lastUsed || '—' }}
            </td>
            <td class="px-4 py-3 text-right">
              <button class="text-destructive hover:text-destructive/80">
                <Trash2 class="h-4 w-4" />
              </button>
            </td>
          </tr>
        </tbody>
      </table>

      <div v-if="apiKeys.length === 0" class="p-8 text-center text-muted-foreground">
        {{ t('apiKeys.empty') }}
      </div>
    </div>
  </div>
</template>
