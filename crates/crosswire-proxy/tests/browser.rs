//! A real headless Chromium session through the proxy, recorded end to end.
//!
//! Ignored by default and skipped unless `CROSSWIRE_BROWSER` names a
//! Chromium-based binary: `just browser-test` runs it.

mod common;

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::relay::wait_for;
use common::upstream::spawn_tls_html;
use common::{loopback_config, ProxyHarness, UpstreamWiring};
use crosswire_model::Fidelity;
use crosswire_proxy::NoopHandler;

#[tokio::test]
#[ignore = "needs a Chromium binary: set CROSSWIRE_BROWSER (see `just browser-test`)"]
async fn chromium_session_is_recorded() {
    let Some(browser) = std::env::var_os("CROSSWIRE_BROWSER").filter(|v| !v.is_empty()) else {
        eprintln!("CROSSWIRE_BROWSER is unset: skipping the browser session test");
        return;
    };

    let up = spawn_tls_html("<html>hi</html>").await;
    let wiring = UpstreamWiring::loopback(up.ca_der(), up.port(), &["browser.test"]);
    let h = ProxyHarness::start_wired(loopback_config(), Arc::new(NoopHandler), wiring).await;

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

    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out_task = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stdout.read_to_end(&mut v);
        v
    });
    let err_task = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = stderr.read_to_end(&mut v);
        v
    });

    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().expect("the browser can be polled") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            let err = err_task.join().unwrap();
            panic!(
                "the browser did not exit within 30 s; stderr:\n{}",
                String::from_utf8_lossy(&err)
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let dom = String::from_utf8_lossy(&out_task.join().unwrap()).into_owned();
    let err = String::from_utf8_lossy(&err_task.join().unwrap()).into_owned();
    assert!(
        dom.contains("hi"),
        "DOM dump lacks the page ({status}):\n{dom}\nstderr:\n{err}"
    );

    let recorded = |h: &ProxyHarness| {
        h.sink.exchanges.lock().unwrap().iter().any(|ex| {
            let c = ex.request().connection();
            c.sni() == Some("browser.test")
                && c.tls_version().is_some()
                && ex.request().fidelity() == Fidelity::Raw
        })
    };
    wait_for("a recorded browser.test exchange", || recorded(&h)).await;

    for ex in h.sink.exchanges.lock().unwrap().iter() {
        let c = ex.request().connection();
        eprintln!(
            "exchange: target={} sni={:?} tls={:?} fidelity={:?}",
            c.target_addr(),
            c.sni(),
            c.tls_version(),
            ex.request().fidelity()
        );
    }
    h.shutdown.shutdown().await;
}
