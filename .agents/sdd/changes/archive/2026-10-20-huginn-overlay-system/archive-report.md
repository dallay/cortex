# Archive: huginn-overlay-system

## Result and destination

- Date: 2026-10-20. Persistence: OpenSpec in `.agents/sdd/` by explicit path.
- Destination: `.agents/sdd/changes/archive/2026-10-20-huginn-overlay-system/`.
- Closure authorized with warnings; it does not certify deployment, human acceptance,
  remote availability nor external operational integration.
- No collision on the destination nor on the three new domains. The existing
  `ui-settings` spec was read and merged; it was not overwritten as a new domain.
- State prepared before the move: `current_phase: archived`, `archive`
  added to `completed`, `next: none` and `archived_to` with the previous destination.
- Fallback execution: no enforcement through a deterministic runner/FSM is claimed.
  Skill resolution: `none`; there is no local skills registry.

## Gate checked before syncing

proposal, design, tasks, four specs, the `ui-settings` base, state, configuration,
verify-report and qa-report were read in full. There are no local Archive rules
nor a more restrictive P2/P3 policy. Apply has 21/21 tasks and 3/3 substeps of 2.2.

- Verify: **PASS WITH WARNINGS**. V-01/V-02 resolved; the original independent
  reproduction passes after the SET-05 fix. The initial FAIL is preserved as history.
- Own QA: **PASS WITH WARNINGS**, gate **AUTHORIZED WITH WARNINGS**.
  There are no pending CRITICAL/P0/P1 nor open functional failures identified.
- QA ran 263 Cargo cases, including 39 PTY, and 10 additional exploratory
  checks; they are not presented as an Archive run nor as exhaustive coverage.
- Apply final CI re-run: 9/9, exit 0, 477 s, Vitest 210 and E2E 90.
  `huginn-fix-v01/ci-summary.txt` was checked and its provenance is preserved.
  Archive does not repeat broad CI for these exclusively documentary operations.

The operational NOT TESTED items are not pending contractual scenarios: OVL-05-A
requires a controlled service and OVL-05-B a controlled storage failure,
both executed by QA. A documentation/configuration exception is not used to
accept the product. The historical sentences in tasks/Verify that handed over to
the next phase are preserved; the later QA report authorizes this gate.

## Synced specs

- `.agents/sdd/specs/huginn-overlays/spec.md`: created, 5 requirements added.
- `.agents/sdd/specs/huginn-ui-events/spec.md`: created, 4 requirements added.
- `.agents/sdd/specs/huginn-status-bar/spec.md`: created, 3 requirements added.
- `.agents/sdd/specs/ui-settings/spec.md`: REQ-07, REQ-08 and REQ-12 replaced
  exactly by their delta blocks; 0 added and 0 removed.

Byte equality of the three new copies was verified. The 16 blocks of
`ui-settings` were compared: the 13 not mentioned and all the content outside
requirements remain identical to the read base, contrasted with HEAD. Historical
headers, checklists and references to previous targets are not normalized.
The inherited reference to the nonexistent `--test settings` remains documented in Verify;
the current evidence uses the real lib/state/PTY targets, not that command.

## Preserved content and evidence

The complete folder keeps proposal, exploration, design, tasks, specs,
apply-progress, apply-validation, verify-report, qa-report and state, plus this
report. The acceptance reports and the deltas are not edited during Archive.
No file is modified after being moved to the historical archive.

The external logs and captures keep their original paths, they are neither deleted
nor moved. Evidence prefix:
`/private/var/folders/zz/d4kl1hfj1j15nxm43d24px300000gn/T/opencode/`.

- `huginn-fix-v01/`: final CI, regressions and Apply focused repetition.
- `huginn-verify/`: initial Verify, independent test and historical RED evidence.
- `huginn-reverify-v01/`: recompiled original test, presentation and static checks.
- `huginn-qa/`: own logs, reproduction, snapshots, PNG and asciicast; also
  failed/interrupted envelopes and `recording.json` with a WebM error.
- The other historical Apply paths remain referenced without alteration in their
  reports. The history is not presented as evidence of the corrected candidate.
- `huginn-archive/before-move.json`: SHA-256 hashes of the 13 files previous
  to this report, to check integrity after the move.

The external evidence is still in temporary storage: keep those paths or
back them up before cleaning temporaries. This file does not embed their binaries.

## Documentary validation and integrity

- Structural check: PASS, 21/21 tasks, three exact replacements,
  thirteen unmodified requirements and three exact new copies.
- Markdownlint of the four deltas and both Verify/QA reports: PASS, 6 files.
- Initial Markdownlint of the four main specs: exit 1 due to inherited MD038
  in `ui-settings`. The representation of the literal was fixed with quotes and one
  clarification: the entry keeps exactly the three spaces and includes no quotes.
  It does not change the behavioral requirement.
- Final Markdownlint of the four main specs and all the Markdown of the
  archived change: PASS, 17 files, zero incidents. `git diff --check`: PASS.
- Expected staging SHA-256:
  `180ff411c3b36a8699b8986dd543e8237cf83fdebe946e6f052722fea4471c54`.
  It is checked before and after. No add/stage/unstage/reset/checkout/commit/push,
  merge/PR, data reset nor production changes.

## Mandatory warnings

- External provider, real OS secure store and two inherited opt-ins: NOT TESTED;
  QA-03 P2 operational. They do not become an external PASS nor validated production.
- QA-01 P3: WebM export of the instrument FAILED; valid PNG/asciicast are not video.
- QA-02 P3: agent-tty startup requires the documented explicit DSR response.
- Cause of the intermediate failure of the PTY helper not established; the complete
  case passed afterwards in the final gates, in reverify and in the 39 own QA PTY.
- No instrumental coverage, screen reader nor complete locales validated.
- The external evidence remains in temporary storage and requires a backup
  before cleaning those directories. The inherited MD038 was resolved.

## Next step

No pending SDD phase. The cycle closes with warnings. Any
extension to external integration requires test credentials and explicit
authorization; Archive does not authorize publication, merge nor deployment.
