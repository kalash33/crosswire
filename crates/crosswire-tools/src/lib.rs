//! Repeater and edit-engine tools built on the Crosswire client.
//!
//! The [`Repeater`] takes a raw HTTP/1.1 request and sends it, either
//! untouched ([`Mode::Raw`]) or with its line endings and `Content-Length`
//! fixed ([`Mode::Assisted`]). It checks the request against a [scope] before
//! dialling, follows `3xx` redirects per a [`FollowRedirects`] policy, and
//! records every exchange through an [`ExchangeSink`] using the
//! [`record::record_exchange`] recipe. It never depends on the store: the
//! caller supplies the sink and an audit hook.
//!
//! [scope]: crosswire_scope
//! [`ExchangeSink`]: crosswire_model::ExchangeSink

mod copy;
pub mod record;
mod redirect;
mod repeater;

pub use copy::{body_is_blob, quote, to_curl, to_raw, Shell};
pub use record::{record_exchange, RecordError, Recorded};
pub use redirect::Url;
pub use repeater::{FollowRedirects, Mode, Repeater, SendError, SendOutcome, SendRequest};
