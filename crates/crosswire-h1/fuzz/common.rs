//! Shared fuzz driver: feeds bytes in pieces with the re-present contract and
//! checks the parser's invariants after every call.
#![allow(dead_code)]

use bytes::BytesMut;
use crosswire_core::{Profile, Span};
use crosswire_h1::*;

/// What a run observed; compared between the split and the whole-input run.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub head_bytes: Vec<u8>,
    pub framing: Option<&'static str>,
    pub head_codes: Vec<&'static str>,
    pub body_codes: Vec<&'static str>,
    pub error: Option<String>,
}

fn framing_name(f: &BodyFraming) -> &'static str {
    match f {
        BodyFraming::None => "none",
        BodyFraming::ContentLength(_) => "content_length",
        BodyFraming::Chunked => "chunked",
        BodyFraming::UntilClose => "until_close",
        BodyFraming::Upgrade => "upgrade",
        BodyFraming::Ambiguous { .. } => "ambiguous",
        _ => "unknown",
    }
}

fn in_bounds(s: Span, len: usize) -> bool {
    s.start() <= s.end() && s.range().end <= len
}

fn check_head(m: &RawMessage, input: &[u8], consumed: usize) {
    let head = m.bytes();
    // The head may have been buffered by earlier calls, so only its tail is
    // in this call's input.
    assert!(
        head.ends_with(&input[..consumed]),
        "head bytes do not end with the consumed input"
    );
    let mut out = BytesMut::new();
    write_exact(m, &mut out);
    assert_eq!(&out[..], &head[..], "write_exact is not byte-identical");
    let n = head.len();
    match m.head().start() {
        StartLine::Request {
            method,
            target,
            version,
        } => {
            for s in [*method, *target, *version] {
                assert!(in_bounds(s, n), "start span out of head");
            }
        }
        StartLine::Response {
            version,
            status,
            reason,
        } => {
            for s in [*version, *status, *reason] {
                assert!(in_bounds(s, n), "start span out of head");
            }
        }
        _ => {}
    }
    for h in m.head().headers() {
        for s in [h.name(), h.value(), h.line()] {
            assert!(in_bounds(s, n), "header span out of head");
        }
    }
}

/// Feeds `data` in pieces of `piece` bytes (`usize::MAX` for one feed).
pub fn run(
    resp_ctx: Option<ResponseContext>,
    profile: Profile,
    data: &[u8],
    piece: usize,
) -> Outcome {
    let cfg = ParserConfig::builder().profile(profile).build();
    let mut p = match resp_ctx {
        Some(ctx) => Parser::response(&cfg, ctx),
        None => Parser::request(&cfg),
    };
    let mut out = Outcome {
        head_bytes: vec![],
        framing: None,
        head_codes: vec![],
        body_codes: vec![],
        error: None,
    };
    let mut carry: Vec<u8> = vec![];
    let mut stopped = false;
    let mut pieces = data.chunks(piece.max(1));
    // An empty input still gets one (empty) feed so the parser always runs.
    let mut first = true;
    loop {
        let next: &[u8] = if first && data.is_empty() {
            first = false;
            &[]
        } else {
            first = false;
            match pieces.next() {
                Some(c) => c,
                None => break,
            }
        };
        let mut buf = std::mem::take(&mut carry);
        buf.extend_from_slice(next);
        let mut off = 0;
        let mut settled = false;
        for _ in 0..(buf.len() * 2 + 100) {
            let rest = &buf[off..];
            let step = match p.feed(rest) {
                Ok(s) => s,
                Err(e) => {
                    let s = format!("{e:?}");
                    out.error = Some(s.chars().take_while(|c| c.is_alphanumeric()).collect());
                    stopped = true;
                    settled = true;
                    break;
                }
            };
            assert!(step.consumed <= rest.len(), "consumed > input");
            let mut finished = false;
            match &step.event {
                Progress::NeedMore => finished = true,
                Progress::Head(m) => {
                    check_head(m, rest, step.consumed);
                    if out.framing.is_none() {
                        out.head_bytes = m.bytes().to_vec();
                        out.framing = Some(framing_name(m.head().framing()));
                        let mut c: Vec<&'static str> =
                            m.diagnostics().iter().map(|d| d.code().as_str()).collect();
                        c.sort_unstable();
                        out.head_codes = c;
                    }
                }
                Progress::BodyData { raw, data } => {
                    assert!(in_bounds(*raw, rest.len()), "raw span out of input");
                    assert!(in_bounds(*data, rest.len()), "data span out of input");
                }
                Progress::ChunkExt(s) => assert!(in_bounds(*s, rest.len()), "ext out of input"),
                Progress::Trailers(t) => {
                    for h in t {
                        for s in [h.name(), h.value(), h.line()] {
                            assert!(in_bounds(s, rest.len()), "trailer span out of input");
                        }
                    }
                }
                Progress::Done => {
                    stopped = true;
                    finished = true;
                }
                _ => {}
            }
            off += step.consumed;
            if finished {
                settled = true;
                break;
            }
        }
        assert!(settled, "feed loop did not settle");
        if stopped {
            break;
        }
        carry = buf[off..].to_vec();
    }
    if !stopped {
        // The whole stream has been presented: signal end of input.
        if let Err(e) = p.feed_eof() {
            let s = format!("{e:?}");
            out.error = Some(s.chars().take_while(|c| c.is_alphanumeric()).collect());
        }
    }
    let mut b: Vec<&'static str> = p
        .body_diagnostics()
        .iter()
        .map(|d| d.code().as_str())
        .collect();
    b.sort_unstable();
    b.dedup();
    out.body_codes = b;
    out
}

/// Entry point shared by both targets.
pub fn fuzz(response: bool, data: &[u8]) {
    if data.len() < 3 {
        return;
    }
    let (b0, b1) = (data[0], data[1]);
    let profile = if b0 & 1 == 1 {
        Profile::Strict
    } else {
        Profile::Lenient
    };
    let piece = 1 + usize::from(b0 >> 1) % 64;
    let ctx =
        response.then(|| ResponseContext::for_method(if b1 % 4 == 0 { b"HEAD" } else { b"GET" }));
    let body = &data[2..];
    let split = run(ctx, profile, body, piece);
    let whole = run(ctx, profile, body, usize::MAX);
    assert_eq!(split, whole, "split run differs from whole-input run");
}
