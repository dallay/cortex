<script lang="ts" setup>
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";
import type {FieldVariants} from ".";
import {fieldVariants} from ".";

const props = defineProps<{
  class?: HTMLAttributes["class"];
  orientation?: FieldVariants["orientation"];
}>();
</script>

<template>
  <div
    :class="cn(
      fieldVariants({ orientation }),
      props.class,
    )"
    :data-orientation="orientation"
    data-slot="field"
    role="group"
  >
    <slot/>
  </div>
</template>
