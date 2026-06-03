<script setup lang="ts">
import { computed } from 'vue'
import { Badge } from '@/components/ui/badge'

const props = defineProps<{
  method: string
}>()

const variant = computed(() => {
  const method = props.method.toUpperCase()
  switch (method) {
    case 'GET':
      return 'default'
    case 'POST':
      return 'default'
    case 'PUT':
      return 'default'
    case 'DELETE':
      return 'destructive'
    case 'PATCH':
      return 'secondary'
    default:
      return 'outline'
  }
})

const colorClass = computed(() => {
  const method = props.method.toUpperCase()
  switch (method) {
    case 'GET':
      return 'bg-blue-100 text-blue-800 hover:bg-blue-100'
    case 'POST':
      return 'bg-green-100 text-green-800 hover:bg-green-100'
    case 'PUT':
      return 'bg-yellow-100 text-yellow-800 hover:bg-yellow-100'
    case 'PATCH':
      return 'bg-purple-100 text-purple-800 hover:bg-purple-100'
    default:
      return ''
  }
})
</script>

<template>
  <Badge :variant="variant" :class="colorClass" class="font-mono text-xs">
    {{ method.toUpperCase() }}
  </Badge>
</template>
