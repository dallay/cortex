<script lang="ts" setup>
import type {PrimitiveProps} from "reka-ui";
import {Primitive} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";
import type {ButtonVariants} from ".";
import {buttonVariants} from ".";

interface Props extends PrimitiveProps {
  variant?: ButtonVariants["variant"];
  size?: ButtonVariants["size"];
  class?: HTMLAttributes["class"];
}

const props = withDefaults(defineProps<Props>(), {
  as: "button",
});
</script>

<template>
  <Primitive
    :as="as"
    :as-child="asChild"
    :class="cn(buttonVariants({ variant, size }), props.class)"
    :data-size="size"
    :data-variant="variant"
    data-slot="button"
  >
    <slot/>
  </Primitive>
</template>
