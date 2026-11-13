<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {ContextMenuLabelProps} from "reka-ui";
import {ContextMenuLabel} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  ContextMenuLabelProps & { class?: HTMLAttributes["class"]; inset?: boolean }
>();

const delegatedProps = reactiveOmit(props, "class");
</script>

<template>
  <ContextMenuLabel
    :class="cn('text-foreground px-2 py-1.5 text-sm font-medium data-[inset]:pl-8', props.class)"
    :data-inset="inset ? '' : undefined"
    data-slot="context-menu-label"
    v-bind="delegatedProps"
  >
    <slot/>
  </ContextMenuLabel>
</template>
