-- =============================================================================
-- V4: Combos (multi-step fallback chains)
-- =============================================================================

-- Combo definitions
CREATE TABLE combos (
    id              TEXT PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE COLLATE NOCASE,
    strategy        TEXT NOT NULL DEFAULT 'priority',
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,

    -- Constraints
    CHECK (length(name) BETWEEN 1 AND 100),
    CHECK (strategy IN ('priority'))
);

-- Combo steps (ordered by priority within a combo)
CREATE TABLE combo_steps (
    combo_id        TEXT NOT NULL,
    step_order      INTEGER NOT NULL,
    provider_id     TEXT NOT NULL,
    model           TEXT NOT NULL,
    connection_id   TEXT,
    priority        INTEGER NOT NULL CHECK (priority BETWEEN 1 AND 255),

    PRIMARY KEY (combo_id, step_order),
    FOREIGN KEY (combo_id) REFERENCES combos(id) ON DELETE CASCADE,

    -- Ensure priority uniqueness within a combo
    UNIQUE (combo_id, priority)
);

-- Indexes for performance
CREATE UNIQUE INDEX idx_combos_name ON combos(name COLLATE NOCASE);
CREATE INDEX idx_combo_steps_combo_id ON combo_steps(combo_id);
CREATE INDEX idx_combos_created_at ON combos(created_at DESC);
