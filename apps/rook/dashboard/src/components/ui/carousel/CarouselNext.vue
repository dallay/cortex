<script lang="ts" setup>
import {ArrowRight} from "@lucide/vue";
import type {ButtonVariants} from "@/components/ui/button";
import {Button} from "@/components/ui/button";
import {cn} from "@/lib/utils";
import type {WithClassAsProps} from "./interface";
import {useCarousel} from "./useCarousel";

const props = withDefaults(
  defineProps<
    {
      variant?: ButtonVariants["variant"];
      size?: ButtonVariants["size"];
    } & WithClassAsProps
  >(),
  {
    variant: "outline",
    size: "icon",
  },
);

const {orientation, canScrollNext, scrollNext} = useCarousel();
</script>

<template>
  <Button
    :class="cn(
      'absolute size-8 rounded-full',
      orientation === 'horizontal'
        ? 'top-1/2 -right-12 -translate-y-1/2'
        : '-bottom-12 left-1/2 -translate-x-1/2 rotate-90',
      props.class,
    )"
    :disabled="!canScrollNext"
    :size="size"
    :variant="variant"
    data-slot="carousel-next"
    @click="scrollNext"
  >
    <slot>
      <ArrowRight/>
      <span class="sr-only">Next Slide</span>
    </slot>
  </Button>
</template>
