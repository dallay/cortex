<script setup lang="ts">
import { ref } from 'vue'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { Eye, EyeOff } from '@lucide/vue'

defineProps<{
  id?: string
  type?: 'current-password' | 'new-password'
  required?: boolean
  error?: boolean
  modelValue?: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const showPassword = ref(false)

const toggleVisibility = () => {
  showPassword.value = !showPassword.value
}
</script>

<template>
  <div class="relative">
    <Input
      :id="id"
      :type="showPassword ? 'text' : 'password'"
      :autocomplete="type"
      :required="required"
      :class="{ 'border-destructive': error }"
      :value="modelValue"
      @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
      class="pr-10"
    />
    <Button
      type="button"
      variant="ghost"
      size="sm"
      class="absolute right-0 top-0 h-full px-3 py-2 hover:bg-transparent"
      @click="toggleVisibility"
      :aria-label="showPassword ? 'Hide password' : 'Show password'"
    >
      <Eye v-if="!showPassword" class="h-4 w-4 text-muted-foreground" />
      <EyeOff v-else class="h-4 w-4 text-muted-foreground" />
    </Button>
  </div>
</template>
