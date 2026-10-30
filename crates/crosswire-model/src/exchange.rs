//! The exchange row: a request, its response and recording metadata.

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use crate::Message;

/// Phase timings in milliseconds.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Timing {
    /// TCP connect time.
    pub connect_ms: u32,
    /// TLS handshake time.
    pub tls_ms: u32,
    /// Time to first response byte.
    pub ttfb_ms: u32,
    /// Total time.
    pub total_ms: u32,
}

/// Whether the exchange matched the project scope.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ScopeState {
    /// In scope.
    In,
    /// Out of scope.
    Out,
    /// No scope is configured.
    #[default]
    Empty,
    /// The Host header differs from the connection target.
    HostMismatch,
    /// A scope state written by a newer version of Crosswire.
    #[serde(other)]
    Unknown,
}

/// Which table an exchange belongs to.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum NamespaceKind {
    /// Proxy history.
    #[default]
    History,
    /// A repeater tab's send history.
    Repeater,
    /// An intruder attack's results.
    Intruder,
    /// A namespace written by a newer version of Crosswire.
    #[serde(other)]
    Unknown,
}

/// The namespace an exchange lives in.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Namespace {
    kind: NamespaceKind,
    #[serde(with = "crate::ulid_serde::option")]
    owner: Option<Ulid>,
}

impl Namespace {
    /// Creates a namespace; `owner` is the repeater tab or attack id.
    #[must_use]
    pub fn new(kind: NamespaceKind, owner: Option<Ulid>) -> Namespace {
        Namespace { kind, owner }
    }

    /// The namespace kind.
    #[must_use]
    pub fn kind(&self) -> NamespaceKind {
        self.kind
    }

    /// The owning tab or attack, if any.
    #[must_use]
    pub fn owner(&self) -> Option<Ulid> {
        self.owner
    }
}

/// Recording state of an exchange.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ExchangeState {
    /// Queued, not yet durable.
    #[default]
    Received,
    /// Durably stored.
    Committed,
    /// Recording failed.
    Error,
    /// A state written by a newer version of Crosswire.
    #[serde(other)]
    Unknown,
}

/// One request/response pair as recorded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Exchange {
    #[serde(with = "crate::ulid_serde")]
    id: Ulid,
    request: Message,
    #[serde(default)]
    response: Option<Message>,
    #[serde(default)]
    timing: Timing,
    #[serde(default)]
    scope_state: ScopeState,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    namespace: Namespace,
    #[serde(default)]
    state: ExchangeState,
}

impl Exchange {
    /// Creates an exchange with a fresh ULID in state `Received`, namespace
    /// `History` and scope `Empty`.
    #[cfg(feature = "std")]
    #[must_use]
    pub fn new(request: Message) -> Exchange {
        Exchange::with_id(Ulid::new(), request)
    }

    /// As [`Exchange::new`] with a caller-supplied id (for `no_std` callers,
    /// replays and tests).
    #[must_use]
    pub fn with_id(id: Ulid, request: Message) -> Exchange {
        Exchange {
            id,
            request,
            response: None,
            timing: Timing::default(),
            scope_state: ScopeState::default(),
            tags: Vec::new(),
            notes: String::new(),
            namespace: Namespace::default(),
            state: ExchangeState::default(),
        }
    }

    /// The exchange id.
    #[must_use]
    pub fn id(&self) -> Ulid {
        self.id
    }

    /// The request.
    #[must_use]
    pub fn request(&self) -> &Message {
        &self.request
    }

    /// The response, once received.
    #[must_use]
    pub fn response(&self) -> Option<&Message> {
        self.response.as_ref()
    }

    /// Phase timings.
    #[must_use]
    pub fn timing(&self) -> Timing {
        self.timing
    }

    /// Scope classification.
    #[must_use]
    pub fn scope_state(&self) -> ScopeState {
        self.scope_state
    }

    /// User tags.
    #[must_use]
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// User notes.
    #[must_use]
    pub fn notes(&self) -> &str {
        &self.notes
    }

    /// The namespace.
    #[must_use]
    pub fn namespace(&self) -> Namespace {
        self.namespace
    }

    /// The recording state.
    #[must_use]
    pub fn state(&self) -> ExchangeState {
        self.state
    }

    /// Attaches the response.
    pub fn set_response(&mut self, response: Message) {
        self.response = Some(response);
    }

    /// Sets the timings.
    pub fn set_timing(&mut self, timing: Timing) {
        self.timing = timing;
    }

    /// Sets the scope classification.
    pub fn set_scope_state(&mut self, scope_state: ScopeState) {
        self.scope_state = scope_state;
    }

    /// Adds a tag.
    pub fn add_tag(&mut self, tag: impl Into<String>) {
        self.tags.push(tag.into());
    }

    /// Replaces the notes.
    pub fn set_notes(&mut self, notes: impl Into<String>) {
        self.notes = notes.into();
    }

    /// Sets the namespace.
    pub fn set_namespace(&mut self, namespace: Namespace) {
        self.namespace = namespace;
    }

    /// Sets the recording state.
    pub fn set_state(&mut self, state: ExchangeState) {
        self.state = state;
    }
}
