<script setup lang="ts">
import { ref } from 'vue'
import { useThemeStore } from '@/stores/theme'

const theme = useThemeStore()
const isAnimating = ref(false)

function animateThemeChange(next: 'dark' | 'light') {
  // Graceful degradation — View Transitions not supported in Firefox < 126, Safari < 18
  if (!document.startViewTransition) {
    theme.setTheme(next)
    return
  }

  // Respect reduced motion — skip the clip-path wipe
  if (globalThis.matchMedia('(prefers-reduced-motion: reduce)').matches) {
    theme.setTheme(next)
    return
  }

  isAnimating.value = true
  const transition = document.startViewTransition(() => {
    theme.setTheme(next)
  })

  transition.ready.then(() => {
    document.documentElement.animate(
      { clipPath: ['inset(0 0 100% 0)', 'inset(0)'] },
      {
        duration: 600,
        pseudoElement: '::view-transition-new(root)',
      },
    )
  })

  transition.finished.then(() => {
    isAnimating.value = false
  })
}

function handleClick() {
  animateThemeChange(theme.current === 'dark' ? 'light' : 'dark')
}
</script>

<template>
  <button
    @click="handleClick"
    :disabled="isAnimating"
    class="p-1.5 hover:bg-accent transition-colors rounded-md border border-border focus:outline-none focus:ring-1 focus:ring-ring cursor-pointer disabled:opacity-50"
    :title="theme.current === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'"
  >
    <!-- Sun icon when dark (click to go light), Moon icon when light (click to go dark) -->
    <svg
      v-if="theme.current === 'dark'"
      role="img"
      aria-label="Sun"
      class="size-4"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      viewBox="0 0 24 24"
    >
      <path
        stroke-linecap="round"
        stroke-linejoin="round"
        d="M12 3v2.25m0 13.5V21M5.136 5.136l1.591 1.591m9.09 9.09l1.591 1.591M3 12h2.25m13.5 0H21M5.136 18.864l1.591-1.591m9.09-9.09l1.591-1.591M12 7.5a4.5 4.5 0 1 1 0 9 4.5 4.5 0 0 1 0-9Z"
      />
    </svg>
    <svg
      v-else
      role="img"
      aria-label="Moon"
      class="size-4"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      viewBox="0 0 24 24"
    >
      <path
        stroke-linecap="round"
        stroke-linejoin="round"
        d="M21.752 15.002A9.72 9.72 0 0 1 18 15.75c-5.385 0-9.75-4.365-9.75-9.75 0-1.33.266-2.597.748-3.752A9.753 9.753 0 0 0 3 11.25C3 16.635 7.365 21 12.75 21a9.753 9.753 0 0 0 9.002-5.998Z"
      />
    </svg>
  </button>
</template>