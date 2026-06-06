# Verification Report — `providers-ui-3-screen-refactor`

> **Verdict: FAIL** — 1 CRITICAL, 3 WARNING, 2 SUGGESTION, 3 INFO.
> The CRITICAL finding (broken i18n parity) blocks `sdd-archive` per the quality gate.

## Metadata

| Field | Value |
|---|---|
| Change | `providers-ui-3-screen-refactor` |
| Mode | openspec (file-based persistence) |
| Commits verified on `main` | `b1eb991` (24 files, 1590+/490-), `54005ee` (follow-up: link to issue #132) |
| Spec requirements | 6 (matches prompt) |
| Spec scenarios | 20 (prompt stated 22 — off by 2) |
| Design decisions | D1–D8 (8) |
| Tasks | 49 total, 21 marked `[x]`, 28 marked `[ ]` in actual file (prompt stated "all [x]" — discrepancy) |
| Date | 2026-06-06 |

---

## 1. Build / Tests / Coverage Evidence

All validation commands listed in `design.md` §11 ran cleanly except the i18n-parity gate.

### 1.1 Backend (untouched by change — smoke check)

```text
$ cargo check --workspace --all-targets
    Finished `dev` profile [unoptimized + debuginfo] in 4.74s

$ cargo fmt --all -- --check
    (no output — clean)

$ cargo clippy --workspace --all-targets -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] in 5.43s

$ cargo test --workspace
    Doc-tests rook
    Running unittests (...)
    test result: ok. ... (all packages)
```

**Result: PASS.**

### 1.2 Frontend unit + type tests

```text
$ cd apps/rook/dashboard && pnpm exec vitest run
 Test Files  10 passed (10)
      Tests  83 passed (83)
   Duration  2.19s
```

`AddProviderDialog.spec.ts` was fully rewritten with 13 tests covering:

| Group | Count | Spec REQ-3 scenario covered |
|---|---|---|
| create mode | 3 | Open in create mode (scoped + unscoped + no delete) |
| edit mode | 4 | Edit existing (prefill + cache-miss fetch + delete confirm + remove) |
| test before save | 4 | Save disabled until `testResult.ok === true`; failed test keeps it disabled; test disabled until name+apikey |
| auth type gating | 1 | OAuth disabled with "coming soon" notice when `supportsOAuth === false` (D8) |
| open/close | 1 | v-model:open controlled by parent |

**Result: PASS.** All 5 REQ-3 modal scenarios have a passing test.

### 1.3 Build + typecheck

```text
$ pnpm run build
    ✓ built in 7.95s
    (rolldown INVALID_ANNOTATION warnings from node_modules/.pnpm/@vueuse+core@14.3.0/...
     — pre-existing, NOT from our code)
```

`pnpm run build` succeeds because Rolldown/Vite does NOT run TypeScript typechecking.

```text
$ pnpm exec vue-tsc --noEmit
  src/components/ui/chart/ChartContainer.vue(38,13): error TS2339:
      Property 'cn' does not exist on type '...'
  src/i18n/index.ts(21,19): error TS2719:
      Type '{ ... }' is missing the following properties from type
      '{ ... }': "addProvider", "addProviderDescription", "active",
      "apiKey", and 9 more.
  src/i18n/index.ts(25,5): error TS18046:
      'i18n.global' is of type 'unknown'.
```

**Three errors total.** Two are pre-existing (ChartContainer, i18n global type).
**One is caused by this change** — see CRITICAL #1 below.

```text
$ wc -l apps/rook/dashboard/src/locales/en.json apps/rook/dashboard/src/locales/es.json
    270 en.json
    257 es.json
```

**i18n parity gate FAILS** (design §11 explicit verification gate).

---

## 2. Spec Compliance Matrix

6 requirements × 20 scenarios. `PASS` = scenario has a covering test or runtime evidence.
`N/A` = not testable in unit suite (e.g. navigation flow). `PARTIAL` = evidence exists but no
test.

| # | Requirement / Scenario | Evidence | Status |
|---|---|---|---|
| **REQ-1** | **Providers Catalog** | `views/ProvidersView.vue`, `composables/useProviderCatalog.ts`, `components/providers/*` | PASS |
| 1.1 | Empty catalog | `useProviderCatalog.items` always returns 5 entries from `PROVIDER_KINDS`; `connectionCount` defaults to 0 | PASS (visual) |
| 1.2 | Catalog with connections | `useProviderCatalog` joins live `useProviders().providers` | PASS (visual) |
| 1.3 | Filter by category | `ProviderCatalogFilter` emits `update:activeCategory`; `ProvidersView` v-models it | PASS (visual) |
| 1.4 | Search the catalog | `useProviderCatalog` filters by `name.toLowerCase().includes(q.toLowerCase())` | PASS (visual) |
| 1.5 | Navigate to details | `ProviderCatalogCard` is a `<RouterLink to="/providers/${kind}">` | PASS (visual) |
| **REQ-2** | **Provider Details** | `views/ProviderDetailsView.vue` | PASS |
| 2.1 | Details with connections | Filters `useProviders().providers` by `route.params.providerKind` | PASS (visual) |
| 2.2 | Empty details state | Shows EmptyState with "Add your first …" CTA | PASS (visual) |
| 2.3 | Test all connections | Sequential `testCredentials()` calls per connection | PASS (visual) |
| 2.4 | Add from details | Opens `AddProviderDialog` with `providerKind` prop pre-scoped | PASS (covered by AddProviderDialog test 514) |
| 2.5 | Navigate back to catalog | Breadcrumb `Providers` + `← Back to providers` link | PASS (visual) |
| **REQ-3** | **Connection Modal** | `components/AddProviderDialog.vue` + 13 spec tests | PASS |
| 3.1 | Open in create mode | `AddProviderDialog.spec.ts:514, 521` (scoped + unscoped) | PASS (test) |
| 3.2 | Test credentials | `AddProviderDialog.spec.ts:612, 633` (success enables, failure keeps disabled) | PASS (test) |
| 3.3 | Save a new connection | `AddProviderDialog.spec.ts:612` exercises the success path | PASS (test) |
| 3.4 | Edit existing connection | `AddProviderDialog.spec.ts:535, 549` (prefill + cache-miss fetch) | PASS (test) |
| 3.5 | Cancel without saving | `AddProviderDialog.spec.ts:673` (v-model:open close + form reset) | PASS (test) |
| **REQ-4** | **Providers Quota Placeholder** | `views/ProvidersQuotaView.vue` | PASS |
| 4.1 | Navigate to quota | `/providers/quota` route, banner with `followUpIssue` link to #132 | PASS (visual) |
| **REQ-5** | **EmptyState Component** | `components/EmptyState.vue` | PASS |
| 5.1 | Backward-compatible callsites | Wraps shadcn-vue `Empty`; public API `{title, description, icon}` unchanged | PASS (code) |
| 5.2 | New shadcn-vue features | Default slot passthrough; sub-component composition supported | PASS (code) |
| **REQ-6** | **Catalog Metadata Source** | `config/providerCatalog.ts` + `useProviderCatalog` | PASS |
| 6.1 | Display kind metadata | All metadata fields read from `PROVIDER_KINDS`; counts derived from `useProviders` | PASS (code) |
| 6.2 | Adding a new kind | Adding to `PROVIDER_KINDS` flows through automatically; route validity check in `router/index.ts` catches drift | PASS (code + risk) |

**Coverage: 20/20 scenarios have evidence. 5/20 have a passing runtime test (the dialog ones).**

---

## 3. Design Coherence (D1–D8)

| # | Decision | Implementation | Status |
|---|---|---|---|
| D1 | Keep `useProviders`, add thin `useProviderCatalog` derived composable | `useProviderCatalog` is a 110-line composable; calls `useProviders()` + `useAvailableModels()` and exposes `items` ComputedRef | **PASS** |
| D2 | No Pinia store | No Pinia references; providers data lives in `useProviders` composable | **PASS** |
| D3 | Static `PROVIDER_KINDS` in TS | `apps/rook/dashboard/src/config/providerCatalog.ts` exports `PROVIDER_KINDS` with 5 entries | **PASS** |
| D4 | Wrap shadcn-vue `Empty` inside existing `EmptyState` (public API unchanged) | `EmptyState.vue` (33 lines) wraps `Empty`/`EmptyHeader`/`EmptyTitle`/`EmptyDescription`/`EmptyContent` | **PASS** |
| D5 | Catalog is route `/providers` (not `/providers/catalog`) | Router has `/providers` → `ProvidersView` | **PASS** |
| D6 | Modal controlled by parent via `v-model:open` | `AddProviderDialog` accepts `open` prop, emits `update:open` | **PASS** |
| D7 | `providerKind` route param validated in `beforeEnter` | `router/index.ts:55-65` validates against `VALID_PROVIDER_KINDS`; invalid kinds redirect to `/providers` | **PASS** |
| D8 | OAuth form is rendered disabled with notice | `AddProviderDialog` ToggleGroup disables OAuth when `supportsOAuth === false`; "coming soon" notice shown; `AddProviderDialog.spec.ts:662` proves behavior | **PASS** |

**Coherence: 8/8 decisions implemented as designed.**

---

## 4. Correctness Table

| Finding | Location | Severity | Status |
|---|---|---|---|
| Missing i18n keys in `es.json` | `apps/rook/dashboard/src/locales/es.json` (13 keys under `providers.*`) | **CRITICAL** | **Confirmed** |
| Stale i18n keys not removed (`providersQuotes`, `providersList`) | `en.json:9-10, 29` + `es.json:9-10, 29` | WARNING | Confirmed |
| `lib/api.ts` type unions not tightened (8 fields stay `string`) | `apps/rook/dashboard/src/lib/api.ts:31, 62-64, 130-132, 520-534` | WARNING | Confirmed |
| Components in `src/components/providers/` subdirectory (design said flat) | `apps/rook/dashboard/src/components/providers/*` | WARNING | Confirmed |
| `tasks.md` checkbox discrepancy (21 [x] / 28 [ ]) | `openspec/changes/.../tasks.md` | SUGGESTION | Confirmed |
| 5 lucide icons handled via local map (not `useNavigation` registry) | `ProviderCatalogCard.vue`, `ProvidersQuotaView.vue` | SUGGESTION | Confirmed |
| 2 pre-existing vue-tsc errors NOT from our change | `src/components/ui/chart/ChartContainer.vue:38`, `src/i18n/index.ts:25` | INFO | Confirmed |
| Rolldown warnings from `@vueuse/core@14.3.0` | `node_modules/.pnpm/@vueuse+core@14.3.0/...` | INFO | Confirmed |
| Spec scenario count prompt said 22, actual is 20 | `specs/providers-ui/spec.md` | INFO | Confirmed |

---

## 5. Issues

### CRITICAL

#### C1. i18n parity broken in `es.json`'s `providers` subtree (13 keys missing)

**Evidence:**

```text
$ jq '.providers | keys' apps/rook/dashboard/src/locales/en.json | wc -l
28 keys in en.json's `providers`
$ jq '.providers | keys' apps/rook/dashboard/src/locales/es.json | wc -l
15 keys in es.json's `providers`

$ pnpm exec vue-tsc --noEmit
  src/i18n/index.ts(21,19): error TS2719:
      Type '{ ... }' is missing the following properties from type '{ ... }':
      "addProvider", "addProviderDescription", "active", "apiKey",
      and 9 more.
```

The 13 missing keys (under `es.json.providers`):

```text
addProvider, addProviderDescription, advancedConfig, active, apiKey,
apiKeyHint, baseUrl, defaultModel, maxConcurrent, priorityHint,
testConnection, testFailed, testSuccess
```

**Why CRITICAL:**

1. The proposal itself said: "i18n schema — `MessageSchema = typeof en` already enforces
   parity; adding a key to `en.json` without `es.json` is a TS error." — this is the
   type-system guard the proposal relied on, and it has been broken.
2. `design.md` §11 lists, as a verification gate:
   > "i18n parity: `en.json` line count == `es.json` line count for the `providers` subtree"
   en.json = 270 lines, es.json = 257 lines. **Gate fails.**
3. Runtime impact: any Spanish-locale user who opens the new connection form will see
   empty strings (or `undefined`/`null` depending on Vue I18n fallback config) for the
   new fields: api key hint, base URL, priority hint, advanced config section, test
   connection button label, test success/failure messages. The form is functionally
   broken in Spanish.
4. The verification plan in `design.md` §11 explicitly says to run
   `pnpm exec vue-tsc --noEmit` — this would have caught the issue before merge. It was
   not run, or its output was ignored.

**Required action:** Mirror the 13 missing keys from `en.json` to `es.json` with
appropriate Spanish translations. Re-run `pnpm exec vue-tsc --noEmit` and confirm 0
errors. Then update the i18n parity line count.

### WARNING

#### W1. Stale i18n keys not removed (`providersQuotes`, `providersList`)

**Evidence:**

```text
$ rg -n "providersQuotes|providersList" apps/rook/dashboard/src/
  src/locales/en.json:9:    "providersList": "List",
  src/locales/en.json:10:   "providersQuotes": "Quotes",
  src/locales/en.json:29:   "providersQuotes": "Provider Quotes",
  src/locales/es.json:9:    "providersList": "Listar",
  src/locales/es.json:10:   "providersQuotes": "Cotizaciones",
  src/locales/es.json:29:   "providersQuotes": "Cotizaciones de Proveedores",
  (no other references in code)
```

`design.md` §7 says verbatim: "Remove `nav.providersQuotes` + `breadcrumb.providersQuotes`
from both files." Not done.

**Impact:** Dead weight in locale bundles; potential confusion for future contributors
who grep for these keys. No functional regression because the keys are unused.

**Required action:** Remove `nav.providersQuotes`, `nav.providersList`,
`breadcrumb.providersQuotes` from both `en.json` and `es.json`. Bundle together with the
C1 fix.

#### W2. `lib/api.ts` type unions not tightened

**Evidence:**

```text
$ rg -n "providerKind|authType" apps/rook/dashboard/src/lib/api.ts
  31:  providerKind: string        # CreateProviderRequest
  62:  providerKind: string        # ProviderConnectionResponse
  64:  authType: string
  130: providerKind: string        # UpdateProviderRequest
  132: authType: string
  520: providerKind: string        # TestCredentialsPayload
  522: authType: string
  532: providerKind?: string       # query params
  534: authType?: string
```

`design.md` §2 file structure table says `lib/api.ts` → MODIFY → "Tighten
`providerKind`/`authType` to unions". The implementation left them as `string`.

**Why this matters:** The proposal's promise that "Adding a new kind requires updating
BOTH the catalog AND this list. Drift would surface as a 404." (in `router/index.ts`) is
fine for the router — but the *request types* in `api.ts` also have no compile-time
guarantee that the runtime value is a valid `ProviderKind`. The dialog and composable
work around this with `as ProviderKind` casts (see `AddProviderDialog.vue:221, 222` and
`useProviderCatalog.ts:70, 89`).

**Impact:** Type-safety is weakened. Adding a new kind without updating `api.ts` would
compile fine. The casts are correct at runtime because `useProviderCatalog.items` is
the only producer of `ProviderKind` values.

**Required action:** Either (a) tighten `api.ts` to use `ProviderKind` and `AuthType`
unions and update call sites, or (b) document this as an intentional deviation in a new
ADR and remove it from `design.md` §2 expectations. The first is the right fix.

#### W3. Components in `src/components/providers/` subdirectory (design said flat)

**Evidence:** `design.md` §2 file structure table lists `src/components/ProviderCatalogCard.vue`,
`src/components/ProviderCategorySection.vue`, `src/components/ProviderCatalogFilter.vue`,
`src/components/ConnectionListItem.vue`. Implementation has them at
`src/components/providers/ProviderCatalogCard.vue` etc. The proposal also said flat.

**Impact:** Minor. Component import paths are slightly longer; auto-import scanning
covers the new directory; tests still pass.

**Required action:** Either move them up one level (preferred — matches design + proposal
+ AGENTS.md "small focused modules"), or update `design.md` §2 to reflect the new
location. The first is consistent with the rest of the codebase.

### SUGGESTION

#### S1. `tasks.md` checkbox discrepancy

The orchestrator prompt said "49 tasks across 12 phases, all marked `[x]`". The actual
file has **21 `[x]` / 28 `[ ]`**. The unchecked tasks are concentrated in:
- Phase 2 (Spec): seems complete (spec exists)
- Phase 3 (Design): seems complete (design exists)
- Phase 9 (Tracking), 10 (Documentation), 11 (Final Review), 12 (Archive): these are
  post-implementation activities

**Impact:** None on runtime. The phase order makes sense and the `[ ]` markers are
plausibly "did not track" rather than "incomplete". But the discrepancy with the prompt
should be noted in the report for the orchestrator.

**Action:** No code change. Verify with the orchestrator that this is acceptable before
archive.

#### S2. 5 lucide icons handled via local map (not `useNavigation` registry)

**Evidence:** `design.md` §2 file structure table lists
`src/composables/useNavigation.ts` → MODIFY → "5 new lucide icons". The implementation
does NOT modify `useNavigation.ts`. Instead:
- `ProviderCatalogCard.vue` has a local `iconMap: Record<ProviderKind, LucideIcon>` with
  `Cpu`, `Sparkles`, `Brain`, `Zap`, `Server`
- `ProvidersQuotaView.vue` imports `Fuel` directly from `@lucide/vue`

**Why defensible:** The task note in `tasks.md` (Phase 7) explicitly states:
> "no registry change needed — `useNavigation`'s `iconRegistry` is exclusive to the
> sidebar nav. Catalog card icons are resolved in `ProviderCatalogCard.vue`'s local map
> and `ProvidersQuotaView.vue` imports `Fuel` directly."

This is intentional and architecturally sound: keeping the icon registry minimal
benefits tree-shaking. But it does contradict `design.md` §2's literal text.

**Action:** Update `design.md` §2 to reflect the actual approach (delete the "5 new
lucide icons" line for `useNavigation.ts`; add a note that catalog icons live in
`ProviderCatalogCard`'s local map).

### INFO

#### I1. 2 pre-existing vue-tsc errors NOT from our change

```text
src/components/ui/chart/ChartContainer.vue(38,13): error TS2339: Property 'cn' does not exist
src/i18n/index.ts(25,5): error TS18046: 'i18n.global' is of type 'unknown'
```

`ChartContainer.vue` was last touched before this change. The `i18n.global` type issue
predates this work (independent of the schema drift in C1). Triage in a separate change.

#### I2. Rolldown `INVALID_ANNOTATION` warnings from `@vueuse/core@14.3.0`

Pre-existing, third-party. Not from our code. Triage upstream.

#### I3. Spec scenario count: prompt stated 22, actual is 20

Counted from `specs/providers-ui/spec.md`:

| Requirement | Scenarios |
|---|---|
| REQ-1 Catalog | 5 |
| REQ-2 Details | 5 |
| REQ-3 Modal | 5 |
| REQ-4 Quota | 1 |
| REQ-5 EmptyState | 2 |
| REQ-6 Catalog Metadata Source | 2 |
| **Total** | **20** |

The prompt's "22" is off by 2. No impact on the report — just an accounting note.

---

## 6. Risks Discovered

1. **CI gap:** `pnpm run build` does NOT run `vue-tsc`. The build pipeline is silent on
   the type errors. The design's verification plan listed `vue-tsc` as a gate, but
   nothing in CI enforces it. This is how C1 escaped.
   - **Mitigation:** Add a `pnpm exec vue-tsc --noEmit` step to CI before
     `pnpm run build`. (Filed as a suggestion, not blocking this change.)
2. **Spanish locale is broken at runtime** (C1) — needs the i18n fix before merge.
3. **Type-safety is weakened** (W2) — future refactors that touch `api.ts` won't be
   caught by the compiler if they pass a `string` where a `ProviderKind` is expected.

---

## 7. Required Actions Before `sdd-archive`

1. **Mirror 13 keys from `en.json.providers` to `es.json.providers`** with Spanish
   translations. Re-run `pnpm exec vue-tsc --noEmit` and confirm 0 errors.
2. **Remove stale keys** from both `en.json` and `es.json`: `nav.providersQuotes`,
   `nav.providersList`, `breadcrumb.providersQuotes`.
3. **Tighten `lib/api.ts`** types from `string` to `ProviderKind` / `AuthType` unions,
   OR document the deviation in a new ADR and update `design.md` §2 accordingly.
4. **Move `components/providers/*.vue` up one level** to `components/*.vue` (matches
   design + proposal + AGENTS.md), OR update `design.md` §2 to reflect the new
   location.
5. **Update `design.md` §2** to remove the literal "5 new lucide icons" line for
   `useNavigation.ts` and add a note about the local map in `ProviderCatalogCard.vue`.

After 1–5 are addressed and `pnpm exec vue-tsc --noEmit` is clean, re-run
`pnpm exec vitest run` (should still be 83/83) and update this report. Then proceed
to `sdd-archive`.

---

## 8. Final Verdict

**FAIL** — 1 CRITICAL (i18n parity broken for Spanish locale), 3 WARNING, 2 SUGGESTION,
3 INFO.

The implementation is functionally correct: 83/83 vitest tests pass, all 8 design
decisions are implemented as designed, and all 20 spec scenarios have evidence. The
single blocker is the i18n parity violation in `es.json`, which:
- breaks a verification gate explicitly listed in `design.md` §11,
- violates a safety promise in the proposal,
- produces a real runtime defect for Spanish-locale users,
- and would have been caught by `vue-tsc` if the verification plan had been followed
  before merge.

Fix the 5 actions above, re-run `vue-tsc` + `vitest`, and re-verify. Then archive.
