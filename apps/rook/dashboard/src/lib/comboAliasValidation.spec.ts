import { describe, expect, it } from "vitest";
import {
	COMBO_ALIAS_MAX_NAME_LENGTH,
	type ComboAliasValidationCode,
	normalizeComboAlias,
	validateComboAlias,
} from "./comboAliasValidation";

describe("validateComboAlias", () => {
	it("accepts lowercase rook aliases with allowed separators", () => {
		expect(validateComboAlias("rook/coding")).toEqual([]);
		expect(validateComboAlias("rook/1st-model_v2")).toEqual([]);
	});

	it.each([
		["", "alias_required"],
		["   ", "alias_required"],
		["coding", "alias_prefix_required"],
		["ROOK/coding", "alias_prefix_required"],
		["rook/", "alias_name_required"],
		["rook/-bad", "alias_format_invalid"],
		["rook/Upper", "alias_format_invalid"],
		["rook/two words", "alias_format_invalid"],
		["rook/a.b", "alias_format_invalid"],
		[`rook/${"a".repeat(COMBO_ALIAS_MAX_NAME_LENGTH + 1)}`, "alias_too_long"],
	] satisfies Array<[string, ComboAliasValidationCode]>)(
		"rejects %j with %s",
		(alias, expectedCode) => {
			expect(validateComboAlias(alias).map((issue) => issue.code)).toContain(
				expectedCode,
			);
		},
	);

	it("accepts the 64-character backend limit", () => {
		expect(
			validateComboAlias(`rook/${"a".repeat(COMBO_ALIAS_MAX_NAME_LENGTH)}`),
		).toEqual([]);
	});
});

describe("normalizeComboAlias", () => {
	it("trims surrounding whitespace before sending the request", () => {
		expect(normalizeComboAlias("  rook/coding  ")).toBe("rook/coding");
	});
});
