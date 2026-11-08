<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
}>();
</script>

<template>
  <div
    :class="cn('group/item-group flex flex-col', props.class)"
    data-slot="item-group"
    role="list"
  >
    <slot/>
  </div>
</template>
