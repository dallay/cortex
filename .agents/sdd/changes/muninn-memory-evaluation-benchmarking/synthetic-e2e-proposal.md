# Synthetic E2E Proposal — Approved Protocol

## Status

Approved by the user on 2026-11-10. This approval covers the scenario and comparison protocol only. It does not authorize model or database startup, inference, or evaluation execution before the remaining pre-run gates pass.

## Scenario

- Version: `synthetic-memory-recall-v1`.
- Use only synthetic records created for this run. Do not use an external dataset.
- In a newly provisioned, isolated, disposable local Muninn database, seed one owner-scoped message:
  `The release workspace identifier is Vesper-Delta-704.`
- Seed one owner-scoped preference:
  `For the campaign status report, use the title Juniper Harbor and the columns Task, Owner, Status in that order.`
- The task prompt asks Huginn to create `summary.md` with the project identifier, report title, and ordered columns. The prompt does not contain the seeded values.
- Keep the message and preference under one host-configured owner scope. Do not expose scope selection to the model.

## Comparison

Run one paired comparison, with no repetitions:

1. **Memory unavailable baseline:** Start the same retrieval-only MCP server with a different fixed owner scope that has no seeded records in the isolated database.
2. **Memory available:** Start the same MCP server with the fixed owner scope containing the two synthetic records.

Run the baseline first. Use separate new Huginn sessions and separate workspace copies with identical fixture contents. Keep the model/provider, prompt, system instructions, tools, context budget, and permissions constant. Keep both retrieval tools available in both conditions. Obtain the normal server-start approval and each required per-call approval; do not bypass `HostPolicy`.

## Observable assertions

- In the memory-available condition, `summary.md` must contain the exact identifier `Vesper-Delta-704`, title `Juniper Harbor`, and columns `Task`, `Owner`, `Status` in the specified order.
- Evidence must show that the message and preference retrieval tools were called under the host-fixed scope and returned only the matching owner's records.
- In the baseline condition, `summary.md` must use `UNKNOWN` for unavailable fields and must not contain either seeded canary value. The baseline must not fabricate the missing values.
- Record each assertion as pass/fail from the file contents and tool-call evidence. Do not use an LLM judge as the sole outcome source.
- Any cross-owner result, unexpected write capability, denied approval, tool/protocol error, missing required evidence, or isolation failure blocks the run or invalidates the affected condition. Do not retry with shared or persistent state.

## Session history and evidence

- Use fresh Huginn sessions for both conditions. No conversation history containing the seeded values may enter either task prompt or workspace.
- Use one new run-specific evidence directory. Retain only the secret-safe run manifest, session/tool-call evidence needed to verify assertions, the two output files, and a descriptive report. Do not record credentials or unrelated user data.
- Report the two per-condition outcomes and any deviations. Do not infer general memory quality, product parity, or statistical significance from this single scenario.

## Narrow claim

If all gates pass, the run can support only this claim: *Under the recorded local configuration, the approved read-only Muninn MCP connector retrieved the two planted owner-scoped synthetic facts and enabled Huginn to reproduce them in one task, while the empty-scope baseline abstained.* It does not establish general retrieval quality or Agent Memory parity.

## Pre-run gates

- User approves this scenario, assertions, baseline semantics, session-history treatment, evidence plan, and narrow claim.
- User identifies or explicitly approves already-ready local generation and embedding models/providers. Both must be compatible and available without credential inspection, downloads, external network access, or cost. If either is unavailable, stop.
- A unique isolated disposable local Muninn database, Huginn session store, workspaces, and new evidence destination are verified before startup. Do not reset or reuse persistent/shared data.
- Connector, owner-isolation, bounds/timeout, protocol, approval, and untrusted-output checks pass. The server uses only read-only message/preference tools.
- If any gate fails, stop before starting the evaluation server or performing inference. No E2E run is authorized by this draft alone.
