---
name: vue
description: Use when working with .vue files, composables, Pinia stores, or form validation in the Rook dashboard.
allowed-tools: Read, Edit, Write, Glob, Grep, Bash
metadata:
  author: cortex
  version: "2.0"
---

# Vue 3 — Rook dashboard

Conventions for `apps/rook/dashboard`. They extend
`.agents/contexts/dashboard.md`; global and dashboard instructions still apply.

## Stack (verify before assuming)

| Piece | Value | Source |
|---|---|---|
| Framework | Vue `^3.6.0-rc` | `apps/rook/dashboard/package.json` |
| Router / state | `vue-router` 5, Pinia 4 (setup stores) | same |
| UI | shadcn-vue components in `src/components/ui` | `components.json` |
| Styling | Tailwind CSS 4 + tokens in `src/style.css` | same |
| i18n | vue-i18n, `src/locales/en.json` + `es.json` | same |
| HTTP | native `fetch` behind `useApi()` in `src/lib/api.ts` | same |
| Forms | vee-validate + `@vee-validate/zod` + Zod | same |
| Lint/format | Biome (tabs, double quotes) | `biome.json` |

There is no Axios dependency and no shared `@profiletailors/*` package. Do not
reintroduce either.

## Component structure

Use `<script setup lang="ts">` with typed props and emits:

```vue
<script setup lang="ts">
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { Button } from "@/components/ui/button";

type Props = {
	title: string;
	count?: number;
};

const props = withDefaults(defineProps<Props>(), { count: 0 });
const emit = defineEmits<{
	update: [value: number];
	close: [];
}>();

const { t } = useI18n();
const localCount = ref(props.count);
const doubled = computed(() => localCount.value * 2);

const increment = () => {
	localCount.value++;
	emit("update", localCount.value);
};
</script>

<template>
	<div>
		<h2>{{ title }}</h2>
		<p>{{ t("common.count", { count: localCount, doubled }) }}</p>
		<Button type="button" @click="increment">{{ t("common.increment") }}</Button>
	</div>
</template>
```

- Every visible string comes from vue-i18n; update both locale files in the
  same change.
- Use `@/components/ui/<component>` imports (for example
  `@/components/ui/button`). Do not import from a UI package.
- Reuse `LoadingState`, `EmptyState` and `ErrorBanner`; every view needs
  loading, empty, error and denied states where applicable.

## Pinia: setup stores

Stores are functions with refs, computed values and actions — not the Options
API object form:

```ts
import { computed, ref } from "vue";
import { defineStore } from "pinia";
import { useApi } from "@/lib/api";

export const useExampleStore = defineStore("example", () => {
	const api = useApi();
	const items = ref<string[]>([]);
	const isLoading = ref(false);
	const error = ref<string | null>(null);

	const isEmpty = computed(() => items.value.length === 0);

	async function load(): Promise<void> {
		isLoading.value = true;
		error.value = null;
		try {
			const providers = await api.getProviders();
			items.value = providers.map((provider) => provider.name);
		} catch (value) {
			error.value = value instanceof Error ? value.message : "Request failed";
		} finally {
			isLoading.value = false;
		}
	}

	return { items, isLoading, error, isEmpty, load };
});
```

- Keep API calls in `useApi()`/`src/lib/api.ts`; components and stores do not
  assemble URLs or headers.
- The management cookie and CSRF token are handled by the API layer. Never
  duplicate or bypass that logic in a component.

## Composables

Prefix with `use`, return refs/computed values, and clean up side effects:

```ts
import { computed, onMounted, onUnmounted, ref, type ComputedRef, type Ref } from "vue";

type UseCounter = {
	count: Ref<number>;
	doubled: ComputedRef<number>;
	increment: () => void;
};

export const useCounter = (initial = 0): UseCounter => {
	const count = ref(initial);
	const doubled = computed(() => count.value * 2);
	const onResize = () => undefined;

	onMounted(() => window.addEventListener("resize", onResize));
	onUnmounted(() => window.removeEventListener("resize", onResize));

	return { count, doubled, increment: () => { count.value++; } };
};
```

## Forms (Vee-Validate + Zod)

Validate on submit or blur; do not validate on mount, which produces noisy
errors before the user interacts:

```vue
<script setup lang="ts">
import { toTypedSchema } from "@vee-validate/zod";
import { useForm } from "vee-validate";
import { z } from "zod";
import { Button } from "@/components/ui/button";
import {
	FormControl,
	FormField,
	FormItem,
	FormLabel,
	FormMessage,
} from "@/components/ui/form";
import { Input } from "@/components/ui/input";

const schema = z.object({
	email: z.string().email(),
	password: z.string().min(8),
});

const { handleSubmit, validateField } = useForm({
	validationSchema: toTypedSchema(schema),
	validateOnMount: false,
});

const onSubmit = handleSubmit(async (values) => {
	await save(values);
});
</script>

<template>
	<form @submit="onSubmit">
		<FormField v-slot="{ componentField }" name="email">
			<FormItem>
				<FormLabel>Email</FormLabel>
				<FormControl>
					<Input v-bind="componentField" @blur="validateField('email')" />
				</FormControl>
				<FormMessage />
			</FormItem>
		</FormField>
		<Button type="submit">Save</Button>
	</form>
</template>
```

## Communication and performance

| Need | Use |
|---|---|
| Parent → child | props |
| Child → parent | typed `emit` |
| Shared/distant state | Pinia store |
| Deeply nested, stable tree | `provide`/`inject` |

- Do not use global event buses.
- Use `computed` instead of methods in templates; `shallowRef` for large
  structures that do not need deep reactivity.
- Remove listeners, intervals and observers in `onUnmounted`.

## Commands

From the repository root:

```bash
pnpm --filter @dallay/rook-dashboard dev
pnpm --filter @dallay/rook-dashboard run build
pnpm --filter @dallay/rook-dashboard run typecheck
pnpm --filter @dallay/rook-dashboard run lint
pnpm --filter @dallay/rook-dashboard exec vitest run src/components/Foo.spec.ts
pnpm --filter @dallay/rook-dashboard exec playwright test e2e/login.spec.ts
```

## Anti-patterns

- Options API or untyped `defineProps`/`defineEmits`.
- `any`; use `unknown` plus narrowing or a schema.
- Mutating props, or copying them into state without a reason.
- Importing `@profiletailors/ui`, `axios` or any package that is not installed.
- Hardcoded user-facing text instead of i18n keys.
- Duplicating CSRF, cookie or URL handling outside `src/lib/api.ts`.
- Editing generated shadcn-vue internals when the wrapper should be composed
  instead; keep `src/components/ui` reusable and product-agnostic.

## Resources

- [Vue 3](https://vuejs.org/guide/)
- [Pinia](https://pinia.vuejs.org/)
- [Vee-Validate + Zod](https://vee-validate.logaretm.com/v4/integrations/zod-schema-validation/)
- [shadcn-vue](https://www.shadcn-vue.com/)
