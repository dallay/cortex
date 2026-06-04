# Combo Repository — Specification

> **Purpose**: This document defines the persistence port for combos, including CRUD operations, query semantics, error handling, and data integrity rules. Technology-agnostic — behavior only.

---

## 1. Overview

The Combo Repository provides persistent storage for combo definitions. It is the single source of truth for combo metadata and steps.

### 1.1 Responsibilities

- Store and retrieve combo aggregates
- Enforce name uniqueness
- Support full CRUD operations
- Maintain referential integrity between combos and steps

### 1.2 Out of Scope

- Runtime provider registration (handled by `ProviderRegistryPort`)
- Provider health checks (handled by circuit breakers)
- Combo execution logic (handled by `RouteRequest`)

---

## 2. Port Interface

### 2.1 ComboRepositoryPort Trait

The system SHALL provide a `ComboRepositoryPort` trait in `rook-core/src/ports.rs`.

```rust
#[async_trait]
pub trait ComboRepositoryPort: Send + Sync {
    async fn list(&self) -> Result<Vec<Combo>, ComboRepositoryError>;
    async fn find(&self, id: &ComboId) -> Result<Option<Combo>, ComboRepositoryError>;
    async fn create(&self, combo: &Combo) -> Result<(), ComboRepositoryError>;
    async fn update(&self, combo: &Combo) -> Result<(), ComboRepositoryError>;
    async fn delete(&self, id: &ComboId) -> Result<(), ComboRepositoryError>;
}
```

**Error Type**:

```rust
pub enum ComboRepositoryError {
    NotFound(ComboId),
    DuplicateName(String),
    Database(String),
}
```

---

## 3. Operations

### Requirement: List All Combos

The system MUST provide a `list()` method that returns all combos ordered by `created_at` descending (newest first).

#### Scenario: Empty repository returns empty list

- GIVEN no combos exist in the repository
- WHEN `list()` is called
- THEN an empty Vec is returned
- AND no error is raised

#### Scenario: Multiple combos returned in order

- GIVEN 3 combos exist: "combo-A" (created 2026-06-01), "combo-B" (created 2026-06-02), "combo-C" (created 2026-06-03)
- WHEN `list()` is called
- THEN 3 combos are returned
- AND they are ordered: combo-C, combo-B, combo-A (newest first)

#### Scenario: Each combo includes all steps

- GIVEN a combo with 3 steps exists
- WHEN `list()` is called
- THEN the returned combo includes all 3 steps
- AND steps are ordered by priority ascending

---

### Requirement: Find Combo by ID

The system MUST provide a `find(id)` method that returns a combo by its ID, or `None` if not found.

#### Scenario: Existing combo found

- GIVEN a combo with ID "550e8400-e29b-41d4-a716-446655440000" exists
- WHEN `find("550e8400-e29b-41d4-a716-446655440000")` is called
- THEN `Some(Combo)` is returned
- AND the combo includes all fields and steps

#### Scenario: Non-existent combo returns None

- GIVEN no combo with ID "00000000-0000-0000-0000-000000000000" exists
- WHEN `find("00000000-0000-0000-0000-000000000000")` is called
- THEN `None` is returned
- AND no error is raised

#### Scenario: Invalid UUID format returns error

- GIVEN an invalid UUID string "not-a-uuid"
- WHEN `find("not-a-uuid")` is called (if string parsing happens before repo call)
- THEN parsing fails before the repository is reached
- AND a parse error is returned (not a repository error)

---

### Requirement: Create Combo

The system MUST provide a `create(combo)` method that persists a new combo and its steps.

**Pre-conditions**:

- Combo has a unique name
- Combo has at least 1 step
- Combo validation has passed (domain layer responsibility)

#### Scenario: Valid combo created successfully

- GIVEN a valid combo with name "main-chain" and 3 steps
- AND no combo with name "main-chain" exists
- WHEN `create(combo)` is called
- THEN the combo is persisted
- AND all 3 steps are persisted with correct priorities
- AND `find(combo.id)` returns the combo

#### Scenario: Duplicate name rejected

- GIVEN a combo with name "main-chain" already exists
- WHEN `create(combo)` is called with a new combo also named "main-chain"
- THEN the operation fails with `ComboRepositoryError::DuplicateName("main-chain")`
- AND no new combo is created

#### Scenario: Steps persisted with combo

- GIVEN a combo with 2 steps (priorities 1 and 2)
- WHEN `create(combo)` is called
- THEN the combo row is created
- AND 2 step rows are created linked to the combo ID
- AND steps are retrieved correctly via `find(combo.id)`

#### Scenario: Transaction rollback on partial failure

- GIVEN a combo creation request
- WHEN step persistence fails after combo row is inserted
- THEN the entire transaction is rolled back
- AND no combo or steps are persisted
- AND `find(combo.id)` returns `None`

---

### Requirement: Update Combo

The system MUST provide an `update(combo)` method that replaces a combo's name, strategy, and steps.

**Pre-conditions**:

- Combo with given ID exists
- New name is unique (or unchanged)
- Combo validation has passed

**Semantics**:

- All steps are replaced (deleted + inserted)
- `updated_at` timestamp is refreshed

#### Scenario: Combo name updated successfully

- GIVEN a combo with ID "abc" and name "old-name" exists
- WHEN `update(combo)` is called with same ID but name "new-name"
- THEN the combo's name is updated
- AND `updated_at` is refreshed
- AND `find(abc)` returns the combo with name "new-name"

#### Scenario: Combo steps replaced

- GIVEN a combo with 3 steps exists
- WHEN `update(combo)` is called with 2 different steps
- THEN the old 3 steps are deleted
- AND the new 2 steps are inserted
- AND `find(combo.id)` returns the combo with 2 steps

#### Scenario: Update non-existent combo fails

- GIVEN no combo with ID "missing" exists
- WHEN `update(combo)` is called for ID "missing"
- THEN the operation fails with `ComboRepositoryError::NotFound("missing")`
- AND no changes are made

#### Scenario: Update with duplicate name fails

- GIVEN combo "A" with name "name-A" exists
- AND combo "B" with name "name-B" exists
- WHEN `update(combo-B)` is called with name "name-A"
- THEN the operation fails with `ComboRepositoryError::DuplicateName("name-A")`
- AND combo "B" remains unchanged

#### Scenario: Transaction rollback on partial update failure

- GIVEN a combo update request
- WHEN step deletion succeeds but step insertion fails
- THEN the entire transaction is rolled back
- AND the combo remains in its original state
- AND `find(combo.id)` returns the original combo

---

### Requirement: Delete Combo

The system MUST provide a `delete(id)` method that removes a combo and all its steps.

**Semantics**:

- Cascade delete: combo row + all step rows
- Idempotent: deleting a non-existent combo succeeds silently

#### Scenario: Existing combo deleted successfully

- GIVEN a combo with ID "abc" and 3 steps exists
- WHEN `delete("abc")` is called
- THEN the combo row is deleted
- AND all 3 step rows are deleted
- AND `find("abc")` returns `None`

#### Scenario: Deleting non-existent combo succeeds

- GIVEN no combo with ID "missing" exists
- WHEN `delete("missing")` is called
- THEN no error is raised
- AND the operation succeeds (idempotent)

#### Scenario: Steps deleted with combo

- GIVEN a combo with 2 steps exists
- WHEN `delete(combo.id)` is called
- THEN the combo row is deleted
- AND both step rows are deleted
- AND no orphaned step rows remain

---

## 4. Data Integrity

### Requirement: Name Uniqueness Constraint

The system MUST enforce combo name uniqueness at the database level.

#### Scenario: Concurrent duplicate name creation fails

- GIVEN two concurrent `create()` calls for combos with name "main-chain"
- WHEN both calls reach the database
- THEN one succeeds and one fails with `DuplicateName`
- AND only one combo with name "main-chain" exists

---

### Requirement: Referential Integrity for Steps

The system MUST ensure combo steps cannot exist without a parent combo.

#### Scenario: Deleting combo removes all steps

- GIVEN a combo with 5 steps exists
- WHEN the combo is deleted
- THEN all 5 step rows are deleted
- AND no orphaned step rows remain in the database

---

## 5. Performance Requirements

| Operation  | Max Latency | Notes                                              |
|------------|-------------|----------------------------------------------------|
| `list()`   | 10ms        | Query with index on `created_at`                   |
| `find(id)` | 1ms         | Primary key lookup + join to steps                 |
| `create()` | 5ms         | Insert combo + steps in transaction                |
| `update()` | 10ms        | Delete old steps + insert new steps in transaction |
| `delete()` | 5ms         | Delete combo + cascade delete steps                |

**Indexing Requirements**:

- Primary key index on `combos.id`
- Unique index on `combos.name`
- Index on `combos.created_at` for `list()` ordering
- Foreign key index on `combo_steps.combo_id` for joins

---

## 6. Error Handling

### Requirement: Repository Error Types

The system MUST return structured errors for all failure modes.

| Error                                       | When                                         | HTTP Status |
|---------------------------------------------|----------------------------------------------|-------------|
| `ComboRepositoryError::NotFound(id)`        | Combo with ID not found during update/delete | 404         |
| `ComboRepositoryError::DuplicateName(name)` | Combo with same name already exists          | 409         |
| `ComboRepositoryError::Database(msg)`       | Unexpected database error                    | 500         |

#### Scenario: Database connection failure returns Database error

- GIVEN the database is unreachable
- WHEN any repository method is called
- THEN `ComboRepositoryError::Database("connection failed")` is returned
- AND the error message contains diagnostic information

#### Scenario: Transaction deadlock returns Database error

- GIVEN a transaction deadlock occurs during `update()`
- WHEN the operation fails
- THEN `ComboRepositoryError::Database("deadlock detected")` is returned
- AND the transaction is rolled back

---

## 7. Transaction Semantics

### Requirement: ACID Guarantees

All write operations (`create`, `update`, `delete`) MUST be atomic.

#### Scenario: Combo and steps created atomically

- GIVEN a combo with 3 steps is being created
- WHEN step 2 insertion fails
- THEN the combo row is NOT persisted
- AND step 1 row is NOT persisted
- AND the database remains in its original state

#### Scenario: Combo and steps deleted atomically

- GIVEN a combo with 5 steps is being deleted
- WHEN step deletion fails mid-operation
- THEN the combo row is NOT deleted
- AND all step rows remain
- AND the database remains in its original state

---

## 8. Observability

### Requirement: Repository Instrumentation

All repository operations MUST emit structured logs and metrics.

#### Scenario: Successful operations logged at DEBUG level

- GIVEN a `create()` call succeeds
- WHEN the operation completes
- THEN a DEBUG log is emitted: "Created combo id={id} name={name} steps={count}"

#### Scenario: Failed operations logged at WARN level

- GIVEN a `create()` call fails with `DuplicateName`
- WHEN the operation fails
- THEN a WARN log is emitted: "Combo creation failed: duplicate name={name}"

#### Scenario: Repository metrics tracked

- GIVEN repository operations are running
- WHEN operations complete
- THEN the following metrics are incremented:
    - `combo_repository_calls_total{operation="create", status="success|error"}`
    - `combo_repository_duration_ms{operation="create"}`
