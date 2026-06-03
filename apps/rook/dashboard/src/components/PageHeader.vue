<script setup lang="ts">
defineProps<{
  title: string
  description?: string
}>()
</script>

<template>
  <div class="mb-6 space-y-2">
    <h1 class="text-3xl font-bold tracking-tight">{{ title }}</h1>
    <p v-if="description" class="text-muted-foreground">
      {{ description }}
    </p>
    <div v-if="$slots.default" class="mt-4">
      <slot />
    </div>
  </div>
</template>
