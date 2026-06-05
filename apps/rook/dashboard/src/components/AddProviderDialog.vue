<script setup lang="ts">
import { ref, computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { Plus, Loader2, CheckCircle2, AlertCircle } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Switch } from '@/components/ui/switch'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import PasswordInput from '@/components/PasswordInput.vue'
import { useProviders } from '@/composables/useProviders'
import type { CreateProviderRequest } from '@/lib/api'

const { t } = useI18n()
const { create, test, fetch } = useProviders()

const open = ref(false)
const testing = ref(false)
const saving = ref(false)
const testResult = ref<{ ok: boolean; message: string } | null>(null)
const showAdvanced = ref(false)

// Form state
const form = ref({
  name: '',
  apiKey: '',
  baseUrl: 'https://api.ollama.com',
  priority: 100,
  isActive: true,
  maxConcurrent: 10,
  defaultModel: '',
})

const isValid = computed(() => {
  return form.value.name.trim() !== '' && form.value.apiKey.trim() !== ''
})

async function handleTest() {
  if (!isValid.value) return

  testing.value = true
  testResult.value = null

  try {
    // Create a temporary provider to test
    const tempProvider = buildCreateRequest()
    const created = await create(tempProvider)
    
    if (!created) {
      testResult.value = { ok: false, message: 'Failed to create test connection' }
      return
    }

    // Test the connection
    const result = await test(created.id)
    
    if (result?.ok) {
      testResult.value = { ok: true, message: `Connected successfully (${result.latencyMs}ms)` }
    } else {
      testResult.value = { ok: false, message: result?.error || 'Connection failed' }
    }
  } catch (error) {
    testResult.value = { 
      ok: false, 
      message: error instanceof Error ? error.message : 'Test failed' 
    }
  } finally {
    testing.value = false
  }
}

async function handleSave() {
  if (!isValid.value) return

  saving.value = true
  try {
    const request = buildCreateRequest()
    const created = await create(request)
    
    if (created) {
      await fetch() // Refresh the provider list
      resetForm()
      open.value = false
    }
  } finally {
    saving.value = false
  }
}

function buildCreateRequest(): CreateProviderRequest {
  // Generate a unique runtime ID for this provider instance
  const timestamp = Date.now()
  const randomSuffix = crypto.randomUUID().substring(0, 8)
  const runtimeId = `ollama-${timestamp}-${randomSuffix}`
  
  return {
    providerKind: 'ollama',
    providerRuntimeId: runtimeId,
    authType: 'apiKey',
    name: form.value.name,
    priority: form.value.priority,
    isActive: form.value.isActive,
    credentials: {
      apiKey: form.value.apiKey,
    },
    config: {
      maxConcurrent: form.value.maxConcurrent,
      quotaWindowThresholds: {
        warning: 0.8,
        error: 0.95,
      },
      defaultModel: form.value.defaultModel || undefined,
      baseUrl: form.value.baseUrl || undefined,
    },
  }
}

function resetForm() {
  form.value = {
    name: '',
    apiKey: '',
    baseUrl: 'https://api.ollama.com',
    priority: 100,
    isActive: true,
    maxConcurrent: 10,
    defaultModel: '',
  }
  testResult.value = null
  showAdvanced.value = false
}

function handleOpenChange(value: boolean) {
  open.value = value
  if (!value) {
    resetForm()
  }
}
</script>

<template>
  <Dialog :open="open" @update:open="handleOpenChange">
    <DialogTrigger as-child>
      <Button>
        <Plus class="h-4 w-4 mr-2" />
        {{ t('providers.add') }}
      </Button>
    </DialogTrigger>
    <DialogContent class="sm:max-w-[525px]">
      <DialogHeader>
        <DialogTitle>{{ t('providers.addProvider') }}</DialogTitle>
        <DialogDescription>
          {{ t('providers.addProviderDescription') }}
        </DialogDescription>
      </DialogHeader>

      <div class="space-y-4 py-4">
        <!-- Provider Name -->
        <div class="space-y-2">
          <Label for="name">{{ t('providers.name') }}</Label>
          <Input
            id="name"
            v-model="form.name"
            placeholder="e.g., Ollama Production"
            :disabled="saving"
          />
        </div>

        <!-- API Key -->
        <div class="space-y-2">
          <Label for="apiKey">{{ t('providers.apiKey') }}</Label>
          <PasswordInput
            id="apiKey"
            v-model="form.apiKey"
            placeholder="Enter your Ollama Cloud API key"
            :disabled="saving"
          />
          <p class="text-xs text-muted-foreground">
            {{ t('providers.apiKeyHint') }}
          </p>
        </div>

        <!-- Base URL -->
        <div class="space-y-2">
          <Label for="baseUrl">{{ t('providers.baseUrl') }}</Label>
          <Input
            id="baseUrl"
            v-model="form.baseUrl"
            placeholder="https://api.ollama.com"
            :disabled="saving"
          />
        </div>

        <!-- Priority -->
        <div class="space-y-2">
          <Label for="priority">{{ t('providers.priority') }}</Label>
          <Input
            id="priority"
            v-model.number="form.priority"
            type="number"
            min="0"
            max="255"
            :disabled="saving"
          />
          <p class="text-xs text-muted-foreground">
            {{ t('providers.priorityHint') }}
          </p>
        </div>

        <!-- Is Active -->
        <div class="flex items-center justify-between">
          <Label for="isActive">{{ t('providers.active') }}</Label>
          <Switch
            id="isActive"
            v-model:checked="form.isActive"
            :disabled="saving"
          />
        </div>

        <!-- Advanced Config -->
        <Collapsible v-model:open="showAdvanced">
          <CollapsibleTrigger class="flex items-center gap-2 text-sm font-medium">
            {{ t('providers.advancedConfig') }}
          </CollapsibleTrigger>
          <CollapsibleContent class="space-y-4 pt-4">
            <div class="space-y-2">
              <Label for="maxConcurrent">{{ t('providers.maxConcurrent') }}</Label>
              <Input
                id="maxConcurrent"
                v-model.number="form.maxConcurrent"
                type="number"
                min="1"
                max="100"
                :disabled="saving"
              />
            </div>
            <div class="space-y-2">
              <Label for="defaultModel">{{ t('providers.defaultModel') }}</Label>
              <Input
                id="defaultModel"
                v-model="form.defaultModel"
                placeholder="e.g., llama3.1"
                :disabled="saving"
              />
            </div>
          </CollapsibleContent>
        </Collapsible>

        <!-- Test Result -->
        <div v-if="testResult" class="flex items-start gap-2 rounded-md border p-3">
          <CheckCircle2 v-if="testResult.ok" class="h-5 w-5 text-green-500 mt-0.5" />
          <AlertCircle v-else class="h-5 w-5 text-destructive mt-0.5" />
          <div class="flex-1">
            <p class="text-sm font-medium" :class="testResult.ok ? 'text-green-600' : 'text-destructive'">
              {{ testResult.ok ? t('providers.testSuccess') : t('providers.testFailed') }}
            </p>
            <p class="text-xs text-muted-foreground mt-0.5">
              {{ testResult.message }}
            </p>
          </div>
        </div>
      </div>

      <DialogFooter>
        <Button
          variant="outline"
          @click="handleTest"
          :disabled="!isValid || testing || saving"
        >
          <Loader2 v-if="testing" class="h-4 w-4 mr-2 animate-spin" />
          {{ t('providers.testConnection') }}
        </Button>
        <Button
          @click="handleSave"
          :disabled="!isValid || saving"
        >
          <Loader2 v-if="saving" class="h-4 w-4 mr-2 animate-spin" />
          {{ t('common.save') }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
