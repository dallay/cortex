import { describe, expect, it } from "vitest";
import { validateModelAlias } from "./modelAliasValidation";

describe("validateModelAlias", () => {
	it("requires alias and canonical model", () => {
		expect(validateModelAlias({ alias: "", canonical: "gpt-4o" })).toEqual([
			{ field: "alias", code: "alias_required" },
		]);
		expect(validateModelAlias({ alias: "chat", canonical: " " })).toEqual([
			{ field: "canonical", code: "canonical_required" },
		]);
	});

	it("rejects the reserved rook prefix case-insensitively after trimming", () => {
		for (const alias of ["rook/coding", " ROOK/coding ", "Rook/chat"]) {
			expect(validateModelAlias({ alias, canonical: "gpt-4o" })).toContainEqual(
				{
					field: "alias",
					code: "reserved_combo_prefix",
				},
			);
		}
	});

	it("accepts a regular model alias", () => {
		expect(
			validateModelAlias({ alias: "chat-fast", canonical: "gpt-4o-mini" }),
		).toEqual([]);
	});
});
