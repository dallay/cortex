<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { Plus, CircleDot, ArrowRight, Trash2 } from '@lucide/vue'
import { Button } from '@/components/ui/button'

const { t } = useI18n()

// Mock data - combos are fallback chains of models
const combos = ref([
  {
    id: '1',
    name: 'Production Fallback',
    description: 'Primary with automatic fallback',
    steps: [
      { provider: 'OpenAI', model: 'gpt-4' },
      { provider: 'Anthropic', model: 'claude-3-opus' },
      { provider: 'Ollama', model: 'llama2' },
    ],
  },
  {
    id: '2',
    name: 'Fast Responses',
    description: 'Optimized for speed',
    steps: [
      { provider: 'OpenAI', model: 'gpt-3.5-turbo' },
      { provider: 'Anthropic', model: 'claude-3-haiku' },
    ],
  },
])

const showCreateForm = ref(false)
</script>

<template>
  <div class="space-y-6">
    <!-- Page Header -->
    <div class="flex items-center justify-between">
      <div>
        <h1 class="text-2xl font-semibold tracking-tight">{{ t('nav.combos') }}</h1>
        <p class="text-muted-foreground">{{ t('combos.description') }}</p>
      </div>
      <Button>
        <Plus class="h-4 w-4 mr-2" />
        {{ t('combos.create') }}
      </Button>
    </div>

    <!-- Create Form -->
    <div v-if="showCreateForm" class="rounded-lg border bg-card p-4">
      <h3 class="font-medium mb-4">{{ t('combos.createNew') }}</h3>
      <p class="text-sm text-muted-foreground">{{ t('common.loading') }}</p>
    </div>

    <!-- Combos List -->
    <div class="grid gap-4 md:grid-cols-2">
      <div
        v-for="combo in combos"
        :key="combo.id"
        class="rounded-lg border bg-card p-4 hover:bg-muted/30 transition-colors"
      >
        <div class="flex items-start justify-between mb-3">
          <div class="flex items-center gap-2">
            <CircleDot class="h-5 w-5 text-primary" />
            <div>
              <h3 class="font-medium">{{ combo.name }}</h3>
              <p class="text-sm text-muted-foreground">{{ combo.description }}</p>
            </div>
          </div>
          <button class="text-muted-foreground hover:text-destructive">
            <Trash2 class="h-4 w-4" />
          </button>
        </div>

        <!-- Fallback Chain -->
        <div class="flex items-center gap-2 flex-wrap">
          <div
            v-for="(step, index) in combo.steps"
            :key="index"
            class="flex items-center gap-2"
          >
            <div class="flex items-center gap-2 px-3 py-1.5 rounded-md bg-muted text-sm">
              <span class="font-medium">{{ step.provider }}</span>
              <span class="text-muted-foreground">·</span>
              <span class="font-mono text-xs">{{ step.model }}</span>
            </div>
            <ArrowRight
              v-if="index < combo.steps.length - 1"
              class="h-4 w-4 text-muted-foreground shrink-0"
            />
          </div>
        </div>
      </div>
    </div>

    <!-- Empty State -->
    <div v-if="combos.length === 0" class="text-center py-12">
      <CircleDot class="h-12 w-12 mx-auto text-muted-foreground mb-4" />
      <h3 class="font-medium mb-2">{{ t('combos.empty') }}</h3>
      <p class="text-sm text-muted-foreground mb-4">{{ t('combos.emptyDescription') }}</p>
      <Button>{{ t('combos.createFirst') }}</Button>
    </div>
  </div>
</template>
