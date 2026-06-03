<script setup lang="ts">
import { Alert, AlertDescription } from '@/components/ui/alert'
import { AlertCircle } from '@lucide/vue'

defineProps<{
  message: string
}>()
</script>

<template>
  <Alert variant="destructive" class="mb-4">
    <AlertCircle class="h-4 w-4" />
    <AlertDescription>
      {{ message }}
    </AlertDescription>
  </Alert>
</template>
