<script lang="ts" setup>
import {Minus} from "@lucide/vue";
import {reactiveOmit} from "@vueuse/core";
import type {NumberFieldDecrementProps} from "reka-ui";
import {NumberFieldDecrement, useForwardProps} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  NumberFieldDecrementProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");

const forwarded = useForwardProps(delegatedProps);
</script>

<template>
  <NumberFieldDecrement
    :class="cn('absolute top-1/2 -translate-y-1/2 left-0 p-3 disabled:cursor-not-allowed disabled:opacity-20', props.class)"
    data-slot="decrement"
    v-bind="forwarded">
    <slot>
      <Minus class="h-4 w-4"/>
    </slot>
  </NumberFieldDecrement>
</template>
