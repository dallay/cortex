<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {TagsInputItemProps} from "reka-ui";
import {TagsInputItem, useForwardProps} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  TagsInputItemProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");

const forwardedProps = useForwardProps(delegatedProps);
</script>

<template>
  <TagsInputItem
    :class="cn('flex h-5 items-center rounded-md bg-secondary data-[state=active]:ring-ring data-[state=active]:ring-2 data-[state=active]:ring-offset-2 ring-offset-background', props.class)"
    v-bind="forwardedProps">
    <slot/>
  </TagsInputItem>
</template>
