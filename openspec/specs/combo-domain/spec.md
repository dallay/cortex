# Combo Domain Model — Specification

> **Purpose**: This document defines the domain model for multi-step fallback chains (combos), including types, validation rules, and invariants. Technology-agnostic — behavior only.

---

## 1. Overview

The Combo domain model represents ordered fallback chains for LLM requests. A combo is a named sequence of provider+model steps that are tried in priority order until one succeeds.

### 1.1 Key Identifiers

| Identifier   | Type    | Format                             | Meaning                         |
|--------------|---------|------------------------------------|---------------------------------|
| `ComboId`    | UUID v4 | String representation              | Unique identifier for a combo   |
| `ProviderId` | String  | Existing type from `shared-kernel` | Reference to a runtime provider |
| `ModelId`    | String  | Existing type from `shared-kernel` | Reference to a model            |

---

## 2. Domain Types

### 2.1 ComboId

A unique identifier for a combo, backed by UUID v4.

**Constraints**:

- MUST be a valid UUID v4
- System-generated on creation, immutable thereafter
- Used for HTTP routing via `X-Rook-Combo` header
- Used for config references via `routing.default_combo`

**Operations**:

- `new() -> ComboId` — generates a new UUID v4
- `parse_str(s: &str) -> Result<ComboId, Error>` — parses from string

### 2.2 Combo

The aggregate root for a fallback chain.

| Field        | Type             | Constraints                                               |
|--------------|------------------|-----------------------------------------------------------|
| `id`         | `ComboId`        | System-generated, immutable                               |
| `name`       | String           | Non-empty, 1-100 Unicode scalar values, unique per system |
| `strategy`   | `ComboStrategy`  | Only `Priority` in MVP                                    |
| `steps`      | `Vec<ComboStep>` | 1-10 steps, priority uniqueness enforced within combo     |
| `created_at` | `DateTime<Utc>`  | System-generated at creation                              |
| `updated_at` | `DateTime<Utc>`  | System-generated at every mutation                        |

**Invariants**:

- Name MUST be non-empty and trimmed
- Name MUST be unique across all combos in the system
- Steps MUST contain at least 1 entry
- Steps MUST contain at most 10 entries
- Each step's `provider_id` MUST reference a valid runtime provider (validated at execution time with warning if missing)
- Each step's `priority` MUST be unique within the combo
- Each step's `priority` MUST be a positive integer (1-255)

### 2.3 ComboStep

A single step in a combo fallback chain.

| Field         | Type         | Constraints                                      |
|---------------|--------------|--------------------------------------------------|
| `provider_id` | `ProviderId` | Reference to runtime provider                    |
| `model`       | `ModelId`    | Model identifier for this provider               |
| `priority`    | `u8` (1-255) | Lower number = higher priority, unique per combo |

**Semantics**:

- Steps are executed in ascending priority order (1 first, then 2, etc.)
- Each step represents a complete provider+model pair
- The `provider_id` MUST exist in the runtime provider registry at execution time
- The `model` is passed to the provider implementation — no validation at combo creation time

### 2.4 ComboStrategy

An enum representing the execution strategy for a combo.

```
Priority — execute steps in priority order until one succeeds
```

**MVP Constraint**: Only `Priority` strategy is supported. Other strategies (WeightedRandom, RoundRobin, P2C, FillFirst) are deferred to future work.

---

## 3. Validation Rules

### Requirement: Combo Name Validation

The system MUST validate combo names on creation and update.

**Rules**:

- Name MUST NOT be empty or whitespace-only
- Name MUST be between 1 and 100 Unicode scalar values
- Name MUST be unique across all combos
- Name is case-sensitive for uniqueness checks

#### Scenario: Valid combo name accepted

- GIVEN a combo creation request with name "OpenAI → Anthropic → Ollama"
- WHEN validation runs
- THEN the name is accepted
- AND the combo is created

#### Scenario: Empty name rejected

- GIVEN a combo creation request with name ""
- WHEN validation runs
- THEN the request is rejected with `VALIDATION_ERROR`
- AND the error message states "Combo name must not be empty"

#### Scenario: Duplicate name rejected

- GIVEN a combo with name "main-chain" already exists
- WHEN a creation request arrives with name "main-chain"
- THEN the request is rejected with `DUPLICATE_COMBO_NAME`
- AND the error message states "Combo with name 'main-chain' already exists"

#### Scenario: Name length exceeded

- GIVEN a combo creation request with name longer than 100 characters
- WHEN validation runs
- THEN the request is rejected with `VALIDATION_ERROR`
- AND the error message states "Combo name must not exceed 100 characters"

---

### Requirement: Combo Steps Validation

The system MUST validate combo steps on creation and update.

**Rules**:

- Steps array MUST contain at least 1 entry
- Steps array MUST contain at most 10 entries
- Each step's `priority` MUST be between 1 and 255
- Each step's `priority` MUST be unique within the combo
- Each step's `provider_id` MUST be a non-empty string
- Each step's `model` MUST be a non-empty string

#### Scenario: Valid steps accepted

- GIVEN a combo with 3 steps, priorities 1, 2, 3
- WHEN validation runs
- THEN all steps are accepted
- AND the combo is created

#### Scenario: Empty steps array rejected

- GIVEN a combo creation request with empty steps array
- WHEN validation runs
- THEN the request is rejected with `VALIDATION_ERROR`
- AND the error message states "Combo must have at least 1 step"

#### Scenario: Too many steps rejected

- GIVEN a combo creation request with 11 steps
- WHEN validation runs
- THEN the request is rejected with `VALIDATION_ERROR`
- AND the error message states "Combo must have at most 10 steps"

#### Scenario: Duplicate priority rejected

- GIVEN a combo with steps having priorities [1, 2, 2]
- WHEN validation runs
- THEN the request is rejected with `VALIDATION_ERROR`
- AND the error message states "Duplicate priority '2' in combo steps"

#### Scenario: Priority out of range rejected

- GIVEN a combo with a step having priority 0
- WHEN validation runs
- THEN the request is rejected with `VALIDATION_ERROR`
- AND the error message states "Priority must be between 1 and 255"

#### Scenario: Empty provider_id rejected

- GIVEN a combo with a step where `provider_id` is empty
- WHEN validation runs
- THEN the request is rejected with `VALIDATION_ERROR`
- AND the error message states "Step provider_id must not be empty"

#### Scenario: Empty model rejected

- GIVEN a combo with a step where `model` is empty
- WHEN validation runs
- THEN the request is rejected with `VALIDATION_ERROR`
- AND the error message states "Step model must not be empty"

---

### Requirement: Provider Reference Validation at Execution Time

The system MUST validate provider references when a combo is executed, NOT at creation time.

**Rationale**: Providers may be added/removed dynamically. Combos reference runtime providers by ID, and availability is checked at request time.

#### Scenario: Missing provider logged with warning

- GIVEN a combo with step referencing provider "openai-primary"
- AND provider "openai-primary" does not exist in runtime registry
- WHEN the combo is executed
- THEN a warning is logged: "Provider 'openai-primary' not found in registry, skipping step"
- AND the step is skipped
- AND execution continues to the next step

#### Scenario: Provider exists at execution time

- GIVEN a combo with step referencing provider "openai-primary"
- AND provider "openai-primary" exists in runtime registry
- WHEN the combo is executed
- THEN the step is attempted
- AND no warning is logged

---

## 4. Domain Errors

| Error Type                | HTTP Status | Description                                                       |
|---------------------------|-------------|-------------------------------------------------------------------|
| `ComboNotFoundError`      | 404         | Combo with given ID does not exist                                |
| `DuplicateComboNameError` | 409         | Combo with given name already exists                              |
| `InvalidComboError`       | 400         | Validation failed (name, steps, priority)                         |
| `ProviderNotFoundWarning` | -           | Provider referenced in step does not exist (logged, not an error) |

---

## 5. Type Summary

### ComboId

- Newtype wrapper over UUID v4
- Implements: `Clone`, `PartialEq`, `Eq`, `Hash`, `Debug`, `Display`
- Serializes as string in JSON/TOML

### Combo

- Aggregate root
- Contains: `id`, `name`, `strategy`, `steps`, `created_at`, `updated_at`
- Enforces all invariants on construction

### ComboStep

- Value object
- Contains: `provider_id`, `model`, `priority`
- Immutable after creation

### ComboStrategy

- Enum with single variant: `Priority`
- Serializes as lowercase string: `"priority"`

---

## 6. Non-Functional Requirements

### Performance

- Combo validation MUST complete in <1ms
- In-memory combo lookups MUST complete in <1ms (index on `id`)

### Observability

- All validation failures MUST emit structured log events with field `validation_error`
- Provider reference warnings MUST emit structured log events with field `provider_missing`
