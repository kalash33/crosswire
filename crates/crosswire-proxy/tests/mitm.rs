//! CONNECT handling: TLS termination with issued leaves, SNI passthrough
//! tunnels and handshake-rejection reporting.

mod common;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use common::upstream::spawn_tls_echo;
use common::*;
use crosswire_model::{Head, MarkerKind, TunnelReason};
use crosswire_proxy::NoopHandler;
use rustls::{ClientConfig, RootCertStore};
use rustls_pki_types::{CertificateDer, ServerName};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;
use x509_parser::extensions::GeneralName;

/// A client config trusting exactly `roots` (DER).
fn client_config(roots: &[Vec<u8>]) -> Arc<ClientConfig> {
    let mut store = RootCertStore::empty();
    for der in roots {
        store.add(CertificateDer::from(der.clone())).unwrap();
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut cfg = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(store)
        .with_no_client_auth();
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Arc::new(cfg)
}

/// Sends `CONNECT authority` and reads the 200.
async fn connect_tunnel(proxy: SocketAddr, authority: &str) -> TcpStream {
    let mut s = connect_raw(proxy).await;
    let req = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let got = read_until(&mut s, b"\r\n\r\n").await;
    assert_eq!(got, b"HTTP/1.1 200 Connection established\r\n\r\n");
    s
}

async fn handshake(
    tcp: TcpStream,
    cfg: Arc<ClientConfig>,
    name: ServerName<'static>,
) -> std::io::Result<TlsStream<TcpStream>> {
    tokio::time::timeout(
        Duration::from_secs(5),
        TlsConnector::from(cfg).connect(name, tcp),
    )
    .await
    .expect("handshake finished in time")
}

/// The subject alternative names of the leaf the client saw.
fn leaf_sans(tls: &TlsStream<TcpStream>) -> (Vec<String>, Vec<IpAddr>) {
    let (_, conn) = tls.get_ref();
    let der = conn.peer_certificates().unwrap()[0].clone();
    let (_, cert) = x509_parser::parse_x509_certificate(der.as_ref()).unwrap();
    let san = cert.subject_alternative_name().unwrap().unwrap();
    let (mut dns, mut ips) = (Vec::new(), Vec::new());
    for name in &san.value.general_names {
        match name {
            GeneralName::DNSName(d) => dns.push((*d).to_string()),
            GeneralName::IPAddress(b) if b.len() == 4 => {
                ips.push(IpAddr::from(<[u8; 4]>::try_from(*b).unwrap()));
            }
            _ => {}
        }
    }
    (dns, ips)
}

async fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !cond() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

#[tokio::test]
async fn mitm_leaf_has_host_san_and_is_trusted_by_ca() {
    let h = ProxyHarness::start_default().await;
    let tcp = connect_tunnel(h.addr, "example.test:443").await;
    let cfg = client_config(&[h.ca.cert_der()]);
    let tls = handshake(tcp, cfg, ServerName::try_from("example.test").unwrap())
        .await
        .expect("client trusting the proxy CA completes the handshake");
    let (dns, _) = leaf_sans(&tls);
    assert!(dns.iter().any(|d| d == "example.test"), "{dns:?}");
    assert_eq!(tls.get_ref().1.alpn_protocol(), Some(&b"http/1.1"[..]));
    assert_eq!(h.counters.handshake_rejections.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn ip_literal_gets_ip_san() {
    let h = ProxyHarness::start_default().await;
    let tcp = connect_tunnel(h.addr, "127.0.0.1:8443").await;
    let cfg = client_config(&[h.ca.cert_der()]);
    let ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let tls = handshake(tcp, cfg, ServerName::IpAddress(ip.into()))
        .await
        .expect("IP-literal handshake succeeds");
    let (_, ips) = leaf_sans(&tls);
    assert_eq!(ips, vec![ip]);
}

#[tokio::test]
async fn passthrough_host_is_tunnelled_and_recorded() {
    let up = spawn_tls_echo().await;
    let mut cfg = loopback_config();
    cfg.passthrough = vec!["pass.test".to_string()];
    let wiring = UpstreamWiring::loopback(up.ca_der(), up.port(), &["pass.test"]);
    let h = ProxyHarness::start_wired(cfg, Arc::new(NoopHandler), wiring).await;

    let tcp = connect_tunnel(h.addr, &format!("pass.test:{}", up.port())).await;
    // Trusting only the upstream's CA: success means the session is with
    // the upstream itself, not a Crosswire leaf.
    let client = client_config(&[up.ca_der()]);
    let mut tls = handshake(tcp, client, ServerName::try_from("pass.test").unwrap())
        .await
        .expect("handshake with the upstream through the tunnel");
    tls.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    tls.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");
    tls.shutdown().await.unwrap();
    drop(tls);

    wait_for("the tunnel exchange", || {
        !h.sink.exchanges.lock().unwrap().is_empty()
    })
    .await;
    let exchanges = h.sink.exchanges.lock().unwrap();
    assert_eq!(exchanges.len(), 1);
    let req = exchanges[0].request();
    let Head::Tunnel {
        bytes_in,
        bytes_out,
        reason,
        ..
    } = req.head()
    else {
        panic!("expected a tunnel head, got {:?}", req.head());
    };
    assert_eq!(*reason, TunnelReason::SniPassthrough);
    assert!(*bytes_in > 0 && *bytes_out > 0, "{bytes_in} / {bytes_out}");
    assert_eq!(req.connection().sni(), Some("pass.test"));
    assert!(exchanges[0].response().is_none());
    assert_eq!(h.counters.tunnels.load(Ordering::Relaxed), 1);
    assert_eq!(up.received.lock().unwrap().as_slice(), b"ping");
}

#[tokio::test]
async fn passthrough_matches_authority_when_sni_differs() {
    let up = spawn_tls_echo().await;
    let mut cfg = loopback_config();
    cfg.passthrough = vec!["pass.test".to_string()];
    let wiring = UpstreamWiring::loopback(up.ca_der(), up.port(), &["pass.test"]);
    let h = ProxyHarness::start_wired(cfg, Arc::new(NoopHandler), wiring).await;

    // The authority is on the list; the SNI is not.
    let tcp = connect_tunnel(h.addr, &format!("pass.test:{}", up.port())).await;
    let client = client_config(&[up.ca_der()]);
    let mut tls = handshake(tcp, client, ServerName::try_from("other.test").unwrap())
        .await
        .expect("tunnelled to the upstream although the SNI is not listed");
    tls.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    tls.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");
    assert_eq!(h.counters.tunnels.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn three_rejections_in_window_emit_error_marker() {
    let h = ProxyHarness::start_default().await;
    for _ in 0..3 {
        let tcp = connect_tunnel(h.addr, "example.test:443").await;
        let trusts_nothing = client_config(&[]);
        let res = handshake(
            tcp,
            trusts_nothing,
            ServerName::try_from("example.test").unwrap(),
        )
        .await;
        assert!(res.is_err(), "a client without the CA must reject the leaf");
    }
    wait_for("three rejections", || {
        h.counters.handshake_rejections.load(Ordering::Relaxed) == 3
    })
    .await;
    wait_for("the marker", || !h.sink.markers.lock().unwrap().is_empty()).await;
    // Give a wrongly-duplicated marker the chance to show up.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let markers = h.sink.markers.lock().unwrap();
    assert_eq!(markers.len(), 1, "{markers:?}");
    assert_eq!(markers[0].kind(), MarkerKind::Error);
    assert!(markers[0].detail().contains("example.test"), "{markers:?}");
    assert_eq!(h.counters.handshake_rejections.load(Ordering::Relaxed), 3);
}
