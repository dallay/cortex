<script setup lang="ts">
import {cn} from "@/lib/utils";
import type {WithClassAsProps} from "./interface";
import {useCarousel} from "./useCarousel";

defineOptions({
  inheritAttrs: false,
});

const props = defineProps<WithClassAsProps>();

// carouselRef is used in the template via ref="carouselRef"
// @ts-expect-error TS6133
const {orientation, carouselRef} = useCarousel();
</script>

<template>
  <div
    ref="carouselRef"
    data-slot="carousel-content"
    class="overflow-hidden"
  >
    <div
      :class="
        cn(
          'flex',
          orientation === 'horizontal' ? '-ml-4' : '-mt-4 flex-col',
          props.class,
        )"
      v-bind="$attrs"
    >
      <slot />
    </div>
  </div>
</template>
