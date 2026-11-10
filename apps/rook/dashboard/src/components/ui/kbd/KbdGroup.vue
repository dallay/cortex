<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <kbd
    :class="cn('inline-flex items-center gap-1', props.class)"
    data-slot="kbd-group"
  >
    <slot/>
  </kbd>
</template>
