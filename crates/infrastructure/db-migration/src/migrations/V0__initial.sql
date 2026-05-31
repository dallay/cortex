-- V0__initial.sql
-- Creates all initial tables for provider, auth, and audit
-- Idempotent: uses CREATE TABLE IF NOT EXISTS

-- =============================================================================
-- provider_connections table
-- =============================================================================
CREATE TABLE IF NOT EXISTS provider_connections (
    id                  TEXT PRIMARY KEY,
    provider_kind       TEXT    NOT NULL,
    provider_runtime_id TEXT    NOT NULL,
    name                TEXT    NOT NULL,
    auth_type           TEXT    NOT NULL CHECK (auth_type IN ('apiKey', 'oauth')),
    priority            INTEGER NOT NULL CHECK (priority BETWEEN 1 AND 255),
    is_active           INTEGER NOT NULL CHECK (is_active IN (0, 1)),
    api_key_ct          TEXT,
    oauth_email_ct      TEXT,
    access_token_ct     TEXT,
    refresh_token_ct    TEXT,
    scope_ct            TEXT,
    id_token_ct         TEXT,
    project_id_ct       TEXT,
    expires_at          INTEGER,
    max_concurrent      INTEGER NOT NULL CHECK (max_concurrent >= 1),
    quota_warning       REAL    NOT NULL,
    quota_error         REAL    NOT NULL,
    default_model       TEXT,
    test_status         TEXT    NOT NULL,
    test_latency_ms     INTEGER,
    test_error          TEXT,
    test_expires_at     INTEGER,
    last_test_at        TEXT,
    base_url            TEXT,
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL,
    UNIQUE (provider_kind, name)
);

CREATE INDEX IF NOT EXISTS idx_pc_provider_kind ON provider_connections (provider_kind);
CREATE INDEX IF NOT EXISTS idx_pc_runtime_id ON provider_connections (provider_runtime_id);
CREATE INDEX IF NOT EXISTS idx_pc_active ON provider_connections (is_active) WHERE is_active = 1;
CREATE INDEX IF NOT EXISTS idx_pc_priority_created ON provider_connections (priority ASC, created_at DESC);

-- =============================================================================
-- api_keys table
-- =============================================================================
CREATE TABLE IF NOT EXISTS api_keys (
    id           TEXT PRIMARY KEY,
    label        TEXT NOT NULL,
    key_hash     TEXT NOT NULL UNIQUE,
    key_prefix   TEXT NOT NULL,
    scopes_json  TEXT NOT NULL,
    tier         TEXT NOT NULL CHECK (tier IN ('free', 'pro', 'enterprise')),
    is_active    INTEGER NOT NULL CHECK (is_active IN (0, 1)),
    revoked_at   TEXT,
    expires_at   TEXT,
    created_at   TEXT NOT NULL,
    last_used_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_api_keys_active ON api_keys (is_active);
CREATE INDEX IF NOT EXISTS idx_api_keys_revoked_at ON api_keys (revoked_at);
CREATE INDEX IF NOT EXISTS idx_api_keys_expires_at ON api_keys (expires_at);

-- =============================================================================
-- users table
-- =============================================================================
CREATE TABLE IF NOT EXISTS users (
    id           TEXT PRIMARY KEY,
    username     TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_users_username ON users (username COLLATE NOCASE);

-- =============================================================================
-- sessions table
-- =============================================================================
CREATE TABLE IF NOT EXISTS sessions (
    id          TEXT PRIMARY KEY,
    token_hash  TEXT NOT NULL UNIQUE,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL,
    expires_at  TEXT NOT NULL,
    revoked     INTEGER NOT NULL DEFAULT 0 CHECK (revoked IN (0, 1))
);

CREATE INDEX IF NOT EXISTS idx_sessions_token_hash ON sessions (token_hash);
CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_expires_at ON sessions (expires_at);

-- =============================================================================
-- audit table
-- =============================================================================
CREATE TABLE IF NOT EXISTS audit (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id  TEXT NOT NULL,
    provider    TEXT NOT NULL,
    model       TEXT NOT NULL,
    status      TEXT NOT NULL,
    prompt_tokens      INTEGER,
    completion_tokens  INTEGER,
    total_tokens       INTEGER,
    estimated_cost_usd REAL,
    latency_ms  INTEGER NOT NULL,
    timestamp   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_request_id ON audit(request_id);
CREATE INDEX IF NOT EXISTS idx_audit_provider ON audit(provider);
CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit(timestamp);
