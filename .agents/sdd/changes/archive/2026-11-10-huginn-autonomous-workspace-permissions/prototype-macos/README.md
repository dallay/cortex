# Disposable macOS App Sandbox feasibility prototype

This is an isolated experiment for SDD change `huginn-autonomous-workspace-permissions`.
It is not Huginn production code and does not authorize production sandboxing, grants,
packaging, signing, or release changes. All authored and generated files are under this
directory. Runtime fixtures are generated under a fresh system temporary directory and
removed when the host action completes.

## Tested environment

- macOS 27.3 (Darwin 27.3.0, arm64)
- Xcode: `/Applications/Xcode.app/Contents/Developer`
- macOS SDK: 27.2
- `security find-identity -v -p codesigning`: `0 valid identities found`
- Local ad-hoc signatures only. No certificate, credential, keychain identity, or
  provisioning profile was requested.

The prototype uses **signed App Sandbox entitlements**. It does **not** prototype
Codex's public macOS sandbox approach (`sandbox-exec` with a dynamic Seatbelt profile).
That is a comparison only; `sandbox-exec` is a deprecated interface and was not used.

## Contents

- `Host.swift`: minimal AppKit host. It creates disposable fixtures, requires an
  `NSOpenPanel` selection of the exact generated `chosen-workspace`, creates a
  security-scoped bookmark, and passes the bookmark data and generated fixture paths
  explicitly to the helper. It rejects any other selected path.
- `Helper.swift`: resolves the bookmark and probes as the helper. `/bin/sh -c` performs
  read/list/create/modify with shell builtins and emits its own `shell` results. For
  replace, the shell launches a nested helper process that performs and reports the
  operation as `nested`. Paths are checked lexically against the host-provided fixture
  root; the helper does not read a grant-ancestor marker before the tested operation.
- `Host.entitlements.plist`: App Sandbox plus user-selected read/write. No `inherit`.
- `Helper.entitlements.plist`: exactly `app-sandbox` and `inherit`.
- `probe.swift`, `harness.py`: deterministic unsandboxed baseline harness. It tests
  read, list, create, modify, and replace for selected, unselected, fake grant storage,
  and grant-directory ancestor fixtures. Each process role is recorded separately.
- `baseline-output.jsonl`: raw RED run output. Temporary paths refer to an already
  removed disposable fixture.
- `build.sh`: compiles and ad-hoc signs the app and helper; verifies both signatures.
- `WorkspaceSelectionTests.swift`: UI-free focused tests for selection validation and
  per-execution panel configuration.
- `test_process_chain.py`: focused executable chain test, including an executable path
  containing spaces.

## Commands and observed output

Build and inspect signatures:

```sh
sh build.sh
APP=build/AppSandboxPrototype.app
codesign -d --entitlements :- "$APP"
codesign -d --entitlements :- "$APP/Contents/Helpers/ProbeHelper"
codesign -dv --verbose=4 "$APP"
codesign -dv --verbose=4 "$APP/Contents/Helpers/ProbeHelper"
```

Observed build result: both targets compiled for arm64 and `codesign --verify
--strict --verbose=2` reported valid on disk and satisfied designated requirements.
The host signature reported `Signature=adhoc`, no TeamIdentifier, App Sandbox and
`files.user-selected.read-write`, with no `inherit`. The helper signature reported
`Signature=adhoc`, no TeamIdentifier, and exactly App Sandbox plus `inherit`.
Ad-hoc signature validity does not establish that App Sandbox was enforced.

The focused RED baseline was run before the host/helper experiment:

```sh
swiftc probe.swift -o probe
python3 harness.py "$PWD/probe" baseline
```

Observed result:

```text
exit status: 1 (expected RED)
SUMMARY mode=baseline probes=160 expectation_failures=140
RED baseline outside_access_successes=140; sandbox boundary absent
```

All 20 selected-workspace controls succeeded. All 140 expected-denied probes against
the unselected workspace and fake grant location/ancestors succeeded without a
sandbox, so the denial assertions failed as expected. This is only a baseline control,
not App Sandbox evidence.
The probe executable also refused a deliberate invocation targeting `$HOME` with
`resource is not in a generated baseline fixture; refusing access` (exit 1); it did
not read or modify that path.

## Interactive attempt #1 and panel state hypothesis

Manual panel attempt #1 ended in a selection mismatch. The cause remains unknown. AppKit
persists `NSSavePanel` state by `identifier`, and the prototype did not assign one. State
restoration is therefore a plausible hypothesis, not a confirmed explanation of the capture.
The old host appended the validation diagnostic, then threw; its catch handler replaced the output with a
generic `BLOCKED/ERROR` message. The expected and received paths were therefore not visible.
The host also removed its disposable fixture when the attempt ended, so those paths cannot be
recovered. Do not infer which path the panel returned or why the mismatch occurred.

The new focused selection regression test verifies this exact rejection format for an
in-fixture mismatch:

```text
BLOCKED: workspace_selection expected=<temporary-fixture>/chosen-workspace received=<temporary-fixture>/sibling-workspace selected_basename=sibling-workspace result=blocked
```

For a selected path outside the generated fixture, the received field is
`received=<outside temporary fixture>`; the unrelated external path is not included. The same
test verifies that the mismatch decision does not launch the helper. `Host.swift` now clears
the output at the start of each attempt, displays the decision message, and returns before
helper launch on mismatch. Later errors append to the visible diagnostic instead of replacing
it. The selection suite passed after the regression fix. The process-chain test and app
compile/sign checks also passed; signed entitlements are listed above. These checks do not
exercise sandbox enforcement.

The host now assigns each `NSOpenPanel` a fresh identifier and sets its initial directory
to the current disposable fixture immediately before `runModal()`. Configuration verifies
that the fixture is an existing directory and that AppKit retained the requested identifier
and directory. If this check fails, the host reports an error and does not open the panel or
launch the helper. The focused configuration test creates an `NSOpenPanel` but does not
present it. Run it with:

```sh
swiftc WorkspaceSelection.swift WorkspaceSelectionTests.swift -framework AppKit -o build/WorkspaceSelectionTests
build/WorkspaceSelectionTests
```

Do not open the panel again as part of this code change. A new human-operated run is still
required to determine the cause of the first mismatch and to test bookmark transfer and the
helper/shell chain. The identifier change tests the state-restoration hypothesis; it does not
confirm that hypothesis.

When a human can operate the visible panel, run the two launch routes independently.
For GUI entry, open the app and click its button. For Terminal entry, run:

```sh
open "$PWD/build/AppSandboxPrototype.app" --args --terminal-entry
```

In each run, select only the generated `chosen-workspace` displayed under the new
`huginn-app-sandbox-*` temporary directory. Cancel if the panel does not show that
fixture. The host will reject any other selection and remove its fixture after the
probe run. Capture unified logging in a separate Terminal before each run, for example:

```sh
log stream --style compact --predicate \
  'process == "AppSandboxPrototype" OR process == "ProbeHelper" OR subsystem == "com.apple.sandbox"'
```

Correlate each observed denial with its process role, resource, operation, and sandbox
diagnostic before calling it an OS-enforced denial. Repeat the full matrix for both
launch routes. A path error, missing resource, process startup failure, or absent
correlated diagnostic is not a pass. Stop if the selected-workspace access fails, if
any unselected/grant-ancestor operation succeeds, or if ad-hoc signing does not result
in enforceable sandbox denials.

## Current conclusion and limits

**Inconclusive / blocked; no App Sandbox enforcement result.** The signed host/helper
and entitlement artifacts build and verify. One user-operated panel attempt occurred and
ended in a selection mismatch; the old catch handler hid the expected/received paths. The
host rejected that mismatch before bookmark transfer or helper launch. A successful selected-
workspace run, `/bin/sh`, nested child, per-operation denial matrix, correlated sandbox
diagnostics, GUI-versus-Terminal comparison, and ad-hoc enforcement have not been exercised.
Therefore there is no feasibility pass or production recommendation. Task 2.3 remains
incomplete. Keep grants dormant and do not begin production work.

The baseline uses distinct role labels to exercise the same unsandboxed executable;
it does not simulate process inheritance. Host performs host-side operations, helper
performs helper-side operations, `/bin/sh` performs shell-builtin operations, and the
nested ProbeHelper performs replace. The interactive signed host run and enforcement
diagnostics remain unobserved. macOS-only source and signatures say nothing about Linux
or Windows, nor do they resolve the independent managed-source verifier requirement.
