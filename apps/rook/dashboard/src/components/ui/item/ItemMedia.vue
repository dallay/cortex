<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";
import type {ItemMediaVariants} from ".";
import {itemMediaVariants} from ".";

const props = defineProps<{
  class?: HTMLAttributes["class"];
  variant?: ItemMediaVariants["variant"];
}>();
</script>

<template>
  <div
    :class="cn(itemMediaVariants({ variant }), props.class)"
    :data-variant="props.variant"
    data-slot="item-media"
  >
    <slot/>
  </div>
</template>
