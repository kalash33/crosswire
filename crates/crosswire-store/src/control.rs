//! The control path: a second write connection for the small, rare writes the
//! sidecar backend makes outside the exchange stream.
//!
//! [`Store::control`](crate::Store::control) hands out a [`Control`] with its
//! own write connection (hardened like the writer's, 5 s busy timeout). It
//! never touches exchange bodies or packs; it reads and writes `settings`,
//! `tokens` and `drafts`, and patches the `tags`/`notes` of an existing
//! exchange. The writer thread keeps its own connection, and the 5 s busy
//! timeout lets a control write wait for a batch commit instead of failing.

use crate::error::StoreError;
use crate::store::Store;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};
use ulid::Ulid;

/// `last_used_at` is refreshed at most this often (milliseconds).
const LAST_USED_RESOLUTION_MS: u64 = 60_000;

/// A capability an API token grants. Stored as the kebab-case wire name.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum TokenScope {
    /// Read recorded exchanges and project state.
    Read,
    /// Send and replay requests (repeater, intruder).
    Send,
    /// Administer the project (tokens, settings, teardown).
    Admin,
}

/// An API token's stored metadata. Never carries the secret or its hash: the
/// secret is shown once at creation and only its SHA-256 is stored.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TokenRecord {
    /// The token id.
    pub id: Ulid,
    /// A human label.
    pub name: String,
    /// The capabilities granted.
    pub scopes: Vec<TokenScope>,
    /// Creation time, unix milliseconds.
    pub created_at: u64,
    /// When the token was revoked, if it was (unix milliseconds).
    pub revoked_at: Option<u64>,
    /// When the token was last used, if ever (unix milliseconds, coarse).
    pub last_used_at: Option<u64>,
}

/// A saved repeater draft (one per tab).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Draft {
    /// The repeater tab id.
    pub tab_id: Ulid,
    /// The draft payload (opaque JSON chosen by the UI).
    pub json: String,
    /// Last save time, unix milliseconds.
    pub updated_at: u64,
}

/// The store's control handle: a dedicated write connection for settings, API
/// tokens, repeater drafts, and an exchange's tags and notes.
///
/// Get one with [`Store::control`](crate::Store::control). It is cheap but not
/// free (it opens a connection), so hold one rather than opening per call. It
/// is `Send` but not `Sync`; use one per thread.
#[derive(Debug)]
pub struct Control {
    conn: Connection,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn as_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn opt_u64(v: Option<i64>) -> Option<u64> {
    v.map(|n| u64::try_from(n).unwrap_or(0))
}

impl Control {
    pub(crate) fn open(dir: &std::path::Path) -> Result<Control, StoreError> {
        Ok(Control {
            conn: crate::project::open_control(dir)?,
        })
    }

    // -- settings ----------------------------------------------------------

    /// The value of a setting, or `None` if it is not set.
    pub fn setting_get(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    /// Sets (or overwrites) a setting.
    pub fn setting_put(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, \
             updated_at = excluded.updated_at",
            rusqlite::params![key, value, as_i64(now_ms())],
        )?;
        Ok(())
    }

    // -- tokens ------------------------------------------------------------

    /// Mints a new API token.
    ///
    /// Returns its stored [`TokenRecord`] and the secret, shown once and never
    /// recoverable: the secret is `cwt_` followed by 32 random bytes in
    /// base64url (no padding), and only `SHA-256` of those 32 bytes is stored.
    pub fn token_create(
        &self,
        name: &str,
        scopes: &[TokenScope],
    ) -> Result<(TokenRecord, String), StoreError> {
        let raw: [u8; 32] = rand::random();
        let secret = format!("cwt_{}", URL_SAFE_NO_PAD.encode(raw));
        let hash: [u8; 32] = Sha256::digest(raw).into();

        let id = Ulid::new();
        let created_at = now_ms();
        let scopes = scopes.to_vec();
        let scopes_json = serde_json::to_string(&scopes)?;
        self.conn.execute(
            "INSERT INTO tokens (id, name, scopes, hash, created_at, revoked_at, last_used_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL)",
            rusqlite::params![
                id.to_string(),
                name,
                scopes_json,
                hash.as_slice(),
                as_i64(created_at),
            ],
        )?;
        let record = TokenRecord {
            id,
            name: name.to_owned(),
            scopes,
            created_at,
            revoked_at: None,
            last_used_at: None,
        };
        Ok((record, secret))
    }

    /// Lists all tokens (active and revoked), newest first. Never includes the
    /// secret or the hash.
    pub fn token_list(&self) -> Result<Vec<TokenRecord>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, scopes, created_at, revoked_at, last_used_at \
             FROM tokens ORDER BY id DESC",
        )?;
        let rows = stmt.query_map([], token_record)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row??);
        }
        Ok(out)
    }

    /// Revokes a token. Returns `true` if an active token was revoked, `false`
    /// if it was unknown or already revoked.
    pub fn token_revoke(&self, id: Ulid) -> Result<bool, StoreError> {
        let n = self.conn.execute(
            "UPDATE tokens SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
            rusqlite::params![id.to_string(), as_i64(now_ms())],
        )?;
        Ok(n > 0)
    }

    /// Looks up an active (not revoked) token by the `SHA-256` of its 32 secret
    /// bytes, and refreshes `last_used_at` at most once per minute.
    ///
    /// The caller is responsible for the constant-time comparison of the
    /// presented secret's hash; this is a plain indexed lookup by the stored
    /// `hash`, which is already a one-way digest.
    pub fn token_lookup(&self, hash: &[u8; 32]) -> Result<Option<TokenRecord>, StoreError> {
        let mut record = match self
            .conn
            .query_row(
                "SELECT id, name, scopes, created_at, revoked_at, last_used_at \
                 FROM tokens WHERE hash = ?1 AND revoked_at IS NULL",
                [hash.as_slice()],
                token_record,
            )
            .optional()?
        {
            Some(r) => r?,
            None => return Ok(None),
        };
        let now = now_ms();
        let stale = record
            .last_used_at
            .is_none_or(|prev| now.saturating_sub(prev) >= LAST_USED_RESOLUTION_MS);
        if stale {
            self.conn.execute(
                "UPDATE tokens SET last_used_at = ?2 WHERE id = ?1",
                rusqlite::params![record.id.to_string(), as_i64(now)],
            )?;
            record.last_used_at = Some(now);
        }
        Ok(Some(record))
    }

    // -- drafts ------------------------------------------------------------

    /// Saves (or overwrites) a tab's repeater draft.
    pub fn draft_put(&self, tab_id: Ulid, json: &str) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO drafts (tab_id, json, updated_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(tab_id) DO UPDATE SET json = excluded.json, \
             updated_at = excluded.updated_at",
            rusqlite::params![tab_id.to_string(), json, as_i64(now_ms())],
        )?;
        Ok(())
    }

    /// A tab's saved draft payload, or `None`.
    pub fn draft_get(&self, tab_id: Ulid) -> Result<Option<String>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT json FROM drafts WHERE tab_id = ?1",
                [tab_id.to_string()],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Lists all saved drafts, newest save first.
    pub fn draft_list(&self) -> Result<Vec<Draft>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT tab_id, json, updated_at FROM drafts ORDER BY updated_at DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (tab_id, json, updated_at) = row?;
            out.push(Draft {
                tab_id: parse_ulid(&tab_id)?,
                json,
                updated_at: u64::try_from(updated_at).unwrap_or(0),
            });
        }
        Ok(out)
    }

    /// Deletes a tab's draft. Returns `true` if one was removed.
    pub fn draft_delete(&self, tab_id: Ulid) -> Result<bool, StoreError> {
        let n = self
            .conn
            .execute("DELETE FROM drafts WHERE tab_id = ?1", [tab_id.to_string()])?;
        Ok(n > 0)
    }

    // -- tags and notes ----------------------------------------------------

    /// Replaces an exchange's tags. Returns `false` if there is no such
    /// exchange.
    ///
    /// Updates both the denormalised `tags` column (so the list filter sees it)
    /// and the `tags` array inside the stored JSON (so [`Store::get`] agrees),
    /// in one transaction.
    pub fn exchange_set_tags(&self, id: Ulid, tags: &[String]) -> Result<bool, StoreError> {
        let column = if tags.is_empty() {
            String::new()
        } else {
            serde_json::to_string(tags)?
        };
        self.patch_exchange(id, "tags", serde_json::to_value(tags)?, &column)
    }

    /// Replaces an exchange's notes. Returns `false` if there is no such
    /// exchange.
    ///
    /// Updates both the `notes` column and the `notes` field in the stored
    /// JSON, in one transaction, so a later [`Store::get`] is consistent.
    pub fn exchange_set_notes(&self, id: Ulid, notes: &str) -> Result<bool, StoreError> {
        self.patch_exchange(
            id,
            "notes",
            serde_json::Value::String(notes.to_owned()),
            notes,
        )
    }

    /// Patches one top-level JSON field and its denormalised column together.
    fn patch_exchange(
        &self,
        id: Ulid,
        field: &str,
        json_value: serde_json::Value,
        column_value: &str,
    ) -> Result<bool, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let json: Option<String> = tx
            .query_row(
                "SELECT json FROM exchanges WHERE ulid = ?1",
                [id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(json) = json else {
            return Ok(false);
        };
        let mut value: serde_json::Value = serde_json::from_str(&json)?;
        let obj = value
            .as_object_mut()
            .ok_or_else(|| StoreError::Corrupt("exchange json is not an object".into()))?;
        obj.insert(field.to_owned(), json_value);
        let patched = serde_json::to_string(&value)?;
        tx.execute(
            &format!("UPDATE exchanges SET {field} = ?2, json = ?3 WHERE ulid = ?1"),
            rusqlite::params![id.to_string(), column_value, patched],
        )?;
        tx.commit()?;
        Ok(true)
    }
}

impl Store {
    /// A control handle: a dedicated write connection for the sidecar's small,
    /// rare writes (settings, tokens, drafts, and an exchange's tags/notes).
    ///
    /// Opening it can fail (it opens a connection), so this returns a `Result`
    /// rather than the bare handle. Hold the handle instead of reopening per
    /// call.
    pub fn control(&self) -> Result<Control, StoreError> {
        Control::open(self.dir())
    }
}

fn parse_ulid(s: &str) -> Result<Ulid, StoreError> {
    Ulid::from_string(s).map_err(|_| StoreError::Corrupt("stored id is not a ULID".into()))
}

/// Builds a [`TokenRecord`] from a row of (id, name, scopes, created_at,
/// revoked_at, last_used_at). Returns an inner `Result` so SQL errors and
/// decode errors stay distinct.
fn token_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<Result<TokenRecord, StoreError>> {
    let id: String = r.get(0)?;
    let name: String = r.get(1)?;
    let scopes: String = r.get(2)?;
    let created_at: i64 = r.get(3)?;
    let revoked_at: Option<i64> = r.get(4)?;
    let last_used_at: Option<i64> = r.get(5)?;
    Ok((|| {
        Ok(TokenRecord {
            id: parse_ulid(&id)?,
            name,
            scopes: serde_json::from_str(&scopes)?,
            created_at: u64::try_from(created_at).unwrap_or(0),
            revoked_at: opt_u64(revoked_at),
            last_used_at: opt_u64(last_used_at),
        })
    })())
}
