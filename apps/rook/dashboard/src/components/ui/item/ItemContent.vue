<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <div
    :class="cn('flex flex-1 flex-col gap-1 [&+[data-slot=item-content]]:flex-none', props.class)"
    data-slot="item-content"
  >
    <slot/>
  </div>
</template>
