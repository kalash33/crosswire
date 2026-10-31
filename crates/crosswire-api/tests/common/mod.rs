//! Shared harness for the `crosswire-api` integration tests.
//!
//! It serves a real [`crosswire_api::router`] over a loopback TCP port with a
//! temp [`Store`], a [`Repeater`] and an [`Intercept`], and drives it with a
//! hand-written HTTP/1 and WebSocket client over raw [`tokio::net::TcpStream`]
//! (no `reqwest`), so the byte-exact header, cookie and upgrade behaviour is
//! exercised directly.
#![allow(dead_code, missing_docs)]

use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use crosswire_api::{bootstrap_token, router, ApiConfig, AppState};
use crosswire_ca::Ca;
use crosswire_client::{DestinationGate, RustlsBackend, Transport};
use crosswire_h1::{Parser, ParserConfig, Progress, ResponseContext};
use crosswire_model::{Body, ConnectionInfo, Exchange, Message};
use crosswire_proxy::{Budget, Intercept, InterceptRules, ProxyLimits};
use crosswire_scope::{Compiled, Scope};
use crosswire_store::{ExchangeFilter, Project, Store, StoreConfig};
use crosswire_tools::Repeater;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Options for building a [`TestApp`].
pub struct Opts {
    /// The project scope (compiled and stored).
    pub scope: Scope,
    /// Whether strict scope enforcement starts on.
    pub strict: bool,
    /// `(host, port)` pairs the repeater transport may dial.
    pub allow: Vec<(String, u16)>,
    /// Broadcast buffer per subscriber (small values force `lagged`).
    pub event_capacity: usize,
    /// Session idle timeout.
    pub idle_timeout: Duration,
    /// Bootstrap-token lifetime.
    pub bootstrap_ttl: Duration,
    /// The proxy address `/health` reports, if any.
    pub proxy_addr: Option<SocketAddr>,
}

impl Default for Opts {
    fn default() -> Opts {
        Opts {
            scope: Scope::default(),
            strict: false,
            allow: Vec::new(),
            event_capacity: 10_000,
            idle_timeout: Duration::from_secs(30 * 60),
            bootstrap_ttl: Duration::from_secs(60),
            proxy_addr: None,
        }
    }
}

/// A running API server and the handles the tests reach into.
pub struct TestApp {
    pub addr: SocketAddr,
    pub port: u16,
    pub state: AppState,
    pub store: Arc<Store>,
    pub scope: Arc<RwLock<Compiled>>,
    pub intercept: Arc<Intercept>,
    pub ca: Arc<Ca>,
    pub strict: Arc<AtomicBool>,
    /// The repeater's upstream transport, so gates can read its connect and
    /// DNS counters.
    pub transport: Arc<Transport>,
    _tmp: TempDir,
}

impl TestApp {
    /// A server with default options.
    pub async fn new() -> TestApp {
        TestApp::with(Opts::default()).await
    }

    /// A server with the given options.
    pub async fn with(opts: Opts) -> TestApp {
        let tmp = TempDir::new().unwrap();
        let project = Project::create(&tmp.path().join("p")).unwrap();
        let cfg = StoreConfig {
            batch_interval: Duration::from_millis(20),
            event_capacity: opts.event_capacity,
            ..StoreConfig::default()
        };
        let store = Arc::new(Store::open(project, cfg).unwrap());
        let ca = Arc::new(Ca::generate("api-test").unwrap());

        let compiled = opts.scope.compile().unwrap();
        let scope = Arc::new(RwLock::new(compiled));
        let scope_doc = Arc::new(RwLock::new(opts.scope.clone()));
        let strict = Arc::new(AtomicBool::new(opts.strict));

        let mut gate = DestinationGate::new(Vec::new());
        for (host, port) in &opts.allow {
            gate.allow(host, *port);
        }
        let transport = Arc::new(Transport::new(
            Arc::new(RustlsBackend::webpki_roots()),
            Arc::new(gate),
        ));
        let audit_store = Arc::clone(&store);
        let repeater = Arc::new(Repeater::new(
            Arc::clone(&transport),
            store.sink(),
            Arc::clone(&scope),
            Arc::clone(&strict),
            Arc::new(move |action: &str, target: &str, result: &str| {
                let _ = audit_store.audit("repeater", action, target, result);
            }),
            ParserConfig::default(),
        ));

        let limits = ProxyLimits::default();
        let budget = Arc::new(Budget::new(limits.body_budget));
        let intercept = Arc::new(Intercept::new(
            &limits,
            Arc::clone(&scope),
            InterceptRules::default(),
            budget,
            store.sink(),
        ));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let port = addr.port();

        let config = ApiConfig {
            bind: addr,
            idle_timeout: opts.idle_timeout,
            bootstrap_ttl: opts.bootstrap_ttl,
            proxy_addr: opts.proxy_addr,
        };
        let state = AppState::new(
            Arc::clone(&store),
            Arc::clone(&repeater),
            Arc::clone(&intercept),
            Arc::clone(&scope),
            scope_doc,
            Arc::clone(&ca),
            Arc::clone(&strict),
            port,
            &config,
        )
        .unwrap();

        let app = router(state.clone());
        tokio::spawn(async move {
            let _ = axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await;
        });

        TestApp {
            addr,
            port,
            state,
            store,
            scope,
            intercept,
            ca,
            strict,
            transport,
            _tmp: tmp,
        }
    }

    /// Mints a fresh bootstrap token for this server.
    pub fn mint_bootstrap(&self) -> String {
        bootstrap_token(&self.state)
    }

    /// Sends a raw HTTP/1 request and reads the whole response (the client
    /// sends `Connection: close`, so the server closes after answering).
    ///
    /// A `Host` header in `headers` overrides the default `localhost:<port>`.
    pub async fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Resp {
        let mut req = format!("{method} {path} HTTP/1.1\r\n");
        let has_host = headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host"));
        if !has_host {
            req.push_str(&format!("Host: localhost:{}\r\n", self.port));
        }
        req.push_str("Connection: close\r\n");
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        if let Some(b) = body {
            req.push_str(&format!("Content-Length: {}\r\n", b.len()));
        }
        req.push_str("\r\n");

        let mut stream = TcpStream::connect(self.addr).await.unwrap();
        stream.write_all(req.as_bytes()).await.unwrap();
        if let Some(b) = body {
            stream.write_all(b).await.unwrap();
        }
        stream.flush().await.unwrap();

        // Read the whole response. When the server short-circuits (for example
        // a 401 from the gate) it may close before draining the request body,
        // which surfaces as a connection reset; treat that as end-of-response
        // and parse whatever arrived.
        let mut raw = Vec::new();
        let mut tmp = [0u8; 8192];
        loop {
            match tokio::time::timeout(Duration::from_secs(10), stream.read(&mut tmp)).await {
                Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
                Ok(Ok(n)) => raw.extend_from_slice(&tmp[..n]),
            }
        }
        Resp::parse(&raw)
    }

    /// A GET with headers.
    pub async fn get(&self, path: &str, headers: &[(&str, &str)]) -> Resp {
        self.send("GET", path, headers, None).await
    }

    /// Creates a session and returns its `cw_sid_<port>=<id>` cookie pair.
    pub async fn session_cookie(&self) -> String {
        let token = self.mint_bootstrap();
        let body = format!("{{\"bootstrap\":\"{token}\"}}");
        let resp = self
            .send(
                "POST",
                "/api/v1/session",
                &[("Content-Type", "application/json")],
                Some(body.as_bytes()),
            )
            .await;
        assert_eq!(resp.status, 200, "bootstrap failed: {}", resp.body_string());
        let set_cookie = resp.header("set-cookie").expect("a Set-Cookie header");
        set_cookie
            .split(';')
            .next()
            .expect("the cookie pair")
            .to_string()
    }

    /// Creates an API token with the given scopes and returns its secret.
    pub async fn make_token(&self, name: &str, scopes: &[&str], cookie: &str) -> String {
        let scopes_json = scopes
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(",");
        let body = format!("{{\"name\":\"{name}\",\"scopes\":[{scopes_json}]}}");
        let resp = self
            .send(
                "POST",
                "/api/v1/tokens",
                &[
                    ("Cookie", cookie),
                    ("Sec-Fetch-Site", "same-origin"),
                    ("Content-Type", "application/json"),
                ],
                Some(body.as_bytes()),
            )
            .await;
        assert_eq!(
            resp.status,
            200,
            "token create failed: {}",
            resp.body_string()
        );
        let v = resp.json();
        v["secret"].as_str().expect("a secret").to_string()
    }
}

/// A parsed HTTP response.
pub struct Resp {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Resp {
    fn parse(raw: &[u8]) -> Resp {
        let split = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("a header terminator");
        let head = String::from_utf8_lossy(&raw[..split]);
        let body = raw[split + 4..].to_vec();
        let mut lines = head.split("\r\n");
        let status_line = lines.next().unwrap_or_default();
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let headers = lines
            .filter_map(|l| l.split_once(':'))
            .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
            .collect();
        Resp {
            status,
            headers,
            body,
        }
    }

    /// The first value of a header (case-insensitive name).
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Every value of a header.
    pub fn headers_all(&self, name: &str) -> Vec<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .filter(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// The body as a UTF-8 string.
    pub fn body_string(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body parsed as JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("a JSON body")
    }
}

/// A minimal WebSocket client over a raw stream.
pub struct Ws {
    stream: TcpStream,
    buf: Vec<u8>,
}

impl Ws {
    /// Performs the HTTP/1 upgrade handshake. Returns the server's status on
    /// failure (anything other than `101`).
    pub async fn connect(
        addr: SocketAddr,
        port: u16,
        path: &str,
        headers: &[(&str, &str)],
    ) -> Result<Ws, u16> {
        let mut req = format!("GET {path} HTTP/1.1\r\n");
        let has_host = headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host"));
        if !has_host {
            req.push_str(&format!("Host: localhost:{port}\r\n"));
        }
        req.push_str("Upgrade: websocket\r\n");
        req.push_str("Connection: Upgrade\r\n");
        req.push_str("Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n");
        req.push_str("Sec-WebSocket-Version: 13\r\n");
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        req.push_str("\r\n");

        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream.write_all(req.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();

        // Read until the end of the response headers.
        let mut buf = Vec::new();
        let mut tmp = [0u8; 1024];
        let status = loop {
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..pos]).into_owned();
                let status = head
                    .split("\r\n")
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|s| s.parse::<u16>().ok())
                    .unwrap_or(0);
                let rest = buf[pos + 4..].to_vec();
                buf = rest;
                break status;
            }
            let n = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut tmp))
                .await
                .expect("handshake in time")
                .unwrap();
            if n == 0 {
                break 0;
            }
            buf.extend_from_slice(&tmp[..n]);
        };

        if status != 101 {
            return Err(status);
        }
        Ok(Ws { stream, buf })
    }

    /// Sends a masked text frame.
    pub async fn send_text(&mut self, text: &str) {
        let payload = text.as_bytes();
        let mut frame = vec![0x81u8];
        let mask = [0x12u8, 0x34, 0x56, 0x78];
        let len = payload.len();
        assert!(len < 126, "test frames are small");
        frame.push(0x80 | len as u8);
        frame.extend_from_slice(&mask);
        for (i, b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i % 4]);
        }
        self.stream.write_all(&frame).await.unwrap();
        self.stream.flush().await.unwrap();
    }

    async fn fill(&mut self, n: usize, deadline: Duration) -> bool {
        let mut tmp = [0u8; 4096];
        while self.buf.len() < n {
            let got = tokio::time::timeout(deadline, self.stream.read(&mut tmp)).await;
            match got {
                Ok(Ok(0)) | Err(_) => return false,
                Ok(Ok(k)) => self.buf.extend_from_slice(&tmp[..k]),
                Ok(Err(_)) => return false,
            }
        }
        true
    }

    /// Reads the next text frame's payload, or `None` on close/timeout.
    pub async fn recv_text(&mut self, deadline: Duration) -> Option<String> {
        loop {
            if !self.fill(2, deadline).await {
                return None;
            }
            let b0 = self.buf[0];
            let b1 = self.buf[1];
            let opcode = b0 & 0x0f;
            let masked = b1 & 0x80 != 0;
            let mut idx = 2;
            let len7 = b1 & 0x7f;
            let len = if len7 < 126 {
                len7 as usize
            } else if len7 == 126 {
                if !self.fill(4, deadline).await {
                    return None;
                }
                let l = u16::from_be_bytes([self.buf[2], self.buf[3]]) as usize;
                idx = 4;
                l
            } else {
                if !self.fill(10, deadline).await {
                    return None;
                }
                let mut arr = [0u8; 8];
                arr.copy_from_slice(&self.buf[2..10]);
                idx = 10;
                u64::from_be_bytes(arr) as usize
            };
            let mask_len = if masked { 4 } else { 0 };
            if !self.fill(idx + mask_len + len, deadline).await {
                return None;
            }
            let mask = if masked {
                let m = [
                    self.buf[idx],
                    self.buf[idx + 1],
                    self.buf[idx + 2],
                    self.buf[idx + 3],
                ];
                idx += 4;
                Some(m)
            } else {
                None
            };
            let mut payload = self.buf[idx..idx + len].to_vec();
            if let Some(m) = mask {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= m[i % 4];
                }
            }
            self.buf.drain(..idx + len);
            match opcode {
                0x1 => return Some(String::from_utf8_lossy(&payload).into_owned()),
                0x8 => return None,
                _ => continue, // ping/pong/binary: skip
            }
        }
    }
}

impl TestApp {
    /// Submits an exchange to the store sink.
    pub fn submit(&self, exchange: Exchange) {
        self.store.sink().submit(exchange).unwrap();
    }

    /// Waits until the store lists at least `n` exchanges.
    pub async fn wait_count(&self, n: usize) {
        let store = Arc::clone(&self.store);
        let ok = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let page = store.list(&ExchangeFilter::default(), None, 1000).unwrap();
                if page.items.len() >= n {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(ok.is_ok(), "{n} exchanges were not committed in time");
    }
}

/// Builds a request message from raw head bytes and a body.
pub fn request_message(raw: &[u8], body: Body) -> Message {
    let mut parser = Parser::request(&ParserConfig::default());
    let Progress::Head(head) = parser.feed(raw).unwrap().event else {
        panic!("the request head did not parse");
    };
    Message::http1_raw(&head, body, ConnectionInfo::new("upstream", 80))
}

/// Builds a response message from raw head bytes and a body.
pub fn response_message(raw: &[u8], body: Body) -> Message {
    let mut parser = Parser::response(
        &ParserConfig::default(),
        ResponseContext::for_method(b"GET"),
    );
    let Progress::Head(head) = parser.feed(raw).unwrap().event else {
        panic!("the response head did not parse");
    };
    Message::http1_raw(&head, body, ConnectionInfo::new("upstream", 80))
}

/// A raw loopback upstream that answers every connection with a fixed
/// response and records the bytes it received.
pub struct Upstream {
    pub addr: SocketAddr,
    pub port: u16,
    pub received: Arc<std::sync::Mutex<Vec<u8>>>,
}

/// Spawns a raw upstream that replies to each connection with `response`.
pub async fn spawn_upstream(response: Vec<u8>) -> Upstream {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();
    let received = Arc::new(std::sync::Mutex::new(Vec::new()));
    let rec = Arc::clone(&received);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let rec = Arc::clone(&rec);
            let response = response.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                // Read the request head (until the blank line) then reply.
                loop {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            rec.lock().unwrap().extend_from_slice(&buf[..n]);
                            if rec.lock().unwrap().windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                    }
                }
                let _ = sock.write_all(&response).await;
                let _ = sock.flush().await;
            });
        }
    });
    Upstream {
        addr,
        port,
        received,
    }
}
