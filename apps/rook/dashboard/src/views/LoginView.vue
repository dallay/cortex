<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { Lock, Eye, EyeOff, ShieldCheck } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { useAuthStore } from '@/stores/auth'

const { t } = useI18n()
const router = useRouter()
const auth = useAuthStore()

// ── shared ────────────────────────────────────────────────────────────────────
const showPassword = ref(false)
const showConfirm = ref(false)

// ── setup form ────────────────────────────────────────────────────────────────
const setupPassword = ref('')
const setupConfirm = ref('')
const setupError = ref<string | null>(null)

const setupPasswordsMatch = computed(
  () => setupPassword.value === setupConfirm.value || setupConfirm.value === '',
)

async function submitSetup() {
  setupError.value = null

  if (setupPassword.value !== setupConfirm.value) {
    setupError.value = t('setup.error.passwordMismatch')
    return
  }

  if (setupPassword.value.length < 12) {
    setupError.value = t('setup.error.passwordTooShort')
    return
  }

  try {
    await auth.setupAdminPassword(setupPassword.value)
    await router.push({ name: 'Home' })
  } catch {
    setupError.value = auth.error ?? t('setup.error.unknown')
  }
}

// ── login form ────────────────────────────────────────────────────────────────
const loginPassword = ref('')
const loginError = ref<string | null>(null)

async function submitLogin() {
  loginError.value = null

  try {
    await auth.login(loginPassword.value)
    await router.push({ name: 'Home' })
  } catch {
    loginError.value = auth.error ?? t('auth.error.unknown')
  }
}
</script>

<template>
  <div class="min-h-screen bg-background flex items-center justify-center p-4">
    <div class="w-full max-w-sm space-y-6">

      <!-- ── Setup mode ─────────────────────────────────────────────────── -->
      <template v-if="auth.bootstrapRequired">
        <div class="space-y-2 text-center">
          <div class="flex justify-center">
            <ShieldCheck class="h-10 w-10 text-primary" />
          </div>
          <h1 class="text-2xl font-semibold tracking-tight">
            {{ t('setup.title') }}
          </h1>
          <p class="text-sm text-muted-foreground">
            {{ t('setup.description') }}
          </p>
        </div>

        <form class="space-y-4" @submit.prevent="submitSetup">
          <div class="space-y-2">
            <Label for="setup-password">{{ t('setup.field.password') }}</Label>
            <div class="relative">
              <Input
                id="setup-password"
                v-model="setupPassword"
                :type="showPassword ? 'text' : 'password'"
                autocomplete="new-password"
                required
              />
              <button
                type="button"
                class="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                :aria-label="showPassword ? t('auth.hidePassword') : t('auth.showPassword')"
                @click="showPassword = !showPassword"
              >
                <Eye v-if="!showPassword" class="h-4 w-4" />
                <EyeOff v-else class="h-4 w-4" />
              </button>
            </div>
          </div>

          <div class="space-y-2">
            <Label for="setup-confirm">{{ t('setup.field.confirm') }}</Label>
            <Input
              id="setup-confirm"
              v-model="setupConfirm"
              :type="showConfirm ? 'text' : 'password'"
              autocomplete="new-password"
              :class="{ 'border-destructive': !setupPasswordsMatch }"
              required
            />
            <p v-if="!setupPasswordsMatch" class="text-xs text-destructive">
              {{ t('setup.error.passwordMismatch') }}
            </p>
          </div>

          <p v-if="setupError" class="text-sm text-destructive text-center">
            {{ setupError }}
          </p>

          <Button type="submit" class="w-full" :disabled="auth.isLoading">
            <Lock class="mr-2 h-4 w-4" />
            {{ auth.isLoading ? t('setup.submitting') : t('setup.submit') }}
          </Button>
        </form>
      </template>

      <!-- ── Login mode ─────────────────────────────────────────────────── -->
      <template v-else>
        <div class="space-y-2 text-center">
          <div class="flex justify-center">
            <Lock class="h-10 w-10 text-primary" />
          </div>
          <h1 class="text-2xl font-semibold tracking-tight">
            {{ t('auth.title') }}
          </h1>
          <p class="text-sm text-muted-foreground">
            {{ t('auth.description') }}
          </p>
        </div>

        <form class="space-y-4" @submit.prevent="submitLogin">
          <div class="space-y-2">
            <Label for="login-password">{{ t('auth.field.password') }}</Label>
            <div class="relative">
              <Input
                id="login-password"
                v-model="loginPassword"
                :type="showPassword ? 'text' : 'password'"
                autocomplete="current-password"
                required
              />
              <button
                type="button"
                class="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                :aria-label="showPassword ? t('auth.hidePassword') : t('auth.showPassword')"
                @click="showPassword = !showPassword"
              >
                <Eye v-if="!showPassword" class="h-4 w-4" />
                <EyeOff v-else class="h-4 w-4" />
              </button>
            </div>
          </div>

          <p v-if="loginError" class="text-sm text-destructive text-center">
            {{ loginError }}
          </p>

          <Button type="submit" class="w-full" :disabled="auth.isLoading">
            {{ auth.isLoading ? t('auth.submitting') : t('auth.submit') }}
          </Button>
        </form>
      </template>

    </div>
  </div>
</template>
