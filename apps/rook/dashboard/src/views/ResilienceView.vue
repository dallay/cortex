<script setup lang="ts">
import { onMounted } from "vue";
import { useI18n } from "vue-i18n";
import LoadingState from "@/components/LoadingState.vue";
import PageHeader from "@/components/PageHeader.vue";
import { Button } from "@/components/ui/button";
import { useResilience } from "@/composables/useResilience";

const { t } = useI18n();
const { states, loading, error, fetch, reset } = useResilience();
async function resetCircuit(provider: string) { if (globalThis.confirm(t("resilience.resetConfirm", { provider }))) await reset(provider); }
onMounted(() => void fetch());
</script>
<template>
	<div class="space-y-6">
		<PageHeader :title="t('resilience.title')" :description="t('resilience.description')" />
		<div v-if="error" role="alert" class="rounded-lg border border-destructive/50 bg-destructive/10 p-4">{{ error }}</div>
		<LoadingState v-if="loading" :message="t('common.loading')" />
		<div v-else class="overflow-x-auto rounded-lg border"><table class="w-full text-sm"><thead><tr class="border-b text-left"><th class="p-3">{{ t("resilience.provider") }}</th><th class="p-3">{{ t("resilience.state") }}</th><th class="p-3">{{ t("resilience.failures") }}</th><th class="p-3">{{ t("resilience.lastFailure") }}</th><th class="p-3">{{ t("resilience.cooldown") }}</th><th class="p-3">{{ t("resilience.rateLimitReset") }}</th><th class="p-3"></th></tr></thead><tbody><tr v-for="circuit in states" :key="circuit.provider" class="border-b"><td class="p-3">{{ circuit.provider }}</td><td class="p-3"><span class="rounded px-2 py-1" :class="circuit.state === 'open' ? 'bg-destructive/10 text-destructive' : 'bg-green-500/10 text-green-700'">{{ circuit.state }}</span></td><td class="p-3">{{ circuit.failures }}</td><td class="p-3">{{ circuit.last_failure ? new Date(circuit.last_failure).toLocaleString() : "—" }}</td><td class="p-3">{{ circuit.cooldown_until ? new Date(circuit.cooldown_until).toLocaleString() : "—" }}</td><td class="p-3">{{ circuit.rate_limit_reset ? new Date(circuit.rate_limit_reset * 1000).toLocaleString() : "—" }}</td><td class="p-3"><Button type="button" variant="outline" :disabled="circuit.state === 'closed'" @click="resetCircuit(circuit.provider)">{{ t("resilience.reset") }}</Button></td></tr><tr v-if="states.length === 0"><td colspan="7" class="p-6 text-center text-muted-foreground">{{ t("resilience.empty") }}</td></tr></tbody></table></div>
	</div>
</template>
