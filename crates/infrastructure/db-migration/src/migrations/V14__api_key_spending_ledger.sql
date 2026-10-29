-- Durable per-request ledger for strict API-key spending budgets.
CREATE TABLE api_key_spending_reservations (
    api_key_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    window_start TEXT NOT NULL,
    window_end TEXT NOT NULL,
    reserved_micros_usd INTEGER NOT NULL CHECK (reserved_micros_usd > 0),
    budget_micros_usd INTEGER NOT NULL CHECK (budget_micros_usd > 0),
    state TEXT NOT NULL CHECK (state IN ('reserved', 'settled', 'unresolved', 'released')),
    settled_micros_usd INTEGER CHECK (settled_micros_usd IS NULL OR settled_micros_usd >= 0),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (api_key_id, request_id),
    CHECK (window_start < window_end),
    CHECK (reserved_micros_usd <= budget_micros_usd),
    CHECK (settled_micros_usd IS NULL OR settled_micros_usd <= reserved_micros_usd),
    CHECK (
        (state = 'settled' AND settled_micros_usd IS NOT NULL)
        OR (state IN ('reserved', 'unresolved', 'released') AND settled_micros_usd IS NULL)
    )
);

CREATE INDEX idx_api_key_spending_window
    ON api_key_spending_reservations(api_key_id, window_start, window_end, state);
