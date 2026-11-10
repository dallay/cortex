<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <div
    :class="cn('flex basis-full items-center justify-between gap-2', props.class)"
    data-slot="item-footer"
  >
    <slot/>
  </div>
</template>
