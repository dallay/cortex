<script setup lang="ts">
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { AlertCircle } from '@lucide/vue'
import CopyButton from '@/components/CopyButton.vue'

defineProps<{
  apiKey: string
  title?: string
  description?: string
  warning?: string
}>()
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle>{{ title || 'API Key Created' }}</CardTitle>
      <CardDescription v-if="description">
        {{ description }}
      </CardDescription>
    </CardHeader>
    <CardContent class="space-y-4">
      <Alert v-if="warning" variant="destructive">
        <AlertCircle class="h-4 w-4" />
        <AlertDescription>
          {{ warning }}
        </AlertDescription>
      </Alert>
      <div class="flex items-center gap-2 p-3 bg-muted rounded-md font-mono text-sm">
        <code data-testid="api-key-display" class="flex-1 break-words overflow-x-auto">{{ apiKey }}</code>
        <CopyButton :value="apiKey" />
      </div>
    </CardContent>
  </Card>
</template>
