<script setup lang="ts">
import { ref } from 'vue'
import { Button } from '@/components/ui/button'
import { Copy, Check } from '@lucide/vue'

const props = defineProps<{
  value: string
  variant?: 'default' | 'ghost' | 'outline'
  size?: 'default' | 'sm' | 'lg' | 'icon'
}>()

const copied = ref(false)

const copyToClipboard = async () => {
  try {
    await navigator.clipboard.writeText(props.value)
    copied.value = true
    setTimeout(() => {
      copied.value = false
    }, 2000)
  } catch (err) {
    console.error('Failed to copy:', err)
  }
}
</script>

<template>
  <Button
    :variant="variant || 'ghost'"
    :size="size || 'icon'"
    @click="copyToClipboard"
    :aria-label="copied ? 'Copied' : 'Copy to clipboard'"
  >
    <Check v-if="copied" class="h-4 w-4 text-green-600" />
    <Copy v-else class="h-4 w-4" />
  </Button>
</template>
