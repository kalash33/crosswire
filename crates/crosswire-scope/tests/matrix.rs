//! Table-driven coverage of the scope evaluator.

use std::collections::BTreeSet;

use crosswire_scope::{
    Compiled, HostPattern, Input, PairVerdict, PathMatch, Rule, Scheme, Scope, Verdict,
};

/// A compact test DSL: rules separated by `;`, each `<disp> <kind> <arg> [opts]`.
///
/// - `disp`: `in` or `ex`
/// - `kind` + `arg`: `exact <h>`, `sfx <apex>`, `apex <apex>`, `rx <pat>`,
///   `ip <addr>`, `cidr <net>`
/// - `opts`: `port=80[,443]`, `scheme=http|https`, `path=/p`, `pathrx=<pat>`
fn scope_from(spec: &str) -> Scope {
    let mut scope = Scope::default();
    for raw in spec.split(';') {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let disp = it.next().expect("disposition");
        let kind = it.next().expect("kind");
        let arg = it.next().expect("arg");
        let host = match kind {
            "exact" => HostPattern::Exact(arg.to_string()),
            "sfx" => HostPattern::Suffix {
                suffix: arg.to_string(),
                also_apex: false,
            },
            "apex" => HostPattern::Suffix {
                suffix: arg.to_string(),
                also_apex: true,
            },
            "rx" => HostPattern::Regex(arg.to_string()),
            "ip" => HostPattern::Ip(arg.parse().expect("ip")),
            "cidr" => HostPattern::Cidr(arg.parse().expect("cidr")),
            other => panic!("bad kind {other}"),
        };
        let mut rule = Rule {
            host,
            ports: None,
            scheme: None,
            path: None,
        };
        for opt in it {
            if let Some(v) = opt.strip_prefix("port=") {
                let set: BTreeSet<u16> = v.split(',').map(|p| p.parse().expect("port")).collect();
                rule.ports = Some(set);
            } else if let Some(v) = opt.strip_prefix("scheme=") {
                rule.scheme = Some(match v {
                    "http" => Scheme::Http,
                    "https" => Scheme::Https,
                    other => panic!("bad scheme {other}"),
                });
            } else if let Some(v) = opt.strip_prefix("path=") {
                rule.path = Some(PathMatch::Prefix(v.to_string()));
            } else if let Some(v) = opt.strip_prefix("pathrx=") {
                rule.path = Some(PathMatch::Regex(v.to_string()));
            } else {
                panic!("bad opt {opt}");
            }
        }
        match disp {
            "in" => scope.include.push(rule),
            "ex" => scope.exclude.push(rule),
            other => panic!("bad disposition {other}"),
        }
    }
    scope
}

/// Parse a test URL into its host (brackets kept for IPv6), port, scheme, path.
fn input_parts(url: &str) -> (String, u16, Scheme, String) {
    let (scheme_s, rest) = url.split_once("://").expect("scheme");
    let scheme = match scheme_s {
        "http" => Scheme::Http,
        "https" => Scheme::Https,
        other => panic!("bad scheme {other}"),
    };
    let default_port = match scheme {
        Scheme::Http => 80,
        Scheme::Https => 443,
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], rest[i..].to_string()),
        None => (rest, "/".to_string()),
    };
    let (host, port) = if let Some(after_open) = authority.strip_prefix('[') {
        let end = after_open.find(']').expect("closing bracket");
        let host = format!("[{}]", &after_open[..end]);
        let tail = &after_open[end + 1..];
        let port = tail
            .strip_prefix(':')
            .map(|p| p.parse().expect("port"))
            .unwrap_or(default_port);
        (host, port)
    } else if let Some((h, p)) = authority.rsplit_once(':') {
        (h.to_string(), p.parse().expect("port"))
    } else {
        (authority.to_string(), default_port)
    };
    (host, port, scheme, path)
}

fn verdict(spec: &str, url: &str) -> Verdict {
    let compiled: Compiled = scope_from(spec).compile().expect("compile");
    let (host, port, scheme, path) = input_parts(url);
    let input = Input {
        host: &host,
        port,
        scheme,
        path: &path,
    };
    compiled.evaluate(&input)
}

const CASES: &[(&str, &str, Verdict)] = &[
    // Exact host.
    ("in exact example.com", "http://example.com/", Verdict::In),
    ("in exact example.com", "http://other.com/", Verdict::Out),
    (
        "in exact example.com",
        "http://sub.example.com/",
        Verdict::Out,
    ),
    ("in exact example.com", "http://EXAMPLE.COM/", Verdict::In),
    ("in exact example.com", "http://example.com./", Verdict::In),
    ("in exact 1.2.3.4", "http://1.2.3.4/", Verdict::Out),
    // Suffix without apex.
    ("in sfx example.com", "http://a.example.com/", Verdict::In),
    ("in sfx example.com", "http://a.b.example.com/", Verdict::In),
    ("in sfx example.com", "http://example.com/", Verdict::Out),
    ("in sfx example.com", "http://notexample.com/", Verdict::Out),
    ("in sfx example.com", "http://xexample.com/", Verdict::Out),
    ("in sfx example.com", "http://A.Example.Com/", Verdict::In),
    // Suffix with apex (Review Focus 3: suffix_rule_apex_and_ip_literals).
    ("in apex example.com", "http://example.com/", Verdict::In),
    ("in apex example.com", "http://a.example.com/", Verdict::In),
    ("in apex example.com", "http://evil.com/", Verdict::Out),
    ("in apex example.com", "http://1.2.3.4/", Verdict::Out),
    ("in apex example.com", "http://EXAMPLE.COM./", Verdict::In),
    // Regex host.
    (
        r"in rx ^(api|www)\.example\.com$",
        "http://api.example.com/",
        Verdict::In,
    ),
    (
        r"in rx ^(api|www)\.example\.com$",
        "http://www.example.com/",
        Verdict::In,
    ),
    (
        r"in rx ^(api|www)\.example\.com$",
        "http://mail.example.com/",
        Verdict::Out,
    ),
    (
        r"in rx ^(api|www)\.example\.com$",
        "http://5.6.7.8/",
        Verdict::Out,
    ),
    // IP literal vs name rule.
    ("in ip 1.2.3.4", "http://1.2.3.4/", Verdict::In),
    ("in ip 1.2.3.4", "http://1.2.3.5/", Verdict::Out),
    ("in ip 1.2.3.4", "http://example.com/", Verdict::Out),
    // CIDR.
    ("in cidr 10.0.0.0/8", "http://10.0.0.1/", Verdict::In),
    ("in cidr 10.0.0.0/8", "http://10.255.255.255/", Verdict::In),
    ("in cidr 10.0.0.0/8", "http://11.0.0.0/", Verdict::Out),
    ("in cidr 10.0.0.0/8", "http://example.com/", Verdict::Out),
    // Port sets.
    (
        "in exact example.com port=80,443",
        "http://example.com/",
        Verdict::In,
    ),
    (
        "in exact example.com port=80,443",
        "https://example.com/",
        Verdict::In,
    ),
    (
        "in exact example.com port=80,443",
        "http://example.com:8080/",
        Verdict::Out,
    ),
    (
        "in exact example.com port=8080",
        "http://example.com:8080/",
        Verdict::In,
    ),
    // Scheme.
    (
        "in exact example.com scheme=https",
        "https://example.com/",
        Verdict::In,
    ),
    (
        "in exact example.com scheme=https",
        "http://example.com/",
        Verdict::Out,
    ),
    (
        "in exact example.com scheme=http",
        "http://example.com/",
        Verdict::In,
    ),
    // Path prefix.
    (
        "in exact example.com path=/api",
        "http://example.com/api/users",
        Verdict::In,
    ),
    (
        "in exact example.com path=/api",
        "http://example.com/other",
        Verdict::Out,
    ),
    (
        "in exact example.com path=/api",
        "http://example.com/api",
        Verdict::In,
    ),
    // Path regex.
    (
        r"in exact example.com pathrx=^/api/v[0-9]+/",
        "http://example.com/api/v2/users",
        Verdict::In,
    ),
    (
        r"in exact example.com pathrx=^/api/v[0-9]+/",
        "http://example.com/api/vX/users",
        Verdict::Out,
    ),
    // Exclude beats include.
    (
        "in apex example.com;ex exact admin.example.com",
        "http://admin.example.com/",
        Verdict::Out,
    ),
    (
        "in apex example.com;ex exact admin.example.com",
        "http://user.example.com/",
        Verdict::In,
    ),
    (
        "in apex example.com;ex sfx internal.example.com",
        "http://db.internal.example.com/",
        Verdict::Out,
    ),
    (
        "in cidr 10.0.0.0/8;ex ip 10.0.0.1",
        "http://10.0.0.1/",
        Verdict::Out,
    ),
    (
        "in cidr 10.0.0.0/8;ex ip 10.0.0.1",
        "http://10.0.0.2/",
        Verdict::In,
    ),
    // No include match -> Out.
    ("ex exact example.com", "http://example.com/", Verdict::Out),
    ("in exact a.com", "http://b.com/", Verdict::Out),
    // Empty scope -> Empty.
    ("", "http://example.com/", Verdict::Empty),
    // Bracketed IPv6.
    ("in ip ::1", "http://[::1]/", Verdict::In),
    ("in cidr ::1/128", "http://[::1]/", Verdict::In),
    (
        "in cidr 2001:db8::/32",
        "http://[2001:db8::1]/",
        Verdict::In,
    ),
    (
        "in cidr 2001:db8::/32",
        "http://[2001:dead::1]/",
        Verdict::Out,
    ),
    ("in exact example.com", "http://[::1]/", Verdict::Out),
];

#[test]
fn matrix() {
    assert!(CASES.len() >= 40, "matrix must have >= 40 rows");
    for (i, (spec, url, want)) in CASES.iter().enumerate() {
        let got = verdict(spec, url);
        assert_eq!(got, *want, "row {i}: spec={spec:?} url={url:?}");
    }
}

#[test]
fn pair_both_in() {
    let c = scope_from("in apex example.com").compile().unwrap();
    let conn = Input {
        host: "example.com",
        port: 443,
        scheme: Scheme::Https,
        path: "/",
    };
    let msg = Input {
        host: "example.com",
        port: 443,
        scheme: Scheme::Https,
        path: "/",
    };
    assert_eq!(c.evaluate_pair(&conn, &msg), PairVerdict::In);
}

#[test]
fn pair_connection_out_message_in_is_mismatch() {
    let c = scope_from("in exact example.com").compile().unwrap();
    let conn = Input {
        host: "other.com",
        port: 80,
        scheme: Scheme::Http,
        path: "/",
    };
    let msg = Input {
        host: "example.com",
        port: 80,
        scheme: Scheme::Http,
        path: "/",
    };
    assert_eq!(c.evaluate_pair(&conn, &msg), PairVerdict::HostMismatch);
}

#[test]
fn pair_connection_in_message_out_is_mismatch() {
    let c = scope_from("in exact example.com").compile().unwrap();
    let conn = Input {
        host: "example.com",
        port: 80,
        scheme: Scheme::Http,
        path: "/",
    };
    let msg = Input {
        host: "other.com",
        port: 80,
        scheme: Scheme::Http,
        path: "/",
    };
    assert_eq!(c.evaluate_pair(&conn, &msg), PairVerdict::HostMismatch);
}

#[test]
fn pair_both_out() {
    let c = scope_from("in exact example.com").compile().unwrap();
    let conn = Input {
        host: "a.com",
        port: 80,
        scheme: Scheme::Http,
        path: "/",
    };
    let msg = Input {
        host: "b.com",
        port: 80,
        scheme: Scheme::Http,
        path: "/",
    };
    assert_eq!(c.evaluate_pair(&conn, &msg), PairVerdict::Out);
}

#[test]
fn pair_empty_scope() {
    let c = scope_from("").compile().unwrap();
    let conn = Input {
        host: "example.com",
        port: 80,
        scheme: Scheme::Http,
        path: "/",
    };
    let msg = Input {
        host: "example.com",
        port: 80,
        scheme: Scheme::Http,
        path: "/",
    };
    assert_eq!(c.evaluate_pair(&conn, &msg), PairVerdict::Empty);
}
