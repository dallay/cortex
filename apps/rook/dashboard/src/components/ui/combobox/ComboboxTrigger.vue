<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {ComboboxTriggerProps} from "reka-ui";
import {ComboboxTrigger, useForwardProps} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  ComboboxTriggerProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");

const forwarded = useForwardProps(delegatedProps);
</script>

<template>
  <ComboboxTrigger
    :class="cn('', props.class)"
    data-slot="combobox-trigger"
    tabindex="0"
    v-bind="forwarded"
  >
    <slot/>
  </ComboboxTrigger>
</template>
