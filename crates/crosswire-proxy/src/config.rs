//! Proxy configuration and its validation.

use std::net::SocketAddr;

use crate::{ProxyError, ProxyLimits};

/// Credentials a client must present with `Proxy-Authorization: Basic`.
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyAuth {
    user: String,
    pass: String,
}

impl ProxyAuth {
    /// Creates credentials.
    ///
    /// # Errors
    /// [`ProxyError::Config`] if the user or password is empty.
    pub fn new(user: impl Into<String>, pass: impl Into<String>) -> Result<ProxyAuth, ProxyError> {
        let (user, pass) = (user.into(), pass.into());
        if user.is_empty() || pass.is_empty() {
            return Err(ProxyError::Config(
                "proxy auth user and password must be non-empty".into(),
            ));
        }
        Ok(ProxyAuth { user, pass })
    }

    /// The user name.
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    /// The password.
    #[must_use]
    pub fn pass(&self) -> &str {
        &self.pass
    }
}

impl std::fmt::Debug for ProxyAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProxyAuth")
            .field("user", &self.user)
            .field("pass", &"<redacted>")
            .finish()
    }
}

/// How the proxy listens and what it does with hosts.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ProxyConfig {
    /// Address to listen on.
    pub bind: SocketAddr,
    /// Permit a non-loopback `bind` (which then requires `proxy_auth`).
    pub allow_non_loopback: bool,
    /// Required proxy credentials, if any.
    pub proxy_auth: Option<ProxyAuth>,
    /// Hosts (or `.suffix` rules) tunnelled without interception.
    pub passthrough: Vec<String>,
    /// Hosts whose upstream certificate errors are accepted (audited).
    pub accept_invalid_cert: Vec<String>,
    /// Resource limits.
    pub limits: ProxyLimits,
}

impl ProxyConfig {
    /// Creates a configuration for `bind` with everything else default.
    #[must_use]
    pub fn new(bind: SocketAddr) -> ProxyConfig {
        ProxyConfig {
            bind,
            allow_non_loopback: false,
            proxy_auth: None,
            passthrough: Vec::new(),
            accept_invalid_cert: Vec::new(),
            limits: ProxyLimits::default(),
        }
    }

    /// Checks the configuration.
    ///
    /// # Errors
    /// [`ProxyError::NonLoopbackBind`] for a non-loopback bind without
    /// `allow_non_loopback`; [`ProxyError::AuthRequired`] for one without
    /// `proxy_auth`; [`ProxyError::Config`] for a passthrough or
    /// accept-invalid-cert entry that is not a hostname or `.suffix`.
    pub fn validate(&self) -> Result<(), ProxyError> {
        if !self.bind.ip().is_loopback() {
            if !self.allow_non_loopback {
                return Err(ProxyError::NonLoopbackBind);
            }
            if self.proxy_auth.is_none() {
                return Err(ProxyError::AuthRequired);
            }
        }
        for entry in &self.passthrough {
            if !valid_rule(entry) {
                return Err(ProxyError::Config(format!(
                    "passthrough entry {:?} is not a hostname or .suffix",
                    crate::generated::cap(entry)
                )));
            }
        }
        for entry in &self.accept_invalid_cert {
            if !valid_hostname(entry) {
                return Err(ProxyError::Config(format!(
                    "accept-invalid-cert entry {:?} is not a hostname",
                    crate::generated::cap(entry)
                )));
            }
        }
        Ok(())
    }
}

fn valid_rule(s: &str) -> bool {
    valid_hostname(s.strip_prefix('.').unwrap_or(s))
}

/// RFC 1123 hostname: labels of 1..=63 ASCII alphanumerics or `-`, not
/// starting or ending with `-`, at most 253 bytes in all.
fn valid_hostname(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 253
        && s.split('.').all(|l| {
            !l.is_empty()
                && l.len() <= 63
                && !l.starts_with('-')
                && !l.ends_with('-')
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}
