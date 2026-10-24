//! `crosswire browser`: launch a Chromium-based browser pre-configured to use
//! the proxy, or reset its profile.
//!
//! The launcher never phones home: it opens only the URL the user asked for
//! (or nothing). It trusts the proxy's CA via a certificate pin passed on the
//! command line, so no system trust store is touched.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crosswire_store::{Project, Store, StoreConfig};

use crate::api_cmd::SETTING_PROXY_BIND;
use crate::ca_cmd::{load_or_generate, resolve_ca_dir};
use crate::cli::{BrowserArgs, BrowserCommand, BrowserLaunchArgs};
use crate::error::CliError;

/// The actor recorded in the audit log for browser commands.
const ACTOR: &str = "cli";

/// The browser profile directory, under the project.
const PROFILE_DIR: &str = "browser-profile";

/// Runs a `crosswire browser` command.
///
/// # Errors
/// [`CliError::NoBrowser`] (exit 3) when no supported browser is found;
/// [`CliError::Config`] / [`CliError::Runtime`] for bad input or failures.
pub fn run(args: BrowserArgs) -> Result<(), CliError> {
    match args.command {
        Some(BrowserCommand::ResetProfile { project }) => reset_profile(&project),
        None => launch(args.launch),
    }
}

fn launch(args: BrowserLaunchArgs) -> Result<(), CliError> {
    let project = args
        .project
        .clone()
        .ok_or_else(|| CliError::Config("--project is required to launch a browser".into()))?;
    let proxy = resolve_proxy(&args, &project)?;

    // Pick the browser first, so the manual fallback can still print the proxy.
    let browser = match &args.browser_path {
        Some(path) if path.exists() => path.clone(),
        // An explicit path that does not exist falls back like "none found".
        Some(_) => return fallback(&proxy),
        None => match detect_browser() {
            Some(path) => path,
            None => return fallback(&proxy),
        },
    };

    let profile = project.join(PROFILE_DIR);
    create_profile_dir(&profile)?;

    // The certificate pin lets the browser trust the proxy's leaves without
    // touching any system trust store.
    let ca_dir = resolve_ca_dir(args.ca_dir.clone())?;
    let spki = load_or_generate(&ca_dir)?.ca.spki_sha256_base64();

    let mut cmd = Command::new(&browser);
    cmd.arg(format!("--proxy-server={proxy}"))
        .arg("--proxy-bypass-list=<-loopback>")
        .arg(format!("--user-data-dir={}", profile.display()))
        .arg(format!("--ignore-certificate-errors-spki-list={spki}"))
        .arg("--no-first-run")
        .arg("--no-default-browser-check");
    if args.headless {
        cmd.arg("--headless=new");
    }
    for extra in &args.extra {
        cmd.arg(extra);
    }
    if let Some(url) = &args.url {
        cmd.arg(url);
    }

    match cmd.status() {
        Ok(status) => {
            if !status.success() {
                tracing::warn!(%status, "the browser exited with a non-zero status");
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fallback(&proxy),
        Err(e) => Err(CliError::runtime(
            &format!("launching {}", browser.display()),
            e,
        )),
    }
}

/// Resolves the proxy address: `--proxy`, else the project's recorded
/// `proxy_bind` (readable only when `serve` is not holding the project).
fn resolve_proxy(args: &BrowserLaunchArgs, project: &Path) -> Result<String, CliError> {
    if let Some(proxy) = &args.proxy {
        return Ok(proxy.clone());
    }
    let opened = Project::open_or_create(project).map_err(|e| {
        CliError::runtime(
            &format!(
                "opening project {} to read the proxy address (pass --proxy if `crosswire serve` is running)",
                project.display()
            ),
            e,
        )
    })?;
    let store = Store::open(opened, StoreConfig::default())
        .map_err(|e| CliError::runtime("opening the store", e))?;
    let read = store
        .control()
        .map_err(|e| CliError::runtime("opening a control handle", e))
        .and_then(|c| {
            c.setting_get(SETTING_PROXY_BIND)
                .map_err(|e| CliError::runtime("reading the proxy address", e))
        });
    let closed = store
        .close()
        .map_err(|e| CliError::runtime("closing the store", e));
    let recorded = read.and_then(|opt| closed.map(|()| opt))?;
    recorded.ok_or_else(|| {
        CliError::Config(
            "no proxy address recorded; pass --proxy or run `crosswire serve` first".into(),
        )
    })
}

/// Deletes the project's browser profile (auditing `browser.profile_reset`).
fn reset_profile(project: &Path) -> Result<(), CliError> {
    let profile = project.join(PROFILE_DIR);
    if profile.exists() {
        std::fs::remove_dir_all(&profile)
            .map_err(|e| CliError::runtime(&format!("deleting {}", profile.display()), e))?;
    }
    let store = Project::open_or_create(project)
        .map_err(|e| {
            CliError::runtime(
                &format!(
                    "opening project {} to audit the reset (is `crosswire serve` running?)",
                    project.display()
                ),
                e,
            )
        })
        .and_then(|p| {
            Store::open(p, StoreConfig::default())
                .map_err(|e| CliError::runtime("opening the store", e))
        })?;
    let audited = store
        .audit(
            ACTOR,
            "browser.profile_reset",
            &profile.display().to_string(),
            "ok",
        )
        .map_err(|e| CliError::runtime("writing the audit row", e));
    let closed = store
        .close()
        .map_err(|e| CliError::runtime("closing the store", e));
    audited.and(closed)?;
    println!("reset browser profile {}", profile.display());
    Ok(())
}

/// Prints the manual instructions and returns the exit-3 error.
fn fallback(proxy: &str) -> Result<(), CliError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "no supported browser found")
        .and_then(|()| writeln!(out, "configure your browser to use the proxy at {proxy}"))
        .and_then(|()| writeln!(out, "then open http://crosswire/"))
        .and_then(|()| out.flush())
        .map_err(|e| CliError::runtime("writing stdout", e))?;
    Err(CliError::NoBrowser("no supported browser found".into()))
}

/// Creates the profile directory with owner-only (0700) permissions.
fn create_profile_dir(path: &Path) -> Result<(), CliError> {
    std::fs::create_dir_all(path)
        .map_err(|e| CliError::runtime(&format!("creating {}", path.display()), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| CliError::runtime(&format!("securing {}", path.display()), e))?;
    }
    Ok(())
}

/// Finds the first installed Chromium-based browser, in the order
/// Chrome → Chromium → Edge → Brave.
#[cfg(target_os = "macos")]
fn detect_browser() -> Option<PathBuf> {
    const CANDIDATES: &[&str] = &[
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    ];
    CANDIDATES.iter().map(PathBuf::from).find(|p| p.exists())
}

/// Finds the first installed Chromium-based browser under `Program Files`.
#[cfg(target_os = "windows")]
fn detect_browser() -> Option<PathBuf> {
    let roots: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "LocalAppData"]
        .iter()
        .filter_map(|v| std::env::var_os(v).map(PathBuf::from))
        .collect();
    const SUFFIXES: &[&str] = &[
        r"Google\Chrome\Application\chrome.exe",
        r"Chromium\Application\chrome.exe",
        r"Microsoft\Edge\Application\msedge.exe",
        r"BraveSoftware\Brave-Browser\Application\brave.exe",
    ];
    for suffix in SUFFIXES {
        for root in &roots {
            let p = root.join(suffix);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

/// Finds the first Chromium-based browser on `PATH`.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn detect_browser() -> Option<PathBuf> {
    const NAMES: &[&str] = &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
        "microsoft-edge-stable",
        "brave-browser",
    ];
    NAMES.iter().find_map(|n| which(n))
}

/// Looks up an executable by name on `PATH`.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}
