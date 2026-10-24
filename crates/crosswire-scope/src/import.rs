//! Parsing a pasted program-scope list into a [`Scope`].

use std::collections::BTreeSet;
use std::net::IpAddr;

use ipnet::IpNet;

use crate::input::Scheme;
use crate::rule::{HostPattern, PathMatch, Rule};
use crate::scope::Scope;

/// An error from [`parse_program_list`]: the 1-based line and a reason.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// A line could not be parsed.
    #[error("line {line}: {reason}")]
    Line {
        /// The 1-based line number.
        line: usize,
        /// Why the line was rejected.
        reason: String,
    },
}

/// Parses a pasted program-scope list into a [`Scope`].
///
/// Every rule becomes an include rule (the list form has no exclude syntax).
/// Blank lines and lines beginning with `#` are ignored. Each remaining line
/// is one of:
///
/// | Form                         | Example                         | Rule                       |
/// |------------------------------|---------------------------------|----------------------------|
/// | bare host                    | `example.com`                   | exact host                 |
/// | wildcard                     | `*.example.com`                 | suffix (sub-domains only)  |
/// | full URL                     | `https://api.example.com:8443/v1` | exact host + scheme/port/path prefix |
/// | CIDR block                   | `10.0.0.0/8`                    | CIDR                       |
/// | IP literal (v4 / bracketed v6) | `1.2.3.4`, `[::1]`            | IP                         |
pub fn parse_program_list(text: &str) -> Result<Scope, ImportError> {
    let mut include = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let rule = parse_line(line).map_err(|reason| ImportError::Line {
            line: idx + 1,
            reason,
        })?;
        include.push(rule);
    }
    Ok(Scope {
        include,
        exclude: Vec::new(),
    })
}

fn parse_line(line: &str) -> Result<Rule, String> {
    if line.contains("://") {
        return parse_url(line);
    }
    if line.contains('/') {
        let net: IpNet = line
            .parse()
            .map_err(|_| format!("invalid CIDR block: {line:?}"))?;
        return Ok(bare(HostPattern::Cidr(net)));
    }
    if let Some(rest) = line.strip_prefix("*.") {
        if rest.is_empty() || !is_hostish(rest) {
            return Err(format!("invalid wildcard: {line:?}"));
        }
        return Ok(bare(HostPattern::Suffix {
            suffix: rest.to_ascii_lowercase(),
            also_apex: false,
        }));
    }
    let unbracketed = unbracket(line);
    if let Ok(ip) = unbracketed.parse::<IpAddr>() {
        return Ok(bare(HostPattern::Ip(ip)));
    }
    if !is_hostish(line) {
        return Err(format!("invalid host: {line:?}"));
    }
    Ok(bare(HostPattern::Exact(line.to_ascii_lowercase())))
}

fn parse_url(line: &str) -> Result<Rule, String> {
    let url = url::Url::parse(line).map_err(|e| format!("invalid URL: {e}"))?;
    let scheme = match url.scheme() {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        other => return Err(format!("unsupported scheme: {other:?}")),
    };
    let host = url
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    let host = if let Ok(ip) = unbracket(host).parse::<IpAddr>() {
        HostPattern::Ip(ip)
    } else {
        HostPattern::Exact(host.to_ascii_lowercase())
    };
    let ports = url.port().map(|p| BTreeSet::from([p]));
    let path = match url.path() {
        "" | "/" => None,
        p => Some(PathMatch::Prefix(p.to_string())),
    };
    Ok(Rule {
        host,
        ports,
        scheme: Some(scheme),
        path,
    })
}

fn bare(host: HostPattern) -> Rule {
    Rule {
        host,
        ports: None,
        scheme: None,
        path: None,
    }
}

fn unbracket(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host)
}

fn is_hostish(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}
