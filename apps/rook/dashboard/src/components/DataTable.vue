<script setup lang="ts" generic="T extends Record<string, any>">
import {
  Table,
} from '@/components/ui/table'

withDefaults(defineProps<{
  columns?: {
    key: string
    label: string
    sortable?: boolean
  }[]
}>(), {
  columns: () => [],
})
</script>

<template>
  <div class="rounded-md border">
    <Table>
      <slot />
    </Table>
  </div>
</template>
