//! Scope rules and the error produced when compiling them.

use std::collections::BTreeSet;
use std::net::IpAddr;

use ipnet::IpNet;
use serde::{Deserialize, Serialize};

use crate::input::Scheme;

/// How a rule matches the host of a request.
///
/// The variants are tagged for serde with a lowercase `type` field and a
/// `value` payload, e.g. `{"type":"exact","value":"example.com"}`. Struct
/// fields use snake_case, so [`Suffix`](HostPattern::Suffix) serializes as
/// `{"type":"suffix","value":{"suffix":"example.com","also_apex":true}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum HostPattern {
    /// Matches one host name exactly (ASCII case-insensitive).
    Exact(String),
    /// Matches a domain suffix: every sub-domain of `suffix`, and the apex
    /// domain itself when `also_apex` is set.
    Suffix {
        /// The apex domain, e.g. `example.com` for `*.example.com`.
        suffix: String,
        /// Whether the apex domain matches as well as its sub-domains.
        also_apex: bool,
    },
    /// Matches a host name against a regular expression.
    Regex(String),
    /// Matches one IP literal exactly. Never matches a host name.
    Ip(IpAddr),
    /// Matches any IP literal inside a CIDR block. Never matches a host name.
    Cidr(IpNet),
}

/// How a rule matches the path of a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum PathMatch {
    /// Matches when the request path starts with this prefix.
    Prefix(String),
    /// Matches the request path against a regular expression.
    Regex(String),
}

/// One scope rule.
///
/// A rule matches a request when its [`host`](Rule::host) pattern matches and
/// every set constraint also matches; an unset (`None`) constraint matches
/// anything.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    /// The host pattern.
    pub host: HostPattern,
    /// The set of allowed ports; `None` matches any port.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ports: Option<BTreeSet<u16>>,
    /// The required scheme; `None` matches any scheme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<Scheme>,
    /// The path constraint; `None` matches any path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathMatch>,
}

/// An error produced while compiling a [`Scope`](crate::Scope) into a
/// [`Compiled`](crate::Compiled) matcher.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScopeError {
    /// A host or path regular expression failed to parse.
    #[error("invalid regex: {0}")]
    Regex(String),
    /// A regular expression compiled to more than the 10 MiB size limit.
    #[error("regex exceeds the compiled-size limit")]
    RegexTooLarge,
    /// A CIDR block could not be parsed.
    #[error("invalid CIDR block")]
    BadCidr,
    /// A port value could not be parsed.
    #[error("invalid port")]
    BadPort,
}
