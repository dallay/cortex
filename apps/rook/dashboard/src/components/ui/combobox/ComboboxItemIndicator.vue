<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {ComboboxItemIndicatorProps} from "reka-ui";
import {ComboboxItemIndicator, useForwardProps} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  ComboboxItemIndicatorProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");

const forwarded = useForwardProps(delegatedProps);
</script>

<template>
  <ComboboxItemIndicator
    :class="cn('ml-auto', props.class)"
    data-slot="combobox-item-indicator"
    v-bind="forwarded"
  >
    <slot/>
  </ComboboxItemIndicator>
</template>
