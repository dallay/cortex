import type { CreateModelAliasRequest } from "@/lib/api";

export type ModelAliasValidationCode =
	| "alias_required"
	| "canonical_required"
	| "reserved_combo_prefix";

export interface ModelAliasValidationIssue {
	field: "alias" | "canonical";
	code: ModelAliasValidationCode;
}

/** Mirrors empty-field and reserved `rook/*` checks in the backend handler. */
export function validateModelAlias(
	data: Pick<CreateModelAliasRequest, "alias" | "canonical">,
): ModelAliasValidationIssue[] {
	const issues: ModelAliasValidationIssue[] = [];
	const alias = data.alias.trim();

	if (!alias) {
		issues.push({ field: "alias", code: "alias_required" });
	} else if (alias.toLowerCase().startsWith("rook/")) {
		issues.push({ field: "alias", code: "reserved_combo_prefix" });
	}

	if (!data.canonical.trim()) {
		issues.push({ field: "canonical", code: "canonical_required" });
	}

	return issues;
}
