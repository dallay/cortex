<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {Input} from "@/components/ui/input";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <Input
    :class="cn(
      'flex-1 rounded-none border-0 bg-transparent shadow-none focus-visible:ring-0 dark:bg-transparent',
      props.class,
    )"
    data-slot="input-group-control"
  />
</template>
