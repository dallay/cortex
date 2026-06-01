import { defineStore } from 'pinia'
import { computed, ref } from 'vue'
import { useApi } from '@/lib/api'

type CurrentUser = {
  username: string
  displayName: string
  email: string
}

export const useAuthStore = defineStore('auth', () => {
  const api = useApi()

  const isLoading = ref(false)
  const error = ref<string | null>(null)
  const bootstrapRequired = ref(false)
  const setupToken = ref<string | null>(null)
  const initialApiKey = ref<string | null>(null)
  const currentUser = ref<CurrentUser | null>(null)

  const isAuthenticated = computed(() => currentUser.value !== null)

  const setAdminSession = () => {
    currentUser.value = {
      username: 'admin',
      displayName: 'Rook Admin',
      email: 'admin@rook.local',
    }
  }

  const toErrorMessage = (value: unknown): string => {
    return value instanceof Error ? value.message : 'Authentication failed'
  }

  async function loadBootstrapStatus(): Promise<void> {
    isLoading.value = true
    error.value = null

    try {
      const status = await api.getBootstrapStatus()
      bootstrapRequired.value = !status.isInitialized
      setupToken.value = status.setupToken
    } catch (value) {
      error.value = toErrorMessage(value)
      throw value
    } finally {
      isLoading.value = false
    }
  }

  async function login(password: string): Promise<void> {
    isLoading.value = true
    error.value = null

    try {
      await api.login({ username: 'admin', password })
      setAdminSession()
    } catch (value) {
      currentUser.value = null
      error.value = toErrorMessage(value)
      throw value
    } finally {
      isLoading.value = false
    }
  }

  async function setupAdminPassword(password: string): Promise<void> {
    if (!setupToken.value) {
      const missingToken = new Error('Setup token is missing')
      error.value = missingToken.message
      throw missingToken
    }

    isLoading.value = true
    error.value = null

    try {
      const result = await api.setupBootstrap({ setupToken: setupToken.value, password })
      initialApiKey.value = result.apiKey
      await api.login({ username: 'admin', password })
      setAdminSession()
      bootstrapRequired.value = false
      setupToken.value = null
    } catch (value) {
      currentUser.value = null
      error.value = toErrorMessage(value)
      throw value
    } finally {
      isLoading.value = false
    }
  }

  async function logout(): Promise<void> {
    isLoading.value = true
    error.value = null

    try {
      await api.logout()
    } finally {
      currentUser.value = null
      isLoading.value = false
    }
  }

  return {
    isLoading,
    error,
    bootstrapRequired,
    setupToken,
    initialApiKey,
    currentUser,
    isAuthenticated,
    loadBootstrapStatus,
    login,
    setupAdminPassword,
    logout,
  }
})
