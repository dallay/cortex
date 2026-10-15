-- =============================================================================
-- V7: Combo aliases (virtual models, e.g. 'rook/coding' -> combo_id)
-- =============================================================================
-- Separate from model_aliases (alias -> canonical model). A combo alias maps a
-- reserved 'rook/*' virtual model to a combo fallback chain. Exact match only.

CREATE TABLE IF NOT EXISTS combo_aliases (
    alias TEXT PRIMARY KEY,
    combo_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    FOREIGN KEY (combo_id) REFERENCES combos(id) ON DELETE CASCADE,
    CHECK (alias GLOB 'rook/*' AND length(alias) BETWEEN 6 AND 69)
);

CREATE INDEX IF NOT EXISTS idx_combo_aliases_combo_id ON combo_aliases(combo_id);
