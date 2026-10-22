//! Shared test helpers: the golden corpus loader.
#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;

/// The `ctx` block of a response case.
#[derive(Debug, Deserialize)]
pub struct Ctx {
    pub method: String,
    #[serde(default)]
    pub is_connect: bool,
}

/// The expectations stored in `<name>.expect.json`.
#[derive(Debug, Deserialize)]
pub struct Expect {
    pub kind: String,
    pub ctx: Option<Ctx>,
    pub framing: String,
    pub resolved: Option<String>,
    pub rule: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub pipelined: bool,
    pub body: Option<String>,
    pub body_hex: Option<String>,
    pub error: Option<String>,
    pub trailers: Option<Vec<[String; 2]>>,
    pub known_bug: Option<String>,
}

/// The `<name>.gen.json` descriptor of a generated case.
#[derive(Debug, Deserialize)]
struct Gen {
    kind: String,
    head: String,
    body_byte: u8,
    body_len: usize,
    framing: String,
    #[serde(default)]
    diagnostics: Vec<String>,
}

/// One corpus case.
#[derive(Debug)]
pub struct Case {
    pub name: String,
    pub input: Vec<u8>,
    pub expect: Expect,
}

pub fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../corpus/h1/cases")
}

/// Loads every case, sorted by name.
pub fn load_cases() -> Vec<Case> {
    let dir = corpus_dir();
    let mut cases = vec![];
    let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {dir:?}: {e}"));
    for entry in entries {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(name) = file.strip_suffix(".msg") {
            let input = fs::read(&path).unwrap();
            let json_path = dir.join(format!("{name}.expect.json"));
            let json = fs::read_to_string(&json_path)
                .unwrap_or_else(|e| panic!("case {name}: missing expect.json: {e}"));
            let expect: Expect = serde_json::from_str(&json)
                .unwrap_or_else(|e| panic!("case {name}: bad expect.json: {e}"));
            cases.push(Case {
                name: name.to_owned(),
                input,
                expect,
            });
        } else if let Some(name) = file.strip_suffix(".gen.json") {
            let g: Gen = serde_json::from_str(&fs::read_to_string(&path).unwrap())
                .unwrap_or_else(|e| panic!("case {name}: bad gen.json: {e}"));
            let mut input = g.head.into_bytes();
            input.resize(input.len() + g.body_len, g.body_byte);
            cases.push(Case {
                name: name.to_owned(),
                input,
                expect: Expect {
                    kind: g.kind,
                    ctx: None,
                    framing: g.framing,
                    resolved: None,
                    rule: None,
                    diagnostics: g.diagnostics,
                    pipelined: false,
                    body: None,
                    body_hex: None,
                    error: None,
                    trailers: None,
                    known_bug: None,
                },
            });
        }
    }
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}
