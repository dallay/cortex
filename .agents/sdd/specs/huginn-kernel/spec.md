# huginn-kernel — Specification

## Purpose

Cooperative service provider substitution, draining, and lifecycle leases.

## Requirements

### Requirement: KRN-01 — In-Process Provider Substitution

The supervisor MUST support replacing an active service provider without process restart.
The replacement provider MUST publish all services provided by the replaced provider.
It MAY declare new services only if no conflicting registrations exist.

#### Scenario: KRN-01-A — Successful substitution

- GIVEN active provider P1 publishing service S
- WHEN supervisor substitutes P2 for P1
- THEN dependents of S transition cleanly to P2

### Requirement: KRN-02 — Lease Admission and Draining

Before committing a substitution, the supervisor MUST close lease admission for the old
provider, execute drain hooks, and wait for active leases within a finite timeout.
Operations with active leases MUST run to completion without premature cancellation.

#### Scenario: KRN-02-A — Clean drain and commit

- GIVEN active operations holding leases on P1
- WHEN substitution begins
- THEN new lease requests are gated, active leases finish, and P2 is committed

#### Scenario: KRN-02-B — Drain failure and abort

- GIVEN a drain timeout or activation failure on P2
- WHEN substitution aborts
- THEN P1 gates are reopened and attempted dependents resumed
