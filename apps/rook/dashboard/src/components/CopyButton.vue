<script setup lang="ts">
import {Check, Copy} from "@lucide/vue";
import {computed, ref} from "vue";
import {Button} from "@/components/ui/button";

const props = withDefaults(
  defineProps<{
    value?: string;
    text?: string;
    variant?: "default" | "ghost" | "outline";
    size?: "default" | "sm" | "lg" | "icon";
  }>(),
  {
    value: undefined,
    text: undefined,
  },
);

const copied = ref(false);

const copyValue = computed(() => props.value ?? props.text ?? "");

const copyToClipboard = async () => {
  const text = copyValue.value;
  if (!text) return;

  // navigator.clipboard requires HTTPS or localhost
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      copied.value = true;
      setTimeout(() => {
        copied.value = false;
      }, 2000);
      return;
    } catch {
      // Fall through to fallback
    }
  }

  // DOM fallback for non-HTTPS or older browsers
  try {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    document.body.appendChild(textarea);
    textarea.select();
    document.execCommand("copy");
    textarea.remove();
    copied.value = true;
    setTimeout(() => {
      copied.value = false;
    }, 2000);
  } catch (err) {
    console.error("Failed to copy:", err);
  }
};
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
