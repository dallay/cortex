<script setup lang="ts">
import {AlertCircle} from "@lucide/vue";
import CopyButton from "@/components/CopyButton.vue";
import {Alert, AlertDescription} from "@/components/ui/alert";
import {Button} from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";

defineProps<{
  apiKey: string;
  title?: string;
  description?: string;
  warning?: string;
  onDone?: () => void;
}>();
</script>

<template>
  <Card>
    <CardHeader>
      <CardTitle>{{ title || 'API Key Created' }}</CardTitle>
      <CardDescription v-if="description">
        {{ description }}
      </CardDescription>
    </CardHeader>
    <CardContent class="space-y-4">
      <Alert v-if="warning" variant="destructive">
        <AlertCircle class="h-4 w-4" />
        <AlertDescription>
          {{ warning }}
        </AlertDescription>
      </Alert>
      <div class="flex items-center gap-2 p-3 bg-muted rounded-md font-mono text-sm">
        <code data-testid="api-key-display" class="flex-1 break-words overflow-x-auto">{{ apiKey }}</code>
        <CopyButton :value="apiKey" />
      </div>
    </CardContent>
    <CardFooter v-if="onDone" class="justify-end">
      <Button @click="onDone">Done</Button>
    </CardFooter>
  </Card>
</template>
