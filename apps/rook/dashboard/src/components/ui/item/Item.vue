<script lang="ts" setup>
import type {PrimitiveProps} from "reka-ui";
import {Primitive} from "reka-ui";
import type {HTMLAttributes} from "vue";
import {cn} from "@/lib/utils";
import type {ItemVariants} from ".";
import {itemVariants} from ".";

const props = withDefaults(
  defineProps<
    PrimitiveProps & {
    class?: HTMLAttributes["class"];
    variant?: ItemVariants["variant"];
    size?: ItemVariants["size"];
  }
  >(),
  {
    as: "div",
  },
);
</script>

<template>
  <Primitive
    :as="as"
    :as-child="asChild"
    :class="cn(itemVariants({ variant, size }), props.class)"
    :data-size="size"
    :data-variant="variant"
    data-slot="item"
  >
    <slot/>
  </Primitive>
</template>
