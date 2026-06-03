<script setup lang="ts">
defineProps<{
  title: string
  description?: string
  icon?: any
}>()
</script>

<template>
  <div class="flex flex-col items-center justify-center py-12 text-center">
    <component
      v-if="icon"
      :is="icon"
      class="h-12 w-12 text-muted-foreground mb-4"
    />
    <h3 class="text-lg font-semibold mb-2">{{ title }}</h3>
    <p v-if="description" class="text-sm text-muted-foreground mb-4 max-w-sm">
      {{ description }}
    </p>
    <div v-if="$slots.default">
      <slot />
    </div>
  </div>
</template>
