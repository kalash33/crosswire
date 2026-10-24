//! Helpers for the loopback REST + WebSocket API listener that `serve`
//! composes alongside the proxy: binding it, loading the persisted scope, and
//! delivering the one-time bootstrap token out of band.

use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;

use crosswire_scope::Scope;
use crosswire_store::Control;
use tokio::net::TcpListener;

use crate::error::CliError;

/// The settings key holding the serialized project scope (`Scope` as JSON).
pub(crate) const SETTING_SCOPE: &str = "scope";

/// The settings key holding the proxy's bound address, written by `serve` at
/// startup so `crosswire browser` can find it.
pub(crate) const SETTING_PROXY_BIND: &str = "proxy_bind";

/// Loads the persisted scope, falling back to the empty scope when nothing is
/// stored or the stored value does not parse (a warning is logged then).
pub(crate) fn load_scope(control: &Control) -> Result<Scope, CliError> {
    let raw = control
        .setting_get(SETTING_SCOPE)
        .map_err(|e| CliError::runtime("reading the stored scope", e))?;
    match raw {
        Some(json) => match serde_json::from_str::<Scope>(&json) {
            Ok(scope) => Ok(scope),
            Err(e) => {
                tracing::warn!(error = %e, "the stored scope did not parse; starting with an empty scope");
                Ok(Scope::default())
            }
        },
        None => Ok(Scope::default()),
    }
}

/// Binds the API listener. A non-loopback bind prints a warning to stderr and
/// binds anyway (spec §11.2); the API itself still serves only `localhost` and
/// `127.0.0.1` hosts.
pub(crate) async fn bind_api(bind: SocketAddr) -> Result<(TcpListener, SocketAddr), CliError> {
    if !bind.ip().is_loopback() {
        eprintln!("warning: the API should stay on loopback");
    }
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|e| CliError::runtime(&format!("could not bind the API listener {bind}"), e))?;
    let addr = listener
        .local_addr()
        .map_err(|e| CliError::runtime("reading the API listener address", e))?;
    Ok((listener, addr))
}

/// The one-time bootstrap URL the web UI opens to exchange the token for a
/// session. The token lives only in the fragment, which browsers never send.
pub(crate) fn bootstrap_url(port: u16, token: &str) -> String {
    format!("http://localhost:{port}/#bootstrap={token}")
}

/// Delivers the bootstrap URL and returns the stdout line to print for it.
///
/// With `--bootstrap-file` the URL (with the token) is written to a 0600 file
/// and the returned line names only the path — never the token. Otherwise the
/// returned line is the URL itself, printed on stdout exactly once.
pub(crate) fn deliver_bootstrap(
    bootstrap_file: Option<&Path>,
    url: &str,
) -> Result<String, CliError> {
    match bootstrap_file {
        Some(path) => {
            write_private(path, url)
                .map_err(|e| CliError::runtime(&format!("writing {}", path.display()), e))?;
            Ok(format!(
                "bootstrap url written to {} (valid for 60 s)",
                path.display()
            ))
        }
        None => Ok(format!("open {url} within 60 s")),
    }
}

/// Writes `contents` to `path` with 0600 permissions (owner read/write only).
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    let mut opts = OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    // Set the mode explicitly too: `mode` only applies when the file is
    // created, and the file may have pre-existed with looser permissions.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(contents.as_bytes())?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}
