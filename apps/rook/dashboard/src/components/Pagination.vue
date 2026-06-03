<script setup lang="ts">
import { computed } from 'vue'
import { Button } from '@/components/ui/button'
import { ChevronLeft, ChevronRight } from '@lucide/vue'

const props = withDefaults(defineProps<{
  offset?: number
  limit?: number
  total?: number
  hasPrev?: boolean
  hasNext?: boolean
  onPrev?: () => void
  onNext?: () => void
}>(), {
  offset: 0,
  limit: 20,
  total: 0,
  hasPrev: false,
  hasNext: false,
})

const currentPage = computed(() => Math.floor(props.offset / props.limit) + 1)
const totalPages = computed(() => Math.ceil(props.total / props.limit) || 1)

const goToPrev = () => {
  if (props.onPrev) props.onPrev()
}

const goToNext = () => {
  if (props.onNext) props.onNext()
}
</script>

<template>
  <div class="flex items-center justify-between">
    <div class="text-sm text-muted-foreground">
      Page {{ currentPage }} of {{ totalPages }}
    </div>
    <div class="flex items-center gap-2">
      <Button
        variant="outline"
        size="sm"
        :disabled="!hasPrev"
        @click="goToPrev"
      >
        <ChevronLeft class="h-4 w-4" />
        Previous
      </Button>
      <Button
        variant="outline"
        size="sm"
        :disabled="!hasNext"
        @click="goToNext"
      >
        Next
        <ChevronRight class="h-4 w-4" />
      </Button>
    </div>
  </div>
</template>
