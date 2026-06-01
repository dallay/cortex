import { defineStore } from 'pinia'
import { ref, watch } from 'vue'

type Theme = 'light' | 'dark'

export const useThemeStore = defineStore('theme', () => {
  const current = ref<Theme>('light')

  function setTheme(theme: Theme) {
    current.value = theme
    applyTheme(theme)
  }

  function toggleTheme() {
    setTheme(current.value === 'dark' ? 'light' : 'dark')
  }

  function applyTheme(theme: Theme) {
    const root = document.documentElement
    if (theme === 'dark') {
      root.classList.add('dark')
    } else {
      root.classList.remove('dark')
    }
  }

  // Initialize from localStorage or system preference
  function init() {
    const stored = localStorage.getItem('theme') as Theme | null
    if (stored) {
      setTheme(stored)
    } else if (globalThis.matchMedia('(prefers-color-scheme: dark)').matches) {
      setTheme('dark')
    }
  }

  // Persist to localStorage on change
  watch(current, (theme) => {
    localStorage.setItem('theme', theme)
  })

  return {
    current,
    setTheme,
    toggleTheme,
    init,
  }
})