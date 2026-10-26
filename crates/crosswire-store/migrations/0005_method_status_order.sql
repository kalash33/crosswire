-- Crosswire project schema, version 5. Documented in docs/format.md.
-- Reshapes the method/status index for the `status >= N AND method = M
-- ORDER BY ulid DESC` history query (spec §13). The v3 shape
-- `(req_method, resp_status, ulid)` could not deliver `ulid` order across a
-- status RANGE, so SQLite fetched every match from the primary b-tree and
-- temp-sorted them (about 12 s at a million rows). With `ulid` second, the
-- equality prefix `req_method` iterates in `ulid DESC` order (the keyset
-- `ulid < ?` still seeks inside the index) and `resp_status` is filtered from
-- the index entry itself, with no temp sort.
--
-- The list columns trail the key so the index COVERS the list statement.
-- Without them, SQLite looks up the `WITHOUT ROWID` primary b-tree (rows carry
-- the ~5 KB `json` inline) for every entry it scans, matching or not; with
-- `resp_status` selecting only 5 % of POSTs that was ~10 us per entry, 44 ms
-- at 300k rows. Covered, the same query takes 0.3 ms. Idempotent for `init`
-- and `migrate` alike.

DROP INDEX IF EXISTS idx_exchanges_method_status;
CREATE INDEX IF NOT EXISTS idx_exchanges_method_status
    ON exchanges (req_method, ulid, resp_status, created_at, req_host, req_path,
                  resp_len, state, tags);

-- The size query (`resp_len >= ?`, selecting ~1 % of rows) has the same
-- range-versus-order mismatch: its matches cannot come out in `ulid` order, so
-- they are collected and sorted, and the uncovered v3 index paid a primary-tree
-- lookup per match (34 to 49 ms at a million rows, too close to the 50 ms
-- target). Covering the list columns makes that a pure index range scan.
DROP INDEX IF EXISTS idx_exchanges_resp_len;
CREATE INDEX IF NOT EXISTS idx_exchanges_resp_len
    ON exchanges (resp_len, ulid, created_at, req_method, req_host, req_path,
                  resp_status, state, tags);
