<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {MenubarSeparatorProps} from "reka-ui";
import {MenubarSeparator, useForwardProps} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  MenubarSeparatorProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");

const forwardedProps = useForwardProps(delegatedProps);
</script>

<template>
  <MenubarSeparator
    :class=" cn('bg-border -mx-1 my-1 h-px', props.class)"
    data-slot="menubar-separator"
    v-bind="forwardedProps"
  />
</template>
