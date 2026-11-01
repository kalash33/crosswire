//! Canonicalisation of IPv6 transition forms to the IPv4 they carry.
use crosswire_client::canonical;
use std::net::IpAddr;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

#[test]
fn canonical_unwraps_transition_forms() {
    for (from, to) in [
        // IPv4-mapped.
        ("::ffff:7f00:1", "127.0.0.1"),
        // IPv4-compatible (deprecated).
        ("::7f00:1", "127.0.0.1"),
        // NAT64 well-known prefix.
        ("64:ff9b::a9fe:a9fe", "169.254.169.254"),
        // 6to4.
        ("2002:7f00:1::", "127.0.0.1"),
        ("2002:7f00:0001:1234:5678::1", "127.0.0.1"),
        // Teredo, RFC 4380 section 4 example: client 192.0.2.45.
        ("2001:0:4136:e378:8000:63bf:3fff:fdd2", "192.0.2.45"),
    ] {
        assert_eq!(canonical(ip(from)), ip(to), "{from}");
    }
}

#[test]
fn canonical_leaves_other_addresses_unchanged() {
    for a in [
        "::1",
        "::",
        "2001:db8::1",
        "fe80::1",
        "fd12::1",
        "127.0.0.1",
        "10.0.0.1",
    ] {
        assert_eq!(canonical(ip(a)), ip(a), "{a}");
    }
}
