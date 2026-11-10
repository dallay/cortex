<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {CalendarHeadCellProps} from "reka-ui";
import {CalendarHeadCell, useForwardProps} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  CalendarHeadCellProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");

const forwardedProps = useForwardProps(delegatedProps);
</script>

<template>
  <CalendarHeadCell
    :class="cn('text-muted-foreground rounded-md flex-1 font-normal text-[0.8rem]', props.class)"
    data-slot="calendar-head-cell"
    v-bind="forwardedProps"
  >
    <slot/>
  </CalendarHeadCell>
</template>
