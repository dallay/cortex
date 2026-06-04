# Archive Report: API Key Scopes and Restrictions

**Change**: `api-key-scopes-and-restrictions`
**Archived**: `openspec/changes/archive/2026-06-02-api-key-scopes-and-restrictions/`
**Archived date**: 2026-06-02
**Mode**: openspec

---

## Executive Summary

The change **PASSES VERIFICATION** and has been archived. All 364 workspace tests pass, format and clippy are clean, and the behavioral compliance matrix shows 30/31 spec scenarios covered. The delta specs were synced into the main specs prior to archiving.

---

## Spec Sync Summary

All five delta specs were merged into their corresponding main specs. The main specs have newer timestamps than the delta specs, confirming successful sync.

| Domain             | Delta File                    | Main Spec                          | Status                          |
|--------------------|-------------------------------|------------------------------------|---------------------------------|
| api-key-domain     | `specs/api-key-domain.md`     | `specs/api-key-domain/spec.md`     | ✅ Synced (Jun 2 21:35 vs 21:15) |
| api-key-repository | `specs/api-key-repository.md` | `specs/api-key-repository/spec.md` | ✅ Synced (Jun 2 21:36 vs 21:15) |
| api-key-usecases   | `specs/api-key-usecases.md`   | `specs/api-key-usecases/spec.md`   | ✅ Synced (Jun 2 21:36 vs 21:15) |
| api-key-transport  | `specs/api-key-transport.md`  | `specs/api-key-transport/spec.md`  | ✅ Synced (Jun 2 21:37 vs 21:15) |
| api-key-dashboard  | `specs/api-key-dashboard.md`  | `specs/api-key-dashboard/spec.md`  | ✅ Synced (Jun 2 21:37 vs 21:15) |

**Sync actions taken:**

- REQ-DOM-2: Scope allowlist replaced binary `read`/`write` with 5 canonical scopes (`chat:read`, `chat:write`, `providers:read`, `providers:write`, `admin`)
- REQ-DOM-9: Model allowlist added (`allowed_models: Vec<ModelId>`, empty = unrestricted)
- REQ-DOM-10: Provider allowlist added (`allowed_providers: Vec<ProviderId>`, empty = unrestricted)
- REQ-DOM-11: Restriction semantics in `ApiKeySubject` (runtime auth principal carries same restriction fields as persisted record)
- Corresponding repository, use case, transport, and dashboard specs updated with new restriction enforcement behavior

---

## Archive Contents

| Artifact                 | Status                                                                          |
|--------------------------|---------------------------------------------------------------------------------|
| `proposal.md`            | ✅ Present                                                                       |
| `specs/` (5 delta specs) | ✅ Present                                                                       |
| `design.md`              | ✅ Present                                                                       |
| `tasks.md`               | ✅ Present (24 tasks, 0 marked `[x]` — implementation complete per code commits) |
| `verify-report.md`       | ✅ Present (PASS WITH WARNINGS)                                                  |
| `state.yaml`             | ✅ Present                                                                       |

---

## Verification Summary

**Overall**: PASS WITH WARNINGS

### Build & Test Results

- `cargo build --workspace`: ✅ PASS
- `cargo fmt --all`: ✅ PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: ✅ PASS (zero warnings)
- `cargo test --workspace`: ✅ 364/364 PASS
- Dashboard Vitest: ✅ 25/25 PASS

### Behavioral Compliance

- 30/31 spec scenarios covered by passing runtime tests
- 3 feature commits on `main`: `2516cea`, `b16a839`, `fcc40a5`

### Warnings

1. `tasks.md` checkboxes not updated (implementation complete via commits, file not synced)
2. `tests/scope_routing.rs` not created as separate file — 5 scope routing cases exist inline in `authz.rs`
3. Dedicated `IntoResponse for RestrictionViolation` not created — existing `CortexError::forbidden()` path reused (functionally equivalent)

---

## SDD Cycle Complete

The change has been fully planned (proposal), specified (specs), designed (design), tasked (tasks), implemented (3 commits), verified (PASS WITH WARNINGS), and archived. The main specs in `openspec/specs/` now reflect the new behavior.

Ready for the next change.
