//! The request identity being tested against a scope.

use serde::{Deserialize, Serialize};

/// The URL scheme of a request.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Scheme {
    /// Plain HTTP.
    Http,
    /// HTTP over TLS.
    Https,
}

/// A single request identity: the host, port, scheme and path to test.
///
/// `host` is matched ASCII-case-insensitively; a single trailing dot is
/// stripped and a bracketed IPv6 literal (`[::1]`) is unbracketed before
/// matching. An IP literal matches only [`Ip`](crate::HostPattern::Ip) and
/// [`Cidr`](crate::HostPattern::Cidr) rules, never a host-pattern rule.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Input<'a> {
    /// The host: a name or an IP literal (optionally bracketed for IPv6).
    pub host: &'a str,
    /// The TCP port.
    pub port: u16,
    /// The scheme.
    pub scheme: Scheme,
    /// The request path, used by [`PathMatch`](crate::PathMatch) rules.
    pub path: &'a str,
}
