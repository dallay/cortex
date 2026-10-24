export const COMBO_ALIAS_PREFIX = "rook/";
export const COMBO_ALIAS_MAX_NAME_LENGTH = 64;

export type ComboAliasValidationCode =
	| "alias_required"
	| "alias_prefix_required"
	| "alias_name_required"
	| "alias_too_long"
	| "alias_format_invalid";

export interface ComboAliasValidationIssue {
	code: ComboAliasValidationCode;
}

/**
 * Client-side equivalent of `ComboAlias::validate_alias_format` in
 * `crates/domain/rook-core/src/model.rs`.
 */
export function validateComboAlias(value: string): ComboAliasValidationIssue[] {
	const alias = value.trim();
	if (alias.length === 0) return [{ code: "alias_required" }];
	if (!alias.startsWith(COMBO_ALIAS_PREFIX)) {
		return [{ code: "alias_prefix_required" }];
	}

	const name = alias.slice(COMBO_ALIAS_PREFIX.length);
	if (name.length === 0) return [{ code: "alias_name_required" }];
	if (name.length > COMBO_ALIAS_MAX_NAME_LENGTH) {
		return [{ code: "alias_too_long" }];
	}
	if (!/^[a-z0-9][a-z0-9_-]{0,63}$/.test(name)) {
		return [{ code: "alias_format_invalid" }];
	}
	return [];
}

export function normalizeComboAlias(value: string): string {
	return value.trim();
}
