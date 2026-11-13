<script lang="ts" setup>
import {reactiveOmit} from "@vueuse/core";
import type {PrimitiveProps} from "reka-ui";
import {Primitive} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";
import type {BadgeVariants} from ".";
import {badgeVariants} from ".";

const props = defineProps<
  PrimitiveProps & {
  variant?: BadgeVariants["variant"];
  class?: HTMLAttributes["class"];
}
>();

const delegatedProps = reactiveOmit(props, "class");
</script>

<template>
  <Primitive
    :class="cn(badgeVariants({ variant }), props.class)"
    data-slot="badge"
    v-bind="delegatedProps"
  >
    <slot/>
  </Primitive>
</template>
