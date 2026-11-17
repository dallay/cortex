# GM-01 acceptance: conversation lifecycle

## Decision

GM-01 (conversation lifecycle) is accepted with the authorized deviations
GM01-DEV-001 and GM01-DEV-002, documented in `gm01-deviations.md`. This decision
does not establish exact deletion parity, and it does not accept H1, other GM
suites, ANN, MiniLM, C1, or H2/H3.

The current acceptance base is the isolated real run preserved with the H1
evidence (`gm01-exact-bound-20261103-r1`), together with its RED/GREEN checks.
Earlier captures remain historical. The pinned reference
`9d45af5529ecb5f2fe352013468a11087f3163d4` was not modified.

## Evidence and coverage

- `comparison.json`: approved comparison of two sessions and seven messages.
  It covers mixed roles, Unicode, empty content, duplicates, nested metadata,
  and a 10,001-character message.
- `lifecycle.json`: 61 cases, `status=pass`,
  `gm01_acceptance=pass_with_authorized_deviations`, zero errors and zero
  pending items. It checks empty creation and idempotency, public returns and
  reads, UUID identity, uniqueness, references, timestamps, and ordering.
- `interleaved-sessions`: two populated sessions, six interleaved writes, four
  roles, and seven checkpoints of both sessions, including the one after
  reconnecting and restarting.
- `graph-first`, `graph-middle`, `graph-last`: labels, properties, and
  relationships are compared. Only the authorized isolation property is
  discounted, after validating its absence upstream and its exact value in
  Rust.
- `delete-effects-first`, `delete-effects-middle`, `delete-effects-last`: the
  expected graph and DTO are derived from the previous state. The target is
  physically removed, links are repaired, and remaining messages are preserved.
- Upstream deletion: authorized error with fully unchanged public state and
  graph. The exception also requires correct effects in Rust.
- Session clearing: physical absence of the original conversations and
  messages, idempotent repetition, and absence after restart.
- Invalid or nonexistent IDs: `false` return and unchanged state. Invalid role:
  `invalid_input` category and unchanged state. Textual error equality is not
  imposed where the contract does not specify it.
- Reference reconnection and bridge restart over the ephemeral databases:
  public reads and persisted properties are preserved.
- Cleanup verified after the run: the runner exited with code 0 and no GM-01
  containers, networks, or volumes remained. No active data was reset and
  nothing was published.

## Deviations

- **GM01-DEV-001 — deletion.** Upstream raises
  `Neo.ClientError.Schema.ConstraintValidationFailed` when deleting a message
  that belongs to a chain, leaving the chain broken. Rust repairs the
  `FIRST_MESSAGE` and `NEXT_MESSAGE` links and keeps the remaining chain valid.
  Keeping the correct Rust behavior with an unpatched reference is authorized,
  on condition that the Rust deletion effects pass the independent oracle.
- **GM01-DEV-002 — `owner_scope`.** Rust persists
  `owner_scope: "unowned"` on the conversation; upstream does not have that
  property. Discounting only that property is authorized after validating its
  absence upstream and its exact value in Rust. Different values, absence in
  Rust, presence upstream, and any other property still fail the comparison.

## Historical captures

Older captures keep their recorded status and are not rewritten. R13 keeps
`status: incomplete` and its five historical pending items. R4 keeps the
append/read failure caused by different clocks. The current base replaces them
for acceptance purposes only; it does not erase their history.

## Pending acceptance

H1-010 and C1 remain pending. C1 provenance from a protected `main` is explicitly
deferred until integration. The current capture retains
`h1_acceptance=not_established`: a passing GM-01 lifecycle capture is not H1
acceptance, and it is not evidence of full functional parity.
