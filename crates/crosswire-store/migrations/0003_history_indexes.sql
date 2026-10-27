-- Crosswire project schema, version 3 (additive). Documented in docs/format.md.
-- Secondary indexes for the named history queries (spec §13). Each trails
-- `ulid` so a `req_host = ? ORDER BY ulid DESC` keyset pages inside the index
-- (equality on the leading column, the ulid in index order), and the §13
-- queries stop scanning the whole `exchanges` table at a million rows.
-- Idempotent: `IF NOT EXISTS` lets `init` (fresh) and `migrate` (existing)
-- apply the same statements.

CREATE INDEX IF NOT EXISTS idx_exchanges_host ON exchanges (req_host, ulid);
CREATE INDEX IF NOT EXISTS idx_exchanges_method_status
    ON exchanges (req_method, resp_status, ulid);
CREATE INDEX IF NOT EXISTS idx_exchanges_resp_len ON exchanges (resp_len, ulid);
CREATE INDEX IF NOT EXISTS idx_exchanges_scope ON exchanges (scope_state, ulid);
