import {defineStore} from "pinia";
import {computed, ref} from "vue";
import {useApi} from "@/lib/api";

type CurrentUser = {
  username: string;
  displayName: string;
  email: string;
};

export const useAuthStore = defineStore("auth", () => {
  const api = useApi();

  const isLoading = ref(false);
  const error = ref<string | null>(null);
  const initialized = ref(false);
  const bootstrapRequired = ref(false);
  const initialApiKey = ref<string | null>(null);
  const currentUser = ref<CurrentUser | null>(null);

  const isAuthenticated = computed(() => currentUser.value !== null);

  const setAdminSession = () => {
    currentUser.value = {
      username: "admin",
      displayName: "Rook Admin",
      email: "admin@rook.local",
    };
  };

  const toErrorMessage = (value: unknown): string => {
    return value instanceof Error ? value.message : "Authentication failed";
  };

  async function loadBootstrapStatus(): Promise<void> {
    isLoading.value = true;
    error.value = null;

    try {
      const status = await api.getBootstrapStatus();
      bootstrapRequired.value = !status.isInitialized;
      // NOTE: setupToken is NOT read from status — it is an out-of-band secret
      // printed only to server logs. User pastes it manually into the setup form.

      // If the system is initialized, try to restore session from existing cookie.
      // GET /api/me returns null (catches 401) when unauthenticated.
      if (status.isInitialized && !currentUser.value) {
        const me = await api.getMe();
        if (me) {
          currentUser.value = {
            username: me.username,
            displayName: me.displayName,
            email: "admin@rook.local",
          };
        }
      }

      initialized.value = true;
    } catch (value) {
      error.value = toErrorMessage(value);
      initialized.value = true; // allow navigation to proceed even if status check fails
      throw value; // re-throw so router guard can handle it
    } finally {
      isLoading.value = false;
    }
  }

  async function login(password: string): Promise<void> {
    isLoading.value = true;
    error.value = null;

    try {
      await api.login({username: "admin", password});
      setAdminSession();
    } catch (value) {
      currentUser.value = null;
      error.value = toErrorMessage(value);
      throw value;
    } finally {
      isLoading.value = false;
    }
  }

  async function setupAdminPassword(
    token: string,
    password: string,
  ): Promise<void> {
    const trimmed = token.trim();
    if (!trimmed) {
      const missingToken = new Error("Setup token is missing");
      error.value = missingToken.message;
      throw missingToken;
    }

    isLoading.value = true;
    error.value = null;

    try {
      const result = await api.setupBootstrap({
        setupToken: trimmed,
        password,
      });
      // Mark bootstrap complete and stash the API key BEFORE calling login,
      // so a login failure does not leave the UI stuck in setup mode.
      bootstrapRequired.value = false;
      initialApiKey.value = result.apiKey;
      await api.login({username: "admin", password});
      setAdminSession();
    } catch (value) {
      currentUser.value = null;
      error.value = toErrorMessage(value);
      throw value;
    } finally {
      isLoading.value = false;
    }
  }

  async function logout(): Promise<void> {
    isLoading.value = true;
    error.value = null;

    try {
      await api.logout();
    } finally {
      currentUser.value = null;
      initialApiKey.value = null;
      isLoading.value = false;
    }
  }

  return {
    isLoading,
    error,
    initialized,
    bootstrapRequired,
    initialApiKey,
    currentUser,
    isAuthenticated,
    loadBootstrapStatus,
    login,
    setupAdminPassword,
    logout,
  };
});
