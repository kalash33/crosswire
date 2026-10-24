//! A real headless Chromium session through the proxy, recorded end to end
//! into a real project (a `Store` on disk), read back through `Store::list`
//! and `Store::get`.
//!
//! Ignored by default and skipped unless `CROSSWIRE_BROWSER` names a
//! Chromium-based binary: `just browser-test` runs it.

mod common;

use std::collections::BTreeSet;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use common::relay::wait_for;
use common::upstream::spawn_tls_html;
use common::{loopback_config, ProxyHarness, UpstreamWiring};
use crosswire_model::{Exchange, ExchangeState, Fidelity, Head};
use crosswire_store::{ExchangeFilter, Project, Store, StoreConfig};

#[tokio::test]
#[ignore = "needs a Chromium binary: set CROSSWIRE_BROWSER (see `just browser-test`)"]
async fn chromium_session_is_recorded() {
    let Some(browser) = std::env::var_os("CROSSWIRE_BROWSER").filter(|v| !v.is_empty()) else {
        eprintln!("CROSSWIRE_BROWSER is unset: skipping the browser session test");
        return;
    };

    let up = spawn_tls_html("<html>hi</html>").await;
    let wiring =
        UpstreamWiring::loopback(up.ca_der(), up.port(), &["browser.test"]).denying_unknown();
    let project = tempfile::tempdir().unwrap();
    let store = Store::open(
        Project::open_or_create(project.path()).unwrap(),
        StoreConfig {
            batch_interval: Duration::from_millis(10),
            ..StoreConfig::default()
        },
    )
    .unwrap();
    let h = ProxyHarness::start_with_sink(store.sink(), loopback_config(), wiring).await;

    let profile = tempfile::tempdir().unwrap();
    // The upstream port rides in the URL so the CONNECT authority is
    // `browser.test:<port>`, which the destination gate allows.
    let url = format!("https://browser.test:{}/", up.port());
    let mut child = Command::new(&browser)
        .args([
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--no-first-run",
            "--disable-background-networking",
            "--disable-component-update",
            "--disable-sync",
            "--no-default-browser-check",
            "--disable-default-apps",
            "--disable-features=OptimizationHints,MediaRouter,Translate",
            // CI runners: /dev/shm is tiny, and the stray background
            // requests the proxy refuses must not keep the dump waiting.
            // `--timeout` is Chrome's own headless budget: after it the
            // page is stopped and the DOM dumped, so the test's 30 s
            // deadline is never reached on a slow runner.
            "--disable-dev-shm-usage",
            "--disable-extensions",
            "--timeout=15000",
        ])
        .arg(format!("--proxy-server=127.0.0.1:{}", h.addr.port()))
        .arg("--proxy-bypass-list=<-loopback>")
        .arg(format!(
            "--ignore-certificate-errors-spki-list={}",
            h.ca.spki_sha256_base64()
        ))
        .arg(format!("--user-data-dir={}", profile.path().display()))
        .arg("--dump-dom")
        .arg(&url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the browser starts");

    // Helper processes can inherit the pipes and outlive a killed parent, so
    // the readers report over channels and every wait on them is bounded.
    let (out_rx, err_rx) = (
        read_pipe(child.stdout.take().unwrap()),
        read_pipe(child.stderr.take().unwrap()),
    );

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("the browser can be polled") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            let err = err_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap_or_default();
            panic!(
                "the browser did not exit within 30 s; stderr so far:\n{}",
                String::from_utf8_lossy(&err)
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let dom = String::from_utf8_lossy(
        &out_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("stdout closed within 5 s of the browser exiting"),
    )
    .into_owned();
    let err = String::from_utf8_lossy(
        &err_rx
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_default(),
    )
    .into_owned();
    assert!(
        dom.contains("hi"),
        "DOM dump lacks the page ({status}):\n{dom}\nstderr:\n{err}"
    );

    // Every exchange the store holds, read back from disk.
    let stored = |store: &Store| -> Vec<Exchange> {
        let rows = store.list(&ExchangeFilter::default(), None, 1000).unwrap();
        rows.items
            .iter()
            .map(|r| {
                store
                    .get(r.id)
                    .unwrap()
                    .expect("a listed exchange reads back")
            })
            .collect()
    };
    let is_browser_row = |ex: &Exchange| {
        let c = ex.request().connection();
        c.sni() == Some("browser.test")
            && c.tls_version().is_some()
            && ex.request().fidelity() == Fidelity::Raw
            && ex.response().is_some_and(|r| r.fidelity() == Fidelity::Raw)
            && ex.state() == ExchangeState::Committed
    };
    wait_for("a stored browser.test exchange", || {
        stored(&store).iter().any(is_browser_row)
    })
    .await;

    h.shutdown.shutdown().await;
    let mut hosts = BTreeSet::new();
    for ex in stored(&store) {
        let c = ex.request().connection();
        let host = c.sni().unwrap_or_else(|| c.target_addr()).to_string();
        let resp = ex.response();
        let generated_502 = resp.is_some_and(|r| {
            r.fidelity() == Fidelity::Generated
                && matches!(r.head(), Head::Http1Raw { bytes, .. }
                    if bytes.starts_with(b"HTTP/1.1 502 "))
        });
        eprintln!(
            "exchange: host={host} port={} state={:?} tls={:?} generated_502={generated_502}",
            c.port(),
            ex.state(),
            c.tls_version(),
        );
        if host != "browser.test" {
            assert!(
                ex.state() == ExchangeState::Error && generated_502,
                "stray host {host} was not refused with a generated 502"
            );
        }
        hosts.insert(host);
    }
    eprintln!("hosts seen: {hosts:?}");
    store.close().unwrap();
}

/// Reads `pipe` to the end on a thread and sends the bytes once.
fn read_pipe(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = pipe.read_to_end(&mut v);
        let _ = tx.send(v);
    });
    rx
}
