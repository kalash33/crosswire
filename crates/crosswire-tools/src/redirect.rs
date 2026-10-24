//! Redirect-target resolution.
//!
//! [`Url`] is the little the repeater needs to chase a `Location` header:
//! scheme (as a TLS flag), host, port and path. [`Url::resolve`] applies a
//! `Location` value to the current URL, handling absolute, scheme-relative,
//! absolute-path and relative references (RFC 3986 §5, enough for redirects).

/// A resolved request URL: just the parts the repeater dials and matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    /// Whether the scheme is `https`.
    pub tls: bool,
    /// The host (no brackets, no port).
    pub host: String,
    /// The port, defaulted from the scheme when the URL omits it.
    pub port: u16,
    /// The path (and query), always starting with `/`.
    pub path: String,
}

impl Url {
    /// Builds a URL, defaulting an empty path to `/`.
    #[must_use]
    pub fn new(tls: bool, host: impl Into<String>, port: u16, path: impl Into<String>) -> Url {
        let path = path.into();
        Url {
            tls,
            host: host.into(),
            port,
            path: if path.is_empty() { "/".into() } else { path },
        }
    }

    /// Resolves a `Location` value against this URL. Returns `None` when the
    /// reference is empty or an absolute URL with an unsupported scheme.
    #[must_use]
    pub fn resolve(&self, location: &str) -> Option<Url> {
        let loc = location.trim();
        if loc.is_empty() {
            return None;
        }
        if let Some(rest) = strip_scheme(loc, "https://") {
            return Some(parse_authority_path(true, rest));
        }
        if let Some(rest) = strip_scheme(loc, "http://") {
            return Some(parse_authority_path(false, rest));
        }
        // An absolute URL with any other scheme is not followable here.
        if has_scheme(loc) {
            return None;
        }
        if let Some(rest) = loc.strip_prefix("//") {
            // Scheme-relative: keep our scheme.
            return Some(parse_authority_path(self.tls, rest));
        }
        if loc.starts_with('/') {
            return Some(Url::new(self.tls, self.host.clone(), self.port, loc));
        }
        // Relative reference: resolve against our path's directory.
        let dir = match self.path.rfind('/') {
            Some(i) => &self.path[..=i],
            None => "/",
        };
        Some(Url::new(
            self.tls,
            self.host.clone(),
            self.port,
            format!("{dir}{loc}"),
        ))
    }
}

fn strip_scheme<'a>(s: &'a str, scheme: &str) -> Option<&'a str> {
    if s.len() >= scheme.len() && s[..scheme.len()].eq_ignore_ascii_case(scheme) {
        Some(&s[scheme.len()..])
    } else {
        None
    }
}

/// Whether `s` begins with a `scheme:` (an ASCII letter followed by letters,
/// digits, `+`, `-` or `.`, then a colon).
fn has_scheme(s: &str) -> bool {
    let mut chars = s.char_indices();
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for (_, c) in chars {
        if c == ':' {
            return true;
        }
        if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
            return false;
        }
    }
    false
}

fn parse_authority_path(tls: bool, rest: &str) -> Url {
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    // Strip any userinfo.
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = split_host_port(authority, tls);
    Url::new(tls, host, port, path)
}

fn split_host_port(authority: &str, tls: bool) -> (String, u16) {
    let default = if tls { 443 } else { 80 };
    if let Some(rest) = authority.strip_prefix('[') {
        // Bracketed IPv6 literal.
        if let Some(end) = rest.find(']') {
            let host = &rest[..end];
            let port = rest[end + 1..]
                .strip_prefix(':')
                .and_then(|p| p.parse().ok())
                .unwrap_or(default);
            return (host.to_string(), port);
        }
    }
    match authority.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() => {
            (host.to_string(), port.parse().unwrap_or(default))
        }
        _ => (authority.to_string(), default),
    }
}

#[cfg(test)]
mod tests {
    use super::Url;

    fn base() -> Url {
        Url::new(false, "a.test", 80, "/dir/page")
    }

    #[test]
    fn absolute_url() {
        let u = base().resolve("https://b.test:8443/x").unwrap();
        assert_eq!(u, Url::new(true, "b.test", 8443, "/x"));
    }

    #[test]
    fn absolute_path_keeps_host() {
        let u = base().resolve("/next").unwrap();
        assert_eq!(u, Url::new(false, "a.test", 80, "/next"));
    }

    #[test]
    fn relative_resolves_against_dir() {
        let u = base().resolve("other").unwrap();
        assert_eq!(u, Url::new(false, "a.test", 80, "/dir/other"));
    }

    #[test]
    fn scheme_relative_keeps_scheme() {
        let u = base().resolve("//c.test/y").unwrap();
        assert_eq!(u, Url::new(false, "c.test", 80, "/y"));
    }

    #[test]
    fn unsupported_scheme_is_none() {
        assert!(base().resolve("mailto:x@y.test").is_none());
        assert!(base().resolve("").is_none());
    }
}
