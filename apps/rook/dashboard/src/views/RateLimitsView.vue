<script setup lang="ts">
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";
import ErrorBanner from "@/components/ErrorBanner.vue";
import LoadingState from "@/components/LoadingState.vue";
import PageHeader from "@/components/PageHeader.vue";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { NativeSelect, NativeSelectOption } from "@/components/ui/native-select";
import { useRateLimits } from "@/composables/useRateLimits";
import type { RateLimitScope } from "@/lib/api";

const { t } = useI18n();
const { rules, status, loading, saving, error, availability, fetch, create, update, remove, fetchStatus } = useRateLimits();
const scope = ref<RateLimitScope>("api-key");
const target = ref("");
const requestsPerMinute = ref(60);
const requestsPerDay = ref<number | "" | undefined>(undefined);
const statusScope = ref<RateLimitScope>("api-key");
const statusTarget = ref("");
const actionError = ref<string | null>(null);

async function submit() {
	actionError.value = null;
	const perDay = requestsPerDay.value === "" ? undefined : requestsPerDay.value;
	if (
		!Number.isInteger(requestsPerMinute.value) ||
		requestsPerMinute.value < 1 ||
		(perDay != null && (!Number.isInteger(perDay) || perDay < 1))
	) {
		actionError.value = t("rateLimits.invalidLimit");
		return;
	}
	const ok = await create({
		scope: scope.value,
		target: target.value.trim(),
		requests_per_minute: requestsPerMinute.value,
		requests_per_day: perDay,
	});
	if (ok) target.value = "";
	else actionError.value = error.value;
}
async function deleteRule(id: string) {
	actionError.value = null;
	if (!globalThis.confirm(t("rateLimits.deleteConfirm"))) return;
	if (!(await remove(id))) actionError.value = error.value;
}
function checkStatus() {
	actionError.value = null;
	void fetchStatus(statusScope.value, statusTarget.value.trim());
}
async function changeRule(id: string, current: number) {
	actionError.value = null;
	const value = globalThis.prompt(t("rateLimits.changeRpm"), String(current));
	if (value == null) return;
	const numeric = Number(value);
	if (!Number.isInteger(numeric) || numeric < 1) { actionError.value = t("rateLimits.invalidLimit"); return; }
	if (!(await update(id, { requests_per_minute: numeric }))) actionError.value = error.value;
}
onMounted(() => {
	void fetch().then(() => {
		const first = rules.value[0];
		if (first) void fetchStatus(first.scope, first.target);
	});
});
</script>

<template>
	<div class="space-y-6">
		<PageHeader :title="t('rateLimits.title')" :description="t('rateLimits.description')" />
		<div v-if="availability === 'unavailable'" role="status" class="rounded-lg border bg-muted p-4">{{ t("rateLimits.unavailable") }}</div>
		<ErrorBanner v-else-if="error && !actionError" :message="error" />
		<ErrorBanner v-if="actionError" :message="actionError" />
		<LoadingState v-if="loading" :message="t('common.loading')" />
		<div v-else-if="availability === 'available'" class="space-y-6">
			<div class="overflow-x-auto rounded-lg border"><table class="w-full text-sm"><thead><tr class="border-b text-left"><th class="p-3">{{ t("rateLimits.scope") }}</th><th class="p-3">{{ t("rateLimits.target") }}</th><th class="p-3">{{ t("rateLimits.perMinute") }}</th><th class="p-3">{{ t("rateLimits.perDay") }}</th><th class="p-3">{{ t("common.actions") }}</th></tr></thead><tbody><tr v-for="rule in rules" :key="rule.id" class="border-b"><td class="p-3">{{ rule.scope }}</td><td class="p-3">{{ rule.target }}</td><td class="p-3">{{ rule.requests_per_minute }}</td><td class="p-3">{{ rule.requests_per_day ?? "—" }}</td><td class="flex gap-2 p-3"><Button type="button" variant="outline" :disabled="saving" @click="changeRule(rule.id, rule.requests_per_minute)">{{ t("common.edit") }}</Button><Button type="button" variant="destructive" :disabled="saving" @click="deleteRule(rule.id)">{{ t("common.delete") }}</Button></td></tr><tr v-if="!rules.length"><td colspan="5" class="p-6 text-center text-muted-foreground">{{ t("rateLimits.empty") }}</td></tr></tbody></table></div>
			<form class="grid gap-3 rounded-lg border p-4 md:grid-cols-4" @submit.prevent="submit"><label for="rate-limit-scope" class="text-sm">{{ t("rateLimits.scope") }}<select id="rate-limit-scope" v-model="scope" class="h-9 w-full rounded-md border bg-transparent px-3 text-sm"><option value="api-key">api-key</option><option value="ip-address">ip-address</option></select></label><label for="rate-limit-target" class="text-sm">{{ t("rateLimits.target") }}<input id="rate-limit-target" v-model="target" required class="h-9 w-full rounded-md border bg-transparent px-3 text-sm" /></label><label for="rate-limit-per-minute" class="text-sm">{{ t("rateLimits.perMinute") }}<input id="rate-limit-per-minute" v-model.number="requestsPerMinute" type="number" min="1" required class="h-9 w-full rounded-md border bg-transparent px-3 text-sm" /></label><label for="rate-limit-per-day" class="text-sm">{{ t("rateLimits.perDay") }}<input id="rate-limit-per-day" v-model.number="requestsPerDay" type="number" min="1" class="h-9 w-full rounded-md border bg-transparent px-3 text-sm" /></label><Button type="submit" :disabled="saving" class="md:col-span-4 md:justify-self-end">{{ t("rateLimits.create") }}</Button></form>
			<section class="space-y-3 rounded-lg border p-4"><h2 class="font-medium">{{ t("rateLimits.statusTitle") }}</h2><div class="grid gap-3 md:grid-cols-3"><NativeSelect v-model="statusScope"><NativeSelectOption value="api-key">api-key</NativeSelectOption><NativeSelectOption value="ip-address">ip-address</NativeSelectOption></NativeSelect><Input v-model="statusTarget" :placeholder="t('rateLimits.target')" /><Button type="button" variant="outline" @click="checkStatus">{{ t("rateLimits.checkStatus") }}</Button></div><p v-if="status" role="status" class="text-sm">{{ t("rateLimits.current") }}: {{ status.current_minute_count }} · {{ t("rateLimits.remaining") }}: {{ status.remaining_minute }} · {{ t("rateLimits.reset") }}: {{ new Date(status.reset_at).toLocaleString() }}</p></section>
		</div>
	</div>
</template>
