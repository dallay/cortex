<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <div
    :class="cn(
      'flex max-w-sm flex-col items-center gap-2 text-center',
      props.class,
    )"
    data-slot="empty-header"
  >
    <slot/>
  </div>
</template>
