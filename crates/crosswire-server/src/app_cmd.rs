//! `crosswire app`: `serve --open=app` plus a browser wired to the proxy, so
//! one command gives the whole workstation. Both browsers are stopped with
//! `serve` (see [`crate::open::Launched`]).

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::process::Child;

use crate::browser::{create_profile_dir, find_browser, wired_command, PROFILE_DIR};
use crate::cli::{AppArgs, OpenMode};
use crate::error::CliError;
use crate::open::{quiet, warn};
use crate::serve;

/// Runs `crosswire app` until Ctrl-C (or SIGTERM).
///
/// # Errors
/// [`CliError::Config`] for `--open=browser` (the app always opens in app
/// mode) and every `serve` configuration error; otherwise as `serve`.
pub fn run(args: AppArgs) -> Result<(), CliError> {
    let mut serve_args = args.serve;
    if serve_args.open == Some(OpenMode::Browser) {
        return Err(CliError::Config(
            "`crosswire app` always opens the UI in app mode; use `crosswire serve --open=browser` for a browser tab"
                .into(),
        ));
    }
    serve_args.open = Some(OpenMode::App);
    serve_args.wired_browser = true;
    serve::run(serve_args)
}

/// Launches a Chromium-based browser wired to the proxy at `proxy` (the
/// project's `browser-profile`, the CA pinned by `spki`), warning on stderr
/// and returning `None` when there is none or it cannot start.
pub(crate) fn launch_wired(
    browser_path: Option<&Path>,
    project: &Path,
    proxy: SocketAddr,
    spki: &str,
) -> Option<Child> {
    let proxy = dialable(proxy);
    let Some(browser) = find_browser(browser_path) else {
        warn(&format!(
            "no Chromium-based browser found; point a browser at the proxy {proxy} and open http://crosswire/ for the CA"
        ));
        return None;
    };
    let profile = project.join(PROFILE_DIR);
    if let Err(e) = create_profile_dir(&profile) {
        warn(&format!("could not launch the wired browser: {e}"));
        return None;
    }
    let mut cmd = wired_command(&browser, &proxy.to_string(), &profile, spki);
    quiet(&mut cmd);
    match cmd.spawn() {
        Ok(child) => Some(child),
        Err(e) => {
            warn(&format!(
                "could not launch the wired browser {}: {e}",
                browser.display()
            ));
            None
        }
    }
}

/// The address a local browser dials: an unspecified bind (`0.0.0.0`, `::`)
/// becomes the matching loopback address.
fn dialable(addr: SocketAddr) -> SocketAddr {
    match addr {
        SocketAddr::V4(a) if a.ip().is_unspecified() => {
            SocketAddr::from((Ipv4Addr::LOCALHOST, a.port()))
        }
        SocketAddr::V6(a) if a.ip().is_unspecified() => {
            SocketAddr::from((Ipv6Addr::LOCALHOST, a.port()))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unspecified_bind_is_dialled_on_loopback() {
        let d = |s: &str| dialable(s.parse().unwrap()).to_string();
        assert_eq!(d("0.0.0.0:8080"), "127.0.0.1:8080");
        assert_eq!(d("[::]:8080"), "[::1]:8080");
        assert_eq!(d("127.0.0.1:9"), "127.0.0.1:9");
        assert_eq!(d("192.0.2.1:9"), "192.0.2.1:9");
    }

    #[test]
    fn app_refuses_open_browser() {
        use clap::Parser;
        let cli = crate::cli::Cli::try_parse_from([
            "crosswire",
            "app",
            "--project",
            "/nonexistent/never-created",
            "--open=browser",
        ])
        .unwrap();
        let crate::cli::Command::App(a) = cli.command else {
            panic!("not app")
        };
        let err = run(a).unwrap_err();
        assert_eq!(err.exit_code(), 2, "{err}");
        assert!(!Path::new("/nonexistent/never-created").exists());
    }
}
