-- V13__api_key_spending_budgets.sql
-- Optional per-key spending policy serialized as JSON. NULL means no policy.
ALTER TABLE api_keys ADD COLUMN spending_budget_json TEXT;
