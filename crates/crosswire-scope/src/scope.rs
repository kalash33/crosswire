//! The [`Scope`] type, its [`Compiled`] form, and evaluation.

use std::collections::BTreeSet;
use std::net::IpAddr;

use ipnet::IpNet;
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use crate::input::{Input, Scheme};
use crate::rule::{HostPattern, PathMatch, Rule, ScopeError};

/// The compiled-size limit for a single regex (spec §4.7).
const REGEX_SIZE_LIMIT: usize = 10 * 1024 * 1024;

/// A project's testing scope: ordered include rules and exclude rules.
///
/// Precedence (spec §4.7): include rules are tried first, then exclude rules.
/// A matching exclude wins over any include. With no include match a request
/// is [`Out`](Verdict::Out); with no rules at all the scope is
/// [`Empty`](Verdict::Empty).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// Rules that bring a request into scope.
    #[serde(default)]
    pub include: Vec<Rule>,
    /// Rules that take a request back out of scope; a matching exclude wins
    /// over any matching include.
    #[serde(default)]
    pub exclude: Vec<Rule>,
}

impl Scope {
    /// Returns `true` when the scope has no rules at all.
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    /// Compiles the scope, building every host and path regex exactly once
    /// (each with a 10 MiB compiled-size limit).
    pub fn compile(&self) -> Result<Compiled, ScopeError> {
        Ok(Compiled {
            empty: self.is_empty(),
            include: self
                .include
                .iter()
                .map(CompiledRule::new)
                .collect::<Result<_, _>>()?,
            exclude: self
                .exclude
                .iter()
                .map(CompiledRule::new)
                .collect::<Result<_, _>>()?,
        })
    }
}

/// The verdict for a single request identity.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// In scope.
    In,
    /// Out of scope.
    Out,
    /// No scope is configured.
    Empty,
}

/// The verdict for a connection/message identity pair.
///
/// This is crosswire-scope's own verdict type; the proxy and tools crates map
/// it to the model's `ScopeState`, which keeps crosswire-scope dependent on
/// `crosswire-core` only.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PairVerdict {
    /// Both identities are in scope.
    In,
    /// Both identities are out of scope.
    Out,
    /// No scope is configured.
    Empty,
    /// The two identities disagree (for example, the Host header differs from
    /// the connection target).
    HostMismatch,
}

/// A compiled [`Scope`]: regexes are built once, ready for fast evaluation.
#[derive(Clone, Debug)]
pub struct Compiled {
    empty: bool,
    include: Vec<CompiledRule>,
    exclude: Vec<CompiledRule>,
}

impl Compiled {
    /// Evaluates a single request identity against the scope.
    pub fn evaluate(&self, input: &Input) -> Verdict {
        if self.empty {
            return Verdict::Empty;
        }
        let norm = Normalized::new(input);
        if !self.include.iter().any(|r| r.matches(&norm)) {
            return Verdict::Out;
        }
        if self.exclude.iter().any(|r| r.matches(&norm)) {
            return Verdict::Out;
        }
        Verdict::In
    }

    /// Evaluates a connection target and a message (Host header) identity
    /// together, returning [`PairVerdict::HostMismatch`] when the two single
    /// verdicts disagree.
    pub fn evaluate_pair(&self, connection: &Input, message: &Input) -> PairVerdict {
        match (self.evaluate(connection), self.evaluate(message)) {
            (Verdict::Empty, _) | (_, Verdict::Empty) => PairVerdict::Empty,
            (Verdict::In, Verdict::In) => PairVerdict::In,
            (Verdict::Out, Verdict::Out) => PairVerdict::Out,
            _ => PairVerdict::HostMismatch,
        }
    }
}

/// A request identity with its host already normalized for matching.
struct Normalized<'a> {
    host: String,
    ip: Option<IpAddr>,
    port: u16,
    scheme: Scheme,
    path: &'a str,
}

impl<'a> Normalized<'a> {
    fn new(input: &Input<'a>) -> Self {
        let mut host = input.host;
        if let Some(inner) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            host = inner;
        }
        let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
        let ip = host.parse::<IpAddr>().ok();
        Normalized {
            host,
            ip,
            port: input.port,
            scheme: input.scheme,
            path: input.path,
        }
    }
}

#[derive(Clone, Debug)]
struct CompiledRule {
    host: CompiledHost,
    ports: Option<BTreeSet<u16>>,
    scheme: Option<Scheme>,
    path: Option<CompiledPath>,
}

impl CompiledRule {
    fn new(rule: &Rule) -> Result<Self, ScopeError> {
        let host = match &rule.host {
            HostPattern::Exact(h) => CompiledHost::Exact(h.to_ascii_lowercase()),
            HostPattern::Suffix { suffix, also_apex } => {
                let apex = suffix.to_ascii_lowercase();
                CompiledHost::Suffix {
                    dot_suffix: format!(".{apex}"),
                    apex,
                    also_apex: *also_apex,
                }
            }
            HostPattern::Regex(p) => CompiledHost::Regex(build_regex(p)?),
            HostPattern::Ip(a) => CompiledHost::Ip(*a),
            HostPattern::Cidr(n) => CompiledHost::Cidr(*n),
        };
        let path = match &rule.path {
            None => None,
            Some(PathMatch::Prefix(p)) => Some(CompiledPath::Prefix(p.clone())),
            Some(PathMatch::Regex(p)) => Some(CompiledPath::Regex(build_regex(p)?)),
        };
        Ok(CompiledRule {
            host,
            ports: rule.ports.clone(),
            scheme: rule.scheme,
            path,
        })
    }

    fn matches(&self, input: &Normalized) -> bool {
        self.host.matches(input)
            && self
                .ports
                .as_ref()
                .is_none_or(|set| set.contains(&input.port))
            && self.scheme.is_none_or(|s| s == input.scheme)
            && self.path.as_ref().is_none_or(|p| p.matches(input.path))
    }
}

#[derive(Clone, Debug)]
enum CompiledHost {
    Exact(String),
    Suffix {
        apex: String,
        dot_suffix: String,
        also_apex: bool,
    },
    Regex(Regex),
    Ip(IpAddr),
    Cidr(IpNet),
}

impl CompiledHost {
    fn matches(&self, input: &Normalized) -> bool {
        match self {
            CompiledHost::Exact(h) => input.ip.is_none() && input.host == *h,
            CompiledHost::Suffix {
                apex,
                dot_suffix,
                also_apex,
            } => {
                input.ip.is_none()
                    && ((*also_apex && input.host == *apex) || input.host.ends_with(dot_suffix))
            }
            CompiledHost::Regex(re) => input.ip.is_none() && re.is_match(&input.host),
            CompiledHost::Ip(a) => input.ip == Some(*a),
            CompiledHost::Cidr(n) => input.ip.is_some_and(|ip| n.contains(&ip)),
        }
    }
}

#[derive(Clone, Debug)]
enum CompiledPath {
    Prefix(String),
    Regex(Regex),
}

impl CompiledPath {
    fn matches(&self, path: &str) -> bool {
        match self {
            CompiledPath::Prefix(p) => path.starts_with(p.as_str()),
            CompiledPath::Regex(re) => re.is_match(path),
        }
    }
}

fn build_regex(pattern: &str) -> Result<Regex, ScopeError> {
    match RegexBuilder::new(pattern)
        .size_limit(REGEX_SIZE_LIMIT)
        .build()
    {
        Ok(re) => Ok(re),
        Err(regex::Error::CompiledTooBig(_)) => Err(ScopeError::RegexTooLarge),
        Err(e) => Err(ScopeError::Regex(e.to_string())),
    }
}
