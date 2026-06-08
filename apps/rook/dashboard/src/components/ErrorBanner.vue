<script setup lang="ts">
import {AlertCircle} from "@lucide/vue";
import {Alert, AlertDescription} from "@/components/ui/alert";

defineProps<{
  message: string;
}>();

const emit = defineEmits<{
  retry: [];
}>();
</script>

<template>
  <Alert variant="destructive" class="mb-4">
    <AlertCircle class="h-4 w-4" />
    <AlertDescription class="flex items-center justify-between gap-3">
      {{ message }}
      <button
        v-if="!$slots.default"
        class="shrink-0 text-xs underline cursor-pointer"
        type="button"
        @click="emit('retry')"
      >
        Retry
      </button>
      <slot v-else />
    </AlertDescription>
  </Alert>
</template>
