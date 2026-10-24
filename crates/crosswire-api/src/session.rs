//! In-memory operator sessions created by a bootstrap exchange.
//!
//! A session is a random 256-bit id delivered in the `cw_sid_<port>` cookie.
//! Sessions live only in memory: they do not survive a restart. Each has an
//! idle expiry; a request that presents a live session refreshes it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use subtle::ConstantTimeEq;

/// One active session: when it was created and when it was last seen.
#[derive(Clone, Copy, Debug)]
struct Session {
    last_seen: Instant,
}

/// A thread-safe store of active sessions, keyed by their secret id.
#[derive(Clone, Debug, Default)]
pub struct SessionStore {
    inner: Arc<Mutex<HashMap<String, Session>>>,
}

impl SessionStore {
    /// A fresh, empty store.
    #[must_use]
    pub fn new() -> SessionStore {
        SessionStore::default()
    }

    /// Mints a new session with a random 256-bit id and returns the id.
    #[must_use]
    pub fn create(&self) -> String {
        let raw: [u8; 32] = rand::random();
        let id = URL_SAFE_NO_PAD.encode(raw);
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id.clone(),
                Session {
                    last_seen: Instant::now(),
                },
            );
        id
    }

    /// Validates the presented id against every stored id with a
    /// constant-time comparison, enforces the idle timeout, and refreshes
    /// the matched session's last-seen time. Returns `true` when a live
    /// session matched.
    ///
    /// An id whose idle window has elapsed is removed and rejected.
    #[must_use]
    pub fn validate_and_refresh(&self, presented: &str, idle_timeout: Duration) -> bool {
        let mut map = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        // Constant-time match against each stored id so a timing side channel
        // cannot reveal which ids exist.
        let mut matched: Option<String> = None;
        let presented = presented.as_bytes();
        for id in map.keys() {
            if bool::from(id.as_bytes().ct_eq(presented)) {
                matched = Some(id.clone());
                break;
            }
        }
        let Some(id) = matched else {
            return false;
        };
        let now = Instant::now();
        let session = map.get_mut(&id).expect("just matched");
        if now.duration_since(session.last_seen) > idle_timeout {
            map.remove(&id);
            return false;
        }
        session.last_seen = now;
        true
    }

    /// The number of active sessions (for tests and diagnostics).
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    /// Whether there are no active sessions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
