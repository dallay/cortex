<script setup lang="ts" generic="T extends Record<string, any>">
import { computed } from 'vue'
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from '@/components/ui/table'

const props = defineProps<{
  data: T[]
  columns: {
    key: string
    label: string
    sortable?: boolean
  }[]
}>()

const emit = defineEmits<{
  'row-click': [row: T]
}>()

const handleRowClick = (row: T) => {
  emit('row-click', row)
}
</script>

<template>
  <div class="rounded-md border">
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead v-for="column in columns" :key="column.key">
            {{ column.label }}
          </TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        <TableRow
          v-for="(row, index) in data"
          :key="index"
          @click="handleRowClick(row)"
          class="cursor-pointer"
        >
          <TableCell v-for="column in columns" :key="column.key">
            <slot :name="`cell-${column.key}`" :row="row" :value="row[column.key]">
              {{ row[column.key] }}
            </slot>
          </TableCell>
        </TableRow>
        <TableRow v-if="data.length === 0">
          <TableCell :colspan="columns.length" class="text-center text-muted-foreground">
            No data available
          </TableCell>
        </TableRow>
      </TableBody>
    </Table>
  </div>
</template>
