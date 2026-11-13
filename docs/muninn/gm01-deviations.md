# GM-01: authorized deviations

GM-01 (conversation lifecycle) is compared against the pinned reference
`9d45af5529ecb5f2fe352013468a11087f3163d4`. Two deviations are authorized.
They are narrow: any difference outside their exact scope remains a failure.

## GM01-DEV-001 — Deletion of a chained message

**Observation.** Deleting a message that belongs to a chain makes the reference
raise `Neo.ClientError.Schema.ConstraintValidationFailed` and leave the chain
broken. Rust removes the target node and repairs the chain so that remaining
messages stay reachable and ordered.

**Authorization.** Keep the correct Rust behavior and leave the reference
unpatched. The deviation applies only to this deletion failure. It requires
that the Rust result pass an independent oracle derived from the previous state:
only the target disappears, no dangling links remain, and the remaining
messages and properties are preserved.

**Not covered.** It does not authorize silent differences in deletion results,
error categories, or any other operation.

## GM01-DEV-002 — `owner_scope` property

**Observation.** Rust persists `owner_scope: "unowned"` on the conversation
node. The reference does not have that property.

**Authorization.** Discount only the `owner_scope` property, and only after
verifying both conditions:

- the reference conversation does not contain `owner_scope`;
- the Rust conversation contains exactly `owner_scope = "unowned"`.

**Not covered.** A different value, a missing property in Rust, `owner_scope`
present upstream, or any additional difference in labels, properties, or
relationships still fails. The Rust isolation value must not be removed to
force textual parity.

## Scope

These deviations belong to the GM-01 conversation lifecycle suite. They do not
accept H1, ANN, MiniLM, C1, other GM suites, or H2/H3.
