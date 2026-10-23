//! Reading the SNI from a client's first TLS record without consuming it.

use std::time::Duration;

use tokio::net::TcpStream;

use crate::ProxyError;

/// The largest TLS record: a 5-byte header and a 2^14-byte payload.
pub(crate) const MAX_RECORD: usize = 5 + (1 << 14);
/// Longest server name accepted (a DNS name's limit).
const MAX_NAME: usize = 253;

/// Why [`parse_sni`] could not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SniError {
    /// The record is longer than the bytes given; read more.
    #[error("the TLS record is incomplete")]
    Incomplete,
    /// The bytes are not a TLS handshake record carrying a ClientHello.
    #[error("not a TLS ClientHello")]
    NotClientHello,
    /// A length or field inside the ClientHello is inconsistent.
    #[error("malformed TLS ClientHello")]
    Malformed,
}

/// A bounds-checked cursor; running off the end is [`SniError::Malformed`].
struct Rd<'a>(&'a [u8]);

impl<'a> Rd<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], SniError> {
        if n > self.0.len() {
            return Err(SniError::Malformed);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<usize, SniError> {
        Ok(usize::from(self.take(1)?[0]))
    }

    fn u16(&mut self) -> Result<usize, SniError> {
        let b = self.take(2)?;
        Ok(usize::from(u16::from_be_bytes([b[0], b[1]])))
    }

    fn u24(&mut self) -> Result<usize, SniError> {
        let b = self.take(3)?;
        Ok((usize::from(b[0]) << 16) | (usize::from(b[1]) << 8) | usize::from(b[2]))
    }

    /// Reads a length of `width` bytes, then that many bytes.
    fn vec(&mut self, width: usize) -> Result<Rd<'a>, SniError> {
        let n = match width {
            1 => self.u8()?,
            _ => self.u16()?,
        };
        Ok(Rd(self.take(n)?))
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Reads the `server_name` from the ClientHello in `client_hello_record`,
/// the first TLS record a client sends.
///
/// Every length in the input is checked against the bytes present, so any
/// input returns rather than panics. Returns `Ok(None)` when the hello has
/// no `server_name` extension (or no `host_name` entry in it), and also
/// when the ClientHello is fragmented across records, since the rest of
/// it is not in this record. The name must be 1..=253 printable ASCII
/// bytes; anything else is [`SniError::Malformed`].
///
/// # Errors
/// [`SniError::Incomplete`] when fewer bytes than the record header
/// declares are given; [`SniError::NotClientHello`] for a non-handshake
/// record or another handshake type; [`SniError::Malformed`] for a length
/// that does not fit.
pub fn parse_sni(client_hello_record: &[u8]) -> Result<Option<String>, SniError> {
    let rec = client_hello_record;
    match rec.first() {
        None => return Err(SniError::Incomplete),
        Some(0x16) => {}
        Some(_) => return Err(SniError::NotClientHello),
    }
    if rec.len() >= 2 && rec[1] != 0x03 {
        return Err(SniError::NotClientHello);
    }
    if rec.len() < 5 {
        return Err(SniError::Incomplete);
    }
    let len = usize::from(u16::from_be_bytes([rec[3], rec[4]]));
    if len == 0 || 5 + len > MAX_RECORD {
        return Err(SniError::Malformed);
    }
    if rec.len() < 5 + len {
        return Err(SniError::Incomplete);
    }
    let mut body = Rd(&rec[5..5 + len]);
    if body.u8()? != 0x01 {
        return Err(SniError::NotClientHello);
    }
    let hs_len = body.u24()?;
    if hs_len > body.0.len() {
        // The hello continues in the next record.
        return Ok(None);
    }
    let mut hello = Rd(body.take(hs_len)?);
    hello.take(2)?; // legacy_version
    hello.take(32)?; // random
    hello.vec(1)?; // legacy_session_id
    hello.vec(2)?; // cipher_suites
    hello.vec(1)?; // legacy_compression_methods
    if hello.is_empty() {
        return Ok(None);
    }
    let mut exts = hello.vec(2)?;
    while !exts.is_empty() {
        let ty = exts.u16()?;
        let data = exts.vec(2)?;
        if ty == 0x0000 {
            return server_name(data);
        }
    }
    Ok(None)
}

/// Parses a `server_name` extension body and returns its `host_name`.
fn server_name(mut data: Rd<'_>) -> Result<Option<String>, SniError> {
    let mut list = data.vec(2)?;
    while !list.is_empty() {
        let ty = list.u8()?;
        let name = list.vec(2)?.0;
        if ty == 0 {
            if name.is_empty() || name.len() > MAX_NAME || !name.iter().all(u8::is_ascii_graphic) {
                return Err(SniError::Malformed);
            }
            let name = std::str::from_utf8(name).map_err(|_| SniError::Malformed)?;
            return Ok(Some(name.to_string()));
        }
    }
    Ok(None)
}

/// Peeks (without consuming) the client's first TLS record and returns its
/// SNI, retrying until the whole record has arrived or `timeout` passes.
pub(crate) async fn peek_sni(
    tcp: &TcpStream,
    timeout: Duration,
) -> Result<Option<String>, ProxyError> {
    let mut buf = vec![0u8; MAX_RECORD];
    let peek = async {
        let mut last = 0;
        loop {
            let n = tcp.peek(&mut buf).await.map_err(ProxyError::Io)?;
            if n == 0 {
                return Err(ProxyError::Protocol(
                    "client closed before its ClientHello".into(),
                ));
            }
            match parse_sni(&buf[..n]) {
                Ok(name) => return Ok(name),
                Err(SniError::Incomplete) => {
                    // peek returns at once while the same bytes are
                    // buffered; wait a little for more rather than spin.
                    if n == last {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    last = n;
                }
                Err(e) => return Err(ProxyError::Protocol(e.to_string())),
            }
        }
    };
    tokio::time::timeout(timeout, peek)
        .await
        .map_err(|_| ProxyError::Timeout("TLS ClientHello"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use rustls::{ClientConfig, ClientConnection, RootCertStore};
    use rustls_pki_types::ServerName;

    fn client_hello(sni: Option<&str>) -> Vec<u8> {
        let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut cfg = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        cfg.enable_sni = sni.is_some();
        let name = ServerName::try_from(sni.unwrap_or("example.com").to_string()).unwrap();
        let mut conn = ClientConnection::new(Arc::new(cfg), name).unwrap();
        let mut out = Vec::new();
        while conn.wants_write() {
            conn.write_tls(&mut out).unwrap();
        }
        out
    }

    /// A TLS 1.2-style ClientHello record with the given extensions block
    /// (`None` = no extensions field at all).
    fn hand_built(extensions: Option<&[u8]>) -> Vec<u8> {
        let mut hello = vec![0x03, 0x03];
        hello.extend_from_slice(&[0x11; 32]); // random
        hello.push(0); // session id
        hello.extend_from_slice(&[0x00, 0x02, 0x13, 0x01]); // one suite
        hello.extend_from_slice(&[0x01, 0x00]); // null compression
        if let Some(ext) = extensions {
            hello.extend_from_slice(&u16::try_from(ext.len()).unwrap().to_be_bytes());
            hello.extend_from_slice(ext);
        }
        let mut hs = vec![0x01];
        hs.extend_from_slice(&u32::try_from(hello.len()).unwrap().to_be_bytes()[1..]);
        hs.extend_from_slice(&hello);
        let mut rec = vec![0x16, 0x03, 0x01];
        rec.extend_from_slice(&u16::try_from(hs.len()).unwrap().to_be_bytes());
        rec.extend_from_slice(&hs);
        rec
    }

    fn sni_ext(name: &[u8]) -> Vec<u8> {
        let mut entry = vec![0x00];
        entry.extend_from_slice(&u16::try_from(name.len()).unwrap().to_be_bytes());
        entry.extend_from_slice(name);
        let mut list = u16::try_from(entry.len()).unwrap().to_be_bytes().to_vec();
        list.extend_from_slice(&entry);
        let mut ext = vec![0x00, 0x00];
        ext.extend_from_slice(&u16::try_from(list.len()).unwrap().to_be_bytes());
        ext.extend_from_slice(&list);
        ext
    }

    #[test]
    fn parses_sni_from_real_client_hello() {
        let rec = client_hello(Some("www.example.test"));
        assert_eq!(parse_sni(&rec), Ok(Some("www.example.test".to_string())));
    }

    #[test]
    fn no_sni_returns_none() {
        // rustls with SNI disabled.
        assert_eq!(parse_sni(&client_hello(None)), Ok(None));
        // No extensions block at all.
        assert_eq!(parse_sni(&hand_built(None)), Ok(None));
        // Extensions, but no server_name (an empty ALPN-type extension).
        assert_eq!(
            parse_sni(&hand_built(Some(&[0x00, 0x10, 0x00, 0x00]))),
            Ok(None)
        );
        // A hand-built one with SNI, for contrast.
        assert_eq!(
            parse_sni(&hand_built(Some(&sni_ext(b"a.test")))),
            Ok(Some("a.test".to_string()))
        );
    }

    #[test]
    fn truncated_is_incomplete() {
        let rec = client_hello(Some("example.test"));
        for cut in [0, 1, 4, 5, 40, rec.len() - 1] {
            assert_eq!(
                parse_sni(&rec[..cut]),
                Err(SniError::Incomplete),
                "cut {cut}"
            );
        }
    }

    #[test]
    fn not_a_client_hello_is_rejected() {
        assert_eq!(
            parse_sni(&[0x17, 0x03, 0x03, 0x00, 0x01, 0x00]),
            Err(SniError::NotClientHello)
        );
        assert_eq!(
            parse_sni(b"GET / HTTP/1.1\r\n"),
            Err(SniError::NotClientHello)
        );
        // A handshake record holding a ServerHello (type 2).
        let mut rec = hand_built(None);
        rec[5] = 0x02;
        assert_eq!(parse_sni(&rec), Err(SniError::NotClientHello));
    }

    #[test]
    fn lying_lengths_are_malformed_not_panics() {
        // Extensions length larger than the hello.
        let mut rec = hand_built(Some(&sni_ext(b"a.test")));
        let ext_len_at = 5 + 4 + 2 + 32 + 1 + 4 + 2;
        rec[ext_len_at] = 0xff;
        assert_eq!(parse_sni(&rec), Err(SniError::Malformed));
        // Server-name entry length past its list.
        let mut ext = sni_ext(b"a.test");
        ext[7] = 0x40;
        assert_eq!(parse_sni(&hand_built(Some(&ext))), Err(SniError::Malformed));
        // Empty and non-ASCII names.
        assert_eq!(
            parse_sni(&hand_built(Some(&sni_ext(b"")))),
            Err(SniError::Malformed)
        );
        assert_eq!(
            parse_sni(&hand_built(Some(&sni_ext("é.test".as_bytes())))),
            Err(SniError::Malformed)
        );
        // Every prefix and every single-byte corruption: no panic.
        let rec = client_hello(Some("example.test"));
        for cut in 0..rec.len() {
            let _ = parse_sni(&rec[..cut]);
        }
        for i in 0..rec.len() {
            let mut bad = rec.clone();
            bad[i] ^= 0xff;
            let _ = parse_sni(&bad);
        }
    }
}
