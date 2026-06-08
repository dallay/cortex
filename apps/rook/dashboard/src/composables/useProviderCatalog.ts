/**
 * useProviderCatalog — derived composable that joins the static
 * `PROVIDER_KINDS` catalog with the live connection list and the
 * available-models list.
 *
 * Per the spec (`REQ-6 Catalog Metadata Source`): the catalog view
 * derives kind-level metadata (displayName, defaultBaseUrl, icon,
 * description) from the static `PROVIDER_KINDS` constant, and
 * connection counts from `GET /api/providers`. Model ids come from
 * `GET /api/models` (joined with active connections inside
 * `useAvailableModels`).
 *
 * The composable is **read-only** — it does not fetch. Callers
 * (e.g. `ProvidersView`) are responsible for invoking
 * `useProviders().fetch()` and `useAvailableModels().fetch()` on
 * mount. Before either fetch resolves, `connectionCount` is 0 and
 * `configuredModels` is empty.
 */
import {type ComputedRef, computed} from "vue";
import {useAvailableModels} from "@/composables/useAvailableModels";
import {useProviders} from "@/composables/useProviders";
import {
  type CatalogEntry,
  type CategoryKind,
  PROVIDER_KINDS,
  type ProviderKind,
} from "@/config/providerCatalog";

/** A catalog entry enriched with live, per-kind derived data. */
export interface ProviderCatalogItem extends CatalogEntry {
  /** Number of connections of this kind (all states). */
  readonly connectionCount: number;
  /** Whether at least one connection of this kind is currently active. */
  readonly hasActiveConnections: boolean;
  /**
   * Models configured across active connections of this kind.
   * Deduped. Empty until `useAvailableModels().fetch()` resolves.
   */
  readonly configuredModels: readonly string[];
}

export interface UseProviderCatalog {
  /**
   * All catalog items, one per `ProviderKind`, in the order defined
   * by `PROVIDER_KINDS`. Reactively updates when the underlying
   * connection list or models list changes.
   */
  readonly items: ComputedRef<readonly ProviderCatalogItem[]>;
  /** Filtered view: items belonging to a single category. */
  byCategory: (
    category: CategoryKind,
  ) => ComputedRef<readonly ProviderCatalogItem[]>;
  /** Find a single item by its `ProviderKind`. Returns `null` if absent. */
  byKind: (kind: ProviderKind) => ComputedRef<ProviderCatalogItem | null>;
}

export function useProviderCatalog(): UseProviderCatalog {
  const {providers} = useProviders();
  const {modelsByProvider} = useAvailableModels();

  /**
   * Pre-index the available-models output by `providerKind` so the
   * `items` computed can aggregate models in O(1) per kind. The index
   * is rebuilt whenever `modelsByProvider` changes.
   */
  const modelsByKind = computed<ReadonlyMap<ProviderKind, readonly string[]>>(
    () => {
      const acc = new Map<ProviderKind, Set<string>>();
      for (const entry of modelsByProvider.value) {
        const kind = entry.provider.providerKind;
        const bucket = acc.get(kind) ?? new Set<string>();
        for (const model of entry.models) {
          bucket.add(model);
        }
        acc.set(kind, bucket);
      }
      const out = new Map<ProviderKind, readonly string[]>();
      for (const [kind, set] of acc) {
        out.set(kind, [...set].sort());
      }
      return out;
    },
  );

  const items = computed<readonly ProviderCatalogItem[]>(() => {
    const providersByKind = new Map<ProviderKind, number>();
    const activeByKind = new Map<ProviderKind, boolean>();
    for (const p of providers.value) {
      const kind = p.providerKind;
      providersByKind.set(kind, (providersByKind.get(kind) ?? 0) + 1);
      if (p.isActive) {
        activeByKind.set(kind, true);
      }
    }
    return PROVIDER_KINDS.map((entry) => ({
      ...entry,
      connectionCount: providersByKind.get(entry.kind) ?? 0,
      hasActiveConnections: activeByKind.get(entry.kind) ?? false,
      configuredModels: modelsByKind.value.get(entry.kind) ?? [],
    }));
  });

  return {
    items,
    byCategory: (category) =>
      computed(() => items.value.filter((i) => i.category === category)),
    byKind: (kind) =>
      computed(() => items.value.find((i) => i.kind === kind) ?? null),
  };
}
