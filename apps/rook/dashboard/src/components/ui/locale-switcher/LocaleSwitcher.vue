<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { setLocale } from '@/i18n'

const { locale } = useI18n()

const locales = [
  { code: 'en', label: 'EN' },
  { code: 'es', label: 'ES' },
] as const

function handleChange(newLocale: 'en' | 'es') {
  setLocale(newLocale)
}
</script>

<template>
  <div class="flex items-center gap-1">
    <button
      v-for="loc in locales"
      :key="loc.code"
      @click="handleChange(loc.code)"
      class="px-1.5 py-0.5 text-xs font-medium rounded transition-colors"
      :class="locale === loc.code
        ? 'bg-sidebar-accent text-sidebar-accent-foreground'
        : 'text-muted-foreground hover:text-foreground'"
    >
      {{ loc.label }}
    </button>
  </div>
</template>