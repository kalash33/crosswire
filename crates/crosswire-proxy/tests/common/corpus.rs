//! Loads the crosswire-h1 golden corpus (`corpus/h1/cases`) for the proxy
//! relay golden. The h1 crate's loader lives in its own test crate and
//! cannot be imported, so this is a small reader of the same files: the
//! `.msg` + `.expect.json` pairs and the `.gen.json` generated cases. The
//! schema is documented in `corpus/h1/README.md`.

use std::fs;
use std::path::PathBuf;

use crosswire_h1::{ParseLimits, Parser, ParserConfig, Progress, ResponseContext};
use serde_json::Value;

/// One corpus case, with the expectations the proxy golden uses.
#[derive(Debug, Clone)]
pub struct Case {
    pub name: String,
    /// The wire bytes.
    pub input: Vec<u8>,
    /// `request` or `response`.
    pub kind: String,
    /// Responses: the method of the request being answered.
    pub method: String,
    pub is_connect: bool,
    /// `None` only for an error case that fails before the head.
    pub framing: Option<String>,
    /// Head plus body diagnostic codes, sorted and deduplicated.
    pub diagnostics: Vec<String>,
    pub pipelined: bool,
    /// The decoded payload (`body`, or `body_hex` decoded).
    pub body: Option<Vec<u8>>,
    /// The fatal `ParseError`, in snake_case.
    pub error: Option<String>,
    pub known_bug: Option<String>,
    /// The parser configuration the case asks for (defaults + `limits`).
    pub config: ParserConfig,
    /// Whether `limits` overrides the default parser limits.
    pub custom_limits: bool,
}

impl Case {
    /// A parser for this case's message.
    pub fn parser(&self) -> Parser {
        match self.kind.as_str() {
            "request" => Parser::request(&self.config),
            _ => {
                let mut ctx = ResponseContext::for_method(self.method.as_bytes());
                ctx.is_connect = self.is_connect;
                Parser::response(&self.config, ctx)
            }
        }
    }
}

pub fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/h1/cases")
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("body_hex"))
        .collect()
}

fn config(limits: Option<&Value>) -> ParserConfig {
    let mut b = ParseLimits::builder();
    let get = |k: &str| limits.and_then(|l| l.get(k)).and_then(Value::as_u64);
    if let Some(v) = get("head_size") {
        b = b.head_size(u32::try_from(v).unwrap());
    }
    if let Some(v) = get("max_headers") {
        b = b.max_headers(u16::try_from(v).unwrap());
    }
    if let Some(v) = get("max_line") {
        b = b.max_line(u32::try_from(v).unwrap());
    }
    if let Some(v) = get("max_chunk_size_line") {
        b = b.max_chunk_size_line(u32::try_from(v).unwrap());
    }
    if let Some(v) = get("max_trailer_size") {
        b = b.max_trailer_size(u32::try_from(v).unwrap());
    }
    if let Some(v) = get("max_chunks") {
        b = b.max_chunks(v);
    }
    if let Some(v) = get("max_body_diagnostics") {
        b = b.max_body_diagnostics(u16::try_from(v).unwrap());
    }
    ParserConfig::builder().limits(b.build()).build()
}

fn from_json(name: &str, input: Vec<u8>, e: &Value) -> Case {
    let s = |k: &str| e.get(k).and_then(Value::as_str).map(str::to_owned);
    let ctx = e.get("ctx");
    let mut diagnostics: Vec<String> = e
        .get("diagnostics")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|v| v.as_str().unwrap().to_owned()).collect())
        .unwrap_or_default();
    diagnostics.sort();
    diagnostics.dedup();
    let limits = e.get("limits");
    Case {
        name: name.to_owned(),
        input,
        kind: s("kind").expect("kind"),
        method: ctx
            .and_then(|c| c.get("method"))
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .to_owned(),
        is_connect: ctx
            .and_then(|c| c.get("is_connect"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        framing: s("framing"),
        diagnostics,
        pipelined: e.get("pipelined").and_then(Value::as_bool).unwrap_or(false),
        body: s("body")
            .map(String::into_bytes)
            .or_else(|| s("body_hex").map(|h| hex(&h))),
        error: s("error"),
        known_bug: s("known_bug"),
        config: config(limits),
        custom_limits: limits.is_some(),
    }
}

/// Loads every case, sorted by name.
pub fn load_cases() -> Vec<Case> {
    let dir = corpus_dir();
    let mut cases = vec![];
    for entry in fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {dir:?}: {e}")) {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(name) = file.strip_suffix(".msg") {
            let json = fs::read_to_string(dir.join(format!("{name}.expect.json"))).unwrap();
            let e: Value = serde_json::from_str(&json).unwrap();
            cases.push(from_json(name, fs::read(&path).unwrap(), &e));
        } else if let Some(name) = file.strip_suffix(".gen.json") {
            // `head` followed by `body_len` copies of `body_byte`.
            let g: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
            let mut input = g["head"].as_str().unwrap().as_bytes().to_vec();
            let len = usize::try_from(g["body_len"].as_u64().unwrap()).unwrap();
            let byte = u8::try_from(g["body_byte"].as_u64().unwrap()).unwrap();
            input.resize(input.len() + len, byte);
            let mut case = from_json(name, input, &g);
            case.body = Some(vec![byte; len]);
            cases.push(case);
        }
    }
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

/// What parsing one message out of some bytes found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// Length of the head.
    pub head_len: usize,
    /// End of the message (head plus framed body).
    pub end: usize,
    /// The decoded payload.
    pub payload: Vec<u8>,
}

/// Parses the first message of `bytes` with `parser`, signalling end of
/// input when the bytes run out (an until-close body ends there). `None`
/// when the bytes do not hold a complete, valid message.
pub fn parse_one(mut parser: Parser, bytes: &[u8]) -> Option<Parsed> {
    let (mut off, mut head_len, mut payload) = (0, None, Vec::new());
    for _ in 0..(bytes.len() * 2 + 100) {
        let rest = &bytes[off..];
        let step = parser.feed(rest).ok()?;
        match &step.event {
            Progress::Head(_) => head_len = Some(off + step.consumed),
            Progress::BodyData { data, .. } => payload.extend_from_slice(&rest[data.range()]),
            Progress::Done => {
                off += step.consumed;
                return Some(Parsed {
                    head_len: head_len?,
                    end: off,
                    payload,
                });
            }
            Progress::NeedMore => {
                off += step.consumed;
                let eof = parser.feed_eof().ok()?;
                return matches!(eof.event, Progress::Done).then_some(Parsed {
                    head_len: head_len?,
                    end: off,
                    payload,
                });
            }
            _ => {}
        }
        off += step.consumed;
    }
    None
}
