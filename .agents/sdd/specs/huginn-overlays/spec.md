# huginn-overlays — Specification

## Purpose

Unified modal and overlay management for the Huginn TUI.

## Requirements

### Requirement: OVL-01 — Single Active Overlay

The TUI MUST display at most one active overlay at any given time.
Opening a new overlay MUST supersede any non-critical active overlay,
or reject if a higher-priority overlay (such as Approval) is active.
Overlays MUST NOT be mixed into conversation history.

#### Scenario: OVL-01-A — Overlay exclusivity

- GIVEN an open settings overlay
- WHEN another ordinary overlay is requested
- THEN the previous overlay is cleanly dismissed or replaced without stacking

### Requirement: OVL-02 — Approval Priority

Approval overlays (`ApprovalRequest`) hold strict priority over ordinary overlays
(such as Settings or Dialogs).
An ordinary overlay MUST NOT preempt a pending Approval.
Resolving an Approval does NOT automatically restore previous ordinary overlays.

#### Scenario: OVL-02-A — Approval precedence

- GIVEN an active Approval overlay
- WHEN an ordinary dialog or settings request arrives
- THEN the ordinary request is queued or rejected, and Approval retains focus

### Requirement: OVL-03 — Clean Dismissal and Focus Restoration

Dismissing an overlay via Esc, cancel button, or successful submission MUST
cleanly revoke input grab and return focus to the conversation composer.

#### Scenario: OVL-03-A — Dismissal to composer

- GIVEN any active overlay
- WHEN dismissed via Esc
- THEN focus immediately returns to the primary chat input area
