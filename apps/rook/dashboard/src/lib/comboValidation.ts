/**
 * Client-side validation for combo drafts.
 *
 * The rules mirror `Combo::validate` in `crates/domain/rook-core/src/model.rs`
 * and `CreateComboRequest` in `crates/infrastructure/transport-axum/src/combo_dto.rs`.
 * Keeping them in sync avoids a round-trip for problems the user can see
 * locally; the backend stays the authority and still validates every request.
 *
 * One deliberate difference: the client trims the name before checking it, so
 * a whitespace-only name is rejected here. The backend does not trim, so
 * `toCreateComboRequest` trims before sending to keep both sides consistent.
 */
import type { CreateComboRequest } from "@/lib/api";

export interface ComboStepDraft {
	providerId: string;
	model: string;
	connectionId: string | null;
	priority: number;
}

export interface ComboDraft {
	name: string;
	steps: ComboStepDraft[];
}

export type ComboValidationCode =
	| "name_required"
	| "name_too_long"
	| "steps_required"
	| "steps_too_many"
	| "step_provider_required"
	| "step_model_required"
	| "step_priority_invalid"
	| "step_priority_duplicate";

export interface ComboValidationIssue {
	code: ComboValidationCode;
	stepIndex?: number;
	priority?: number;
}

export const COMBO_NAME_MAX_LENGTH = 100;
export const COMBO_MAX_STEPS = 10;
export const COMBO_PRIORITY_MIN = 1;
export const COMBO_PRIORITY_MAX = 255;

export function emptyComboStep(priority = COMBO_PRIORITY_MIN): ComboStepDraft {
	return { providerId: "", model: "", connectionId: null, priority };
}

export function emptyComboDraft(): ComboDraft {
	return { name: "", steps: [emptyComboStep()] };
}

/**
 * Returns every independent issue in the draft, in a stable order:
 * name, step count, then per-step provider, model and priority.
 */
export function validateComboDraft(draft: ComboDraft): ComboValidationIssue[] {
	const issues: ComboValidationIssue[] = [];
	const name = draft.name.trim();

	if (name.length === 0) {
		issues.push({ code: "name_required" });
	} else if (name.length > COMBO_NAME_MAX_LENGTH) {
		issues.push({ code: "name_too_long" });
	}

	if (draft.steps.length === 0) {
		issues.push({ code: "steps_required" });
	} else if (draft.steps.length > COMBO_MAX_STEPS) {
		issues.push({ code: "steps_too_many" });
	}

	const seenPriorities = new Set<number>();
	for (const [stepIndex, step] of draft.steps.entries()) {
		if (step.providerId.trim().length === 0) {
			issues.push({ code: "step_provider_required", stepIndex });
		}
		if (step.model.trim().length === 0) {
			issues.push({ code: "step_model_required", stepIndex });
		}

		const validPriority =
			Number.isInteger(step.priority) &&
			step.priority >= COMBO_PRIORITY_MIN &&
			step.priority <= COMBO_PRIORITY_MAX;

		if (!validPriority) {
			issues.push({ code: "step_priority_invalid", stepIndex });
			continue;
		}

		if (seenPriorities.has(step.priority)) {
			issues.push({
				code: "step_priority_duplicate",
				stepIndex,
				priority: step.priority,
			});
		} else {
			seenPriorities.add(step.priority);
		}
	}

	return issues;
}

/**
 * Maps a draft to the wire format. `strategy` is pinned to `priority`, the only
 * value `parse_combo_strategy` accepts today.
 */
export function toCreateComboRequest(draft: ComboDraft): CreateComboRequest {
	return {
		name: draft.name.trim(),
		strategy: "priority",
		steps: draft.steps.map((step) => ({
			providerId: step.providerId,
			model: step.model,
			connectionId:
				step.connectionId && step.connectionId.length > 0
					? step.connectionId
					: null,
			priority: step.priority,
		})),
	};
}
