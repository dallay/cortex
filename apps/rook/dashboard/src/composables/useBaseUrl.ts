/**
 * Base URL composable for the API endpoints page.
 *
 * Auto-detects from window.location.origin and allows override via localStorage.
 * Override is useful when:
 * - Running behind a reverse proxy with different external URL
 * - Using a custom domain for the API
 */
import {computed, ref} from "vue";

const STORAGE_KEY = "rook-api-base-url";

function getStoredOverride(): string | null {
  if (typeof window === "undefined") return null;
  return localStorage.getItem(STORAGE_KEY);
}

function storeOverride(url: string | null): void {
  if (typeof window === "undefined") return;
  if (url) {
    localStorage.setItem(STORAGE_KEY, url);
  } else {
    localStorage.removeItem(STORAGE_KEY);
  }
}

function detectOrigin(): string {
  if (typeof window !== "undefined") {
    return window.location.origin;
  }
  return "http://127.0.0.1:3773";
}

export function useBaseUrl() {
  const override = ref<string | null>(getStoredOverride());
  const detectedOrigin = ref(detectOrigin());

  const baseUrl = computed(() => {
    if (override.value) {
      return override.value;
    }
    return detectedOrigin.value;
  });

  const fullBaseUrl = computed(() => `${baseUrl.value}/v1`);

  const isOverridden = computed(() => !!override.value);

  function setOverride(url: string | null) {
    override.value = url;
    storeOverride(url);
  }

  function clearOverride() {
    setOverride(null);
  }

  return {
    baseUrl,
    fullBaseUrl,
    isOverridden,
    setOverride,
    clearOverride,
  };
}
