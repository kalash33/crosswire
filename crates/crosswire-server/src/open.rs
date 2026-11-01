//! `serve --open` and `crosswire app`: opening the one-time bootstrap URL so
//! the user never copies the 60-second token by hand, and stopping the
//! browsers `serve` launched when it stops.
//!
//! Every launch is a [`std::process::Command`] with the URL as a single argv
//! entry; nothing is ever passed through a shell string. The bootstrap URL is
//! built by crosswire itself (`http://localhost:PORT/#bootstrap=` plus a
//! URL-safe base64 token), so it holds no character `cmd /c start` would
//! interpret on Windows (pinned by a test below). The token is single-use and
//! consumed by the first page load, so its brief appearance in the opener's
//! argv (visible to local `ps`) is spent by the time anyone could reuse it.
//!
//! A failure to open (no browser, a spawn error, a non-zero opener exit) only
//! prints a one-line warning on stderr: `serve` keeps running and its exit
//! code is unchanged.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::browser::{create_profile_dir, find_browser};
use crate::cli::OpenMode;

/// Overrides the default-browser opener (`open`, `xdg-open`, `cmd /c start`)
/// with a program of the caller's choice, which receives the URL as its only
/// argument. Honoured only when opening in the default browser (including
/// the fallback from `--open=app`), never for app mode. Tests point it at a
/// nonexistent path.
pub(crate) const OPENER_ENV: &str = "CROSSWIRE_OPENER";

/// The app window's own profile directory, under the project. Separate from
/// the wired browser's profile, so the two run as separate browser instances
/// (a Chromium launched on an in-use profile would hand its arguments to the
/// running instance instead of opening its own window).
pub(crate) const APP_PROFILE_DIR: &str = "app-profile";

/// How long a launched browser gets to exit after SIGTERM before it is
/// killed.
const STOP_GRACE: Duration = Duration::from_secs(5);

/// A program and its arguments, built without a shell.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Launch {
    /// The program to run.
    pub program: OsString,
    /// Its arguments; the URL is always exactly one of them, intact.
    pub args: Vec<OsString>,
    /// `--open=app` found no Chromium-based browser and fell back to the
    /// default browser.
    pub fell_back: bool,
}

impl Launch {
    /// The command, with stdin, stdout and stderr detached so a browser's
    /// own logging does not interleave with `serve`'s output.
    pub(crate) fn command(&self) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args);
        quiet(&mut cmd);
        cmd
    }
}

/// What [`open_command`] needs to know about the machine.
#[derive(Debug)]
pub(crate) struct OpenEnv<'a> {
    /// The platform, as [`std::env::consts::OS`] spells it.
    pub os: &'a str,
    /// The [`OPENER_ENV`] override, if set.
    pub opener: Option<&'a Path>,
    /// The Chromium-based browser for app mode, if one was found.
    pub browser: Option<&'a Path>,
    /// The app window's profile directory.
    pub app_profile: &'a Path,
}

/// Builds the command that opens `url` in `mode`. Pure: nothing is spawned.
pub(crate) fn open_command(mode: OpenMode, url: &str, env: &OpenEnv<'_>) -> Launch {
    match (mode, env.browser) {
        (OpenMode::App, Some(browser)) => {
            let mut profile = OsString::from("--user-data-dir=");
            profile.push(env.app_profile.as_os_str());
            Launch {
                program: browser.as_os_str().to_owned(),
                args: vec![
                    format!("--app={url}").into(),
                    profile,
                    "--no-first-run".into(),
                    "--no-default-browser-check".into(),
                ],
                fell_back: false,
            }
        }
        (OpenMode::App, None) => Launch {
            fell_back: true,
            ..default_opener(url, env)
        },
        (OpenMode::Browser, _) => default_opener(url, env),
    }
}

/// The platform's "open this URL in the default browser" command, or the
/// [`OPENER_ENV`] override.
fn default_opener(url: &str, env: &OpenEnv<'_>) -> Launch {
    let (program, mut args): (OsString, Vec<OsString>) = match (env.opener, env.os) {
        (Some(opener), _) => (opener.as_os_str().to_owned(), Vec::new()),
        (None, "macos") => ("open".into(), Vec::new()),
        // `start`'s first quoted argument is the window title, hence `""`.
        // `cmd` re-parses its command line, so this is safe only while the
        // URL holds no `cmd` metacharacter: any change to the bootstrap URL
        // format must keep `bootstrap_urls_hold_no_cmd_metacharacter` passing.
        (None, "windows") => ("cmd".into(), vec!["/c".into(), "start".into(), "".into()]),
        (None, _) => ("xdg-open".into(), Vec::new()),
    };
    args.push(url.into());
    Launch {
        program,
        args,
        fell_back: false,
    }
}

/// Opens `url` in `mode`, warning on stderr when it cannot. Returns the app
/// window (to be stopped with `serve`); a default-browser opener exits on its
/// own and is reaped in the background.
pub(crate) fn open_ui(
    mode: OpenMode,
    url: &str,
    browser_path: Option<&Path>,
    project: &Path,
) -> Option<Child> {
    let browser = match mode {
        OpenMode::App => find_browser(browser_path),
        OpenMode::Browser => None,
    };
    let opener: Option<PathBuf> = std::env::var_os(OPENER_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let profile = project.join(APP_PROFILE_DIR);
    if browser.is_some() {
        if let Err(e) = create_profile_dir(&profile) {
            warn(&format!(
                "could not open the UI: {e}; open the bootstrap URL by hand"
            ));
            return None;
        }
    }
    let launch = open_command(
        mode,
        url,
        &OpenEnv {
            os: std::env::consts::OS,
            opener: opener.as_deref(),
            browser: browser.as_deref(),
            app_profile: &profile,
        },
    );
    if launch.fell_back {
        warn("no Chromium-based browser found for --open=app; opening the default browser instead");
    }
    let program = Path::new(&launch.program).display().to_string();
    match launch.command().spawn() {
        Err(e) => {
            warn(&format!(
                "could not open the UI with {program}: {e}; open the bootstrap URL by hand"
            ));
            None
        }
        Ok(child) if mode == OpenMode::App && !launch.fell_back => Some(child),
        Ok(child) => {
            reap_in_background(child, program);
            None
        }
    }
}

/// Waits for a short-lived opener so it is not left a zombie, warning if it
/// reports failure (for example `xdg-open` with no browser configured).
fn reap_in_background(mut child: Child, program: String) {
    std::thread::spawn(move || match child.wait() {
        Ok(status) if !status.success() => warn(&format!(
            "{program} exited with {status}; open the bootstrap URL by hand"
        )),
        Ok(_) => {}
        Err(e) => warn(&format!("waiting for {program}: {e}")),
    });
}

/// The browsers `serve` launched (the app window, the wired browser), stopped
/// when `serve` stops so one Ctrl-C ends the whole workstation.
#[derive(Debug, Default)]
pub(crate) struct Launched(Vec<Child>);

impl Launched {
    /// Tracks a launched browser, if one was launched.
    pub(crate) fn extend(&mut self, child: Option<Child>) {
        self.0.extend(child);
    }

    /// Asks every browser still running to exit (SIGTERM on Unix, so it
    /// shuts its profile down cleanly), then kills any still running after
    /// [`STOP_GRACE`]. Blocks until all have exited.
    pub(crate) fn stop(mut self) {
        for child in &mut self.0 {
            if matches!(child.try_wait(), Ok(None)) {
                terminate(child);
            }
        }
        let deadline = Instant::now() + STOP_GRACE;
        for child in &mut self.0 {
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

/// Sends SIGTERM through `kill(1)` (the crate forbids `unsafe`, so no direct
/// `libc::kill`): `/bin/kill` first, then `kill` from `PATH`, and
/// [`Child::kill`] (SIGKILL) when neither succeeds.
#[cfg(unix)]
fn terminate(child: &mut Child) {
    let pid = child.id().to_string();
    let sent = ["/bin/kill", "kill"].iter().any(|program| {
        let mut cmd = Command::new(program);
        cmd.arg("-TERM").arg(&pid);
        quiet(&mut cmd);
        cmd.status().is_ok_and(|s| s.success())
    });
    if !sent {
        let _ = child.kill();
    }
}

/// Stops the child (Windows has no SIGTERM).
#[cfg(not(unix))]
fn terminate(child: &mut Child) {
    let _ = child.kill();
}

/// Detaches a launched program's standard streams.
pub(crate) fn quiet(cmd: &mut Command) {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
}

/// Prints a one-line warning on stderr (stdout carries the banner).
pub(crate) fn warn(msg: &str) {
    eprintln!("warning: {msg}");
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL: &str = "http://localhost:8081/#bootstrap=Ab-_9z";

    fn env<'a>(os: &'a str, opener: Option<&'a Path>, browser: Option<&'a Path>) -> OpenEnv<'a> {
        OpenEnv {
            os,
            opener,
            browser,
            app_profile: Path::new("/proj/app-profile"),
        }
    }

    fn strs(l: &Launch) -> (String, Vec<String>) {
        (
            l.program.to_string_lossy().into_owned(),
            l.args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect(),
        )
    }

    #[test]
    fn browser_mode_uses_the_platform_opener_with_the_url_as_one_argument() {
        for (os, program, args) in [
            ("macos", "open", vec![URL]),
            ("linux", "xdg-open", vec![URL]),
            ("freebsd", "xdg-open", vec![URL]),
            ("windows", "cmd", vec!["/c", "start", "", URL]),
        ] {
            let l = open_command(OpenMode::Browser, URL, &env(os, None, None));
            assert_eq!(
                strs(&l),
                (
                    program.to_string(),
                    args.iter().map(|a| a.to_string()).collect()
                ),
                "{os}"
            );
            assert!(!l.fell_back);
            // No shell: the program is never `sh`/`bash`, and no argument is
            // a `-c` script that embeds the URL.
            assert!(!["sh", "bash", "zsh"].contains(&program));
            assert_eq!(
                l.args
                    .iter()
                    .filter(|a| a.to_string_lossy().contains(URL))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn browser_mode_ignores_a_detected_browser() {
        let l = open_command(
            OpenMode::Browser,
            URL,
            &env("linux", None, Some(Path::new("/usr/bin/chromium"))),
        );
        assert_eq!(strs(&l).0, "xdg-open");
    }

    #[test]
    fn the_opener_override_replaces_the_platform_opener() {
        for os in ["macos", "linux", "windows"] {
            let l = open_command(
                OpenMode::Browser,
                URL,
                &env(os, Some(Path::new("/nonexistent")), None),
            );
            assert_eq!(
                strs(&l),
                ("/nonexistent".into(), vec![URL.to_string()]),
                "{os}"
            );
        }
    }

    #[test]
    fn app_mode_launches_the_browser_with_app_and_its_own_profile() {
        let browser = Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome");
        for os in ["macos", "linux", "windows"] {
            // The opener override is ignored in app mode.
            let l = open_command(
                OpenMode::App,
                URL,
                &env(os, Some(Path::new("/nonexistent")), Some(browser)),
            );
            let (program, args) = strs(&l);
            assert_eq!(program, browser.to_string_lossy(), "{os}");
            assert_eq!(
                args,
                vec![
                    format!("--app={URL}"),
                    "--user-data-dir=/proj/app-profile".to_string(),
                    "--no-first-run".to_string(),
                    "--no-default-browser-check".to_string(),
                ],
                "{os}"
            );
            assert!(!l.fell_back);
        }
    }

    #[test]
    fn app_mode_without_a_browser_falls_back_to_the_default_opener() {
        let l = open_command(OpenMode::App, URL, &env("macos", None, None));
        assert!(l.fell_back);
        assert_eq!(strs(&l), ("open".into(), vec![URL.to_string()]));
        // The fallback is the default-browser path, so the override applies.
        let l = open_command(
            OpenMode::App,
            URL,
            &env("linux", Some(Path::new("/x/opener")), None),
        );
        assert!(l.fell_back);
        assert_eq!(strs(&l), ("/x/opener".into(), vec![URL.to_string()]));
    }

    #[test]
    fn a_url_with_spaces_and_quotes_stays_one_argument() {
        let hostile = "http://localhost:1/#a b\"c';rm -rf ~&calc";
        let l = open_command(OpenMode::Browser, hostile, &env("linux", None, None));
        assert_eq!(l.args, vec![OsString::from(hostile)]);
    }

    #[test]
    fn bootstrap_urls_hold_no_cmd_metacharacter() {
        // `cmd /c start` re-parses its command line; the URL crosswire builds
        // must stay within a safe alphabet.
        let store = crosswire_api::BootstrapStore::new();
        for _ in 0..64 {
            let url = crate::api_cmd::bootstrap_url(8081, &store.mint(Duration::from_secs(60)));
            assert!(
                url.chars()
                    .all(|c| c.is_ascii_alphanumeric() || ":/#=._-".contains(c)),
                "{url}"
            );
        }
    }
}
