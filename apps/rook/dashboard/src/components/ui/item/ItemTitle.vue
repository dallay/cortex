<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <div
    :class="cn('flex w-fit items-center gap-2 text-sm leading-snug font-medium', props.class)"
    data-slot="item-title"
  >
    <slot/>
  </div>
</template>
