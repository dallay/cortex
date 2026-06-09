<script setup lang="ts">
import {computed} from "vue";
import {Badge} from "@/components/ui/badge";

const props = defineProps<{
  status: string;
}>();

const variant = computed(() => {
  const status = props.status.toLowerCase();
  switch (status) {
    case "active":
    case "enabled":
    case "connected":
      return "default";
    case "inactive":
    case "disabled":
    case "disconnected":
      return "secondary";
    case "error":
    case "failed":
      return "destructive";
    default:
      return "outline";
  }
});

const colorClass = computed(() => {
  const status = props.status.toLowerCase();
  switch (status) {
    case "active":
    case "enabled":
    case "connected":
      return "bg-green-100 text-green-800 hover:bg-green-100";
    case "inactive":
    case "disabled":
    case "disconnected":
      return "bg-gray-100 text-gray-800 hover:bg-gray-100";
    case "error":
    case "failed":
      return "";
    default:
      return "";
  }
});
</script>

<template>
  <Badge :variant="variant" :class="colorClass">
    {{ status }}
  </Badge>
</template>
