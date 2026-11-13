<script lang="ts" setup>
import {X} from "@lucide/vue";
import {reactiveOmit} from "@vueuse/core";
import type {TagsInputItemDeleteProps} from "reka-ui";
import {TagsInputItemDelete, useForwardProps} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";

const props = defineProps<
  TagsInputItemDeleteProps & { class?: HTMLAttributes["class"] }
>();

const delegatedProps = reactiveOmit(props, "class");

const forwardedProps = useForwardProps(delegatedProps);
</script>

<template>
  <TagsInputItemDelete :class="cn('flex rounded bg-transparent mr-1', props.class)"
                       v-bind="forwardedProps">
    <slot>
      <X class="w-4 h-4"/>
    </slot>
  </TagsInputItemDelete>
</template>
