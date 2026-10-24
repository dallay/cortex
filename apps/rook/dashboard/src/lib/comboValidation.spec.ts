import { describe, expect, it } from "vitest";
import {
	COMBO_MAX_STEPS,
	COMBO_NAME_MAX_LENGTH,
	type ComboDraft,
	type ComboStepDraft,
	emptyComboDraft,
	toCreateComboRequest,
	validateComboDraft,
} from "./comboValidation";

function step(overrides: Partial<ComboStepDraft> = {}): ComboStepDraft {
	return {
		providerId: "provider-openai",
		model: "gpt-4o",
		connectionId: null,
		priority: 1,
		...overrides,
	};
}

function draft(overrides: Partial<ComboDraft> = {}): ComboDraft {
	return {
		name: "Production fallback",
		steps: [step()],
		...overrides,
	};
}

describe("validateComboDraft", () => {
	it("accepts a draft that mirrors the backend rules", () => {
		expect(validateComboDraft(draft())).toEqual([]);
	});

	it("requires a non-empty name after trimming", () => {
		expect(validateComboDraft(draft({ name: "" }))).toEqual([
			{ code: "name_required" },
		]);
		expect(validateComboDraft(draft({ name: "   " }))).toEqual([
			{ code: "name_required" },
		]);
	});

	it("rejects names longer than the backend limit", () => {
		const atLimit = "a".repeat(COMBO_NAME_MAX_LENGTH);
		const overLimit = "a".repeat(COMBO_NAME_MAX_LENGTH + 1);

		expect(validateComboDraft(draft({ name: atLimit }))).toEqual([]);
		expect(validateComboDraft(draft({ name: overLimit }))).toEqual([
			{ code: "name_too_long" },
		]);
	});

	it("requires between one and ten steps", () => {
		expect(validateComboDraft(draft({ steps: [] }))).toEqual([
			{ code: "steps_required" },
		]);

		const maxSteps = Array.from({ length: COMBO_MAX_STEPS }, (_, index) =>
			step({ priority: index + 1 }),
		);
		expect(validateComboDraft(draft({ steps: maxSteps }))).toEqual([]);

		const tooManySteps = Array.from(
			{ length: COMBO_MAX_STEPS + 1 },
			(_, index) => step({ priority: index + 1 }),
		);
		expect(validateComboDraft(draft({ steps: tooManySteps }))).toEqual([
			{ code: "steps_too_many" },
		]);
	});

	it("flags a step without provider or model", () => {
		expect(
			validateComboDraft(
				draft({ steps: [step(), step({ providerId: "", priority: 2 })] }),
			),
		).toEqual([{ code: "step_provider_required", stepIndex: 1 }]);

		expect(
			validateComboDraft(
				draft({ steps: [step(), step({ model: "", priority: 2 })] }),
			),
		).toEqual([{ code: "step_model_required", stepIndex: 1 }]);
	});

	it("rejects priorities outside the 1-255 integer range", () => {
		for (const priority of [0, -1, 256, 1.5, Number.NaN]) {
			expect(
				validateComboDraft(draft({ steps: [step({ priority })] })),
			).toEqual([{ code: "step_priority_invalid", stepIndex: 0 }]);
		}

		expect(
			validateComboDraft(draft({ steps: [step({ priority: 255 })] })),
		).toEqual([]);
	});

	it("rejects duplicated priorities and reports the offending step", () => {
		expect(
			validateComboDraft(
				draft({
					steps: [
						step({ priority: 1 }),
						step({ priority: 2 }),
						step({ priority: 1 }),
					],
				}),
			),
		).toEqual([{ code: "step_priority_duplicate", stepIndex: 2, priority: 1 }]);
	});

	it("reports every independent problem in one pass", () => {
		const issues = validateComboDraft({
			name: "",
			steps: [step({ providerId: "", priority: 0 })],
		});

		expect(issues.map((issue) => issue.code)).toEqual([
			"name_required",
			"step_provider_required",
			"step_priority_invalid",
		]);
	});
});

describe("emptyComboDraft", () => {
	it("starts with one empty step so the form is usable", () => {
		const value = emptyComboDraft();

		expect(value.name).toBe("");
		expect(value.steps).toHaveLength(1);
		expect(value.steps[0]).toEqual({
			providerId: "",
			model: "",
			connectionId: null,
			priority: 1,
		});
	});
});

describe("toCreateComboRequest", () => {
	it("trims the name, pins the strategy and normalises empty connection ids", () => {
		const payload = toCreateComboRequest({
			name: "  Production fallback  ",
			steps: [step({ connectionId: "", priority: 3 })],
		});

		expect(payload).toEqual({
			name: "Production fallback",
			strategy: "priority",
			steps: [
				{
					providerId: "provider-openai",
					model: "gpt-4o",
					connectionId: null,
					priority: 3,
				},
			],
		});
	});
});
