<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {MenubarLabelProps} from "reka-ui";
import {MenubarLabel} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  MenubarLabelProps & { class?: HTMLAttributes["class"]; inset?: boolean }
>();
const delegatedProps = reactiveOmit(props, "class", "inset");
</script>

<template>
  <MenubarLabel
    :class="cn('px-2 py-1.5 text-sm font-medium data-[inset]:pl-8', props.class)"
    :data-inset="inset ? '' : undefined"
    v-bind="delegatedProps"
  >
    <slot/>
  </MenubarLabel>
</template>
