//! Command-line definition.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Crosswire: an HTTP/1.1 intercepting proxy that records into a project.
#[derive(Debug, Parser)]
#[command(name = "crosswire", version)]
pub struct Cli {
    /// Log filter (a level such as `info` or `debug`, or an `EnvFilter`
    /// directive). `RUST_LOG` takes precedence when set. Logs go to stderr.
    #[arg(long, global = true, default_value = "info", value_name = "LEVEL")]
    pub log: String,

    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the intercepting proxy, recording into a project.
    Serve(ServeArgs),
    /// Inspect or export the interception CA.
    #[command(subcommand)]
    Ca(CaCommand),
}

/// Arguments of `crosswire serve`.
#[derive(Debug, Args)]
pub struct ServeArgs {
    /// Project directory (created if it does not exist).
    #[arg(long, value_name = "PATH")]
    pub project: PathBuf,

    /// Address the proxy listens on. Port 0 picks a free port.
    #[arg(long, default_value = "127.0.0.1:8080", value_name = "ADDR")]
    pub proxy_bind: SocketAddr,

    /// Permit a non-loopback --proxy-bind (requires --proxy-auth).
    #[arg(long)]
    pub allow_non_loopback: bool,

    /// Require `Proxy-Authorization: Basic` with these credentials.
    #[arg(long, value_name = "USER:PASS", value_parser = parse_user_pass)]
    pub proxy_auth: Option<(String, String)>,

    /// Tunnel this host (or `.suffix`) without interception. Repeatable.
    #[arg(long, value_name = "HOST")]
    pub passthrough: Vec<String>,

    /// Let the proxy reach this loopback or private destination. Repeatable.
    #[arg(long, value_name = "HOST:PORT", value_parser = parse_host_port)]
    pub allow: Vec<(String, u16)>,

    /// Accept an invalid upstream certificate from this host. Repeatable;
    /// each entry is audited.
    #[arg(long, value_name = "HOST")]
    pub accept_invalid_cert: Vec<String>,

    /// CA directory (default: the platform config dir, see `crosswire ca path`).
    #[arg(long, value_name = "PATH")]
    pub ca_dir: Option<PathBuf>,

    /// Bytes of each body kept in the capture; the rest is relayed only.
    #[arg(long, value_name = "BYTES")]
    pub capture_cap: Option<u64>,
}

/// `crosswire ca` subcommands.
#[derive(Debug, Subcommand)]
pub enum CaCommand {
    /// Write the CA certificate (generating the CA if there is none).
    Export {
        /// Output encoding.
        #[arg(long, value_enum)]
        format: CertFormat,
        /// Output file (default: stdout).
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// CA directory (default: see `crosswire ca path`).
        #[arg(long, value_name = "PATH")]
        ca_dir: Option<PathBuf>,
    },
    /// Print the base64 SHA-256 pin of the CA's SubjectPublicKeyInfo.
    Spki {
        /// CA directory (default: see `crosswire ca path`).
        #[arg(long, value_name = "PATH")]
        ca_dir: Option<PathBuf>,
    },
    /// Print the default CA directory.
    Path,
}

/// Certificate encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum CertFormat {
    /// Binary DER.
    Der,
    /// PEM text.
    Pem,
}

/// Splits `USER:PASS` on the first `:`.
fn parse_user_pass(s: &str) -> Result<(String, String), String> {
    match s.split_once(':') {
        Some((u, p)) if !u.is_empty() && !p.is_empty() => Ok((u.to_string(), p.to_string())),
        _ => Err("expected USER:PASS with both parts non-empty".into()),
    }
}

/// Parses `host:port` or `[ipv6]:port`.
fn parse_host_port(s: &str) -> Result<(String, u16), String> {
    let (host, port) = s
        .rsplit_once(':')
        .ok_or_else(|| "expected HOST:PORT".to_string())?;
    let host = match host.strip_prefix('[') {
        Some(rest) => rest
            .strip_suffix(']')
            .filter(|h| h.contains(':'))
            .ok_or_else(|| "malformed bracketed IPv6 host".to_string())?,
        None if host.contains(':') => return Err("an IPv6 host must be bracketed".into()),
        None => host,
    };
    if host.is_empty() {
        return Err("empty host".into());
    }
    let port: u16 = port.parse().map_err(|_| format!("invalid port {port:?}"))?;
    if port == 0 {
        return Err("port must be non-zero".into());
    }
    Ok((host.to_ascii_lowercase(), port))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_forms() {
        assert_eq!(
            parse_host_port("127.0.0.1:8443"),
            Ok(("127.0.0.1".into(), 8443))
        );
        assert_eq!(parse_host_port("Lab.Test:80"), Ok(("lab.test".into(), 80)));
        assert_eq!(parse_host_port("[::1]:9000"), Ok(("::1".into(), 9000)));
        for bad in [
            "host",
            ":80",
            "host:",
            "host:0",
            "host:70000",
            "::1:80",
            "[]:80",
            "[x:1",
        ] {
            assert!(parse_host_port(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn user_pass_splits_on_first_colon() {
        assert_eq!(parse_user_pass("u:p:q"), Ok(("u".into(), "p:q".into())));
        assert!(parse_user_pass("u").is_err());
        assert!(parse_user_pass(":p").is_err());
        assert!(parse_user_pass("u:").is_err());
    }

    #[test]
    fn cli_is_well_formed() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
