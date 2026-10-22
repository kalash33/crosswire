//! Destination gate: refusal is decided on resolved addresses.
use crosswire_client::{DestinationGate, GateRefusal};

fn ip(s: &str) -> std::net::IpAddr {
    s.parse().unwrap()
}

#[test]
fn gate_refuses_after_resolution() {
    let g = DestinationGate::new(vec![8080]);
    for a in [
        "127.0.0.1",
        "::1",
        "169.254.169.254",
        "fd00:ec2::254",
        "100.100.100.200",
        "169.254.1.1",
    ] {
        assert!(g.check("whatever.example", 443, &[ip(a)]).is_err(), "{a}");
    }
    assert!(g.check("example.com", 443, &[ip("93.184.216.34")]).is_ok());
    let mut g = DestinationGate::new(vec![8080]);
    g.allow("localhost", 3000);
    assert!(g.check("localhost", 3000, &[ip("127.0.0.1")]).is_ok());
    assert!(matches!(
        g.check("localhost", 8080, &[ip("127.0.0.1")]),
        Err(GateRefusal::OwnListener(8080))
    ));
    assert!(g.check("localhost", 3001, &[ip("127.0.0.1")]).is_err());
}

#[test]
fn every_resolved_address_is_checked() {
    let g = DestinationGate::new(vec![]);
    let mixed = [ip("93.184.216.34"), ip("127.0.0.1")];
    assert!(matches!(
        g.check("rebind.example", 443, &mixed),
        Err(GateRefusal::Loopback(_))
    ));
    assert!(g.check("empty.example", 443, &[]).is_err());
}

#[test]
fn spellings_that_resolve_to_refused_ranges_are_refused() {
    let g = DestinationGate::new(vec![]);
    for a in [
        "::ffff:127.0.0.1",
        "::ffff:169.254.169.254",
        "0.0.0.0",
        "::",
        "127.255.255.254",
        "fe80::1",
        "febf::1",
    ] {
        assert!(g.check("x.example", 443, &[ip(a)]).is_err(), "{a}");
    }
    assert!(matches!(
        g.check("x.example", 443, &[ip("169.254.169.254")]),
        Err(GateRefusal::Metadata(_))
    ));
    assert!(matches!(
        g.check("x.example", 443, &[ip("fe80::1")]),
        Err(GateRefusal::LinkLocal(_))
    ));
}

#[test]
fn own_ports_are_refused_on_loopback_even_when_allowlisted() {
    let mut g = DestinationGate::new(vec![9000]);
    g.allow("127.0.0.1", 9000);
    g.allow("::ffff:127.0.0.1", 9000);
    assert!(matches!(
        g.check("127.0.0.1", 9000, &[ip("127.0.0.1")]),
        Err(GateRefusal::OwnListener(9000))
    ));
    assert!(matches!(
        g.check("127.0.0.1", 9000, &[ip("::ffff:127.0.0.1")]),
        Err(GateRefusal::OwnListener(9000))
    ));
    // The same port on a public address is not our listener.
    assert!(g.check("example.com", 9000, &[ip("93.184.216.34")]).is_ok());
}

#[test]
fn allowlist_is_exact_and_case_insensitive_on_host() {
    let mut g = DestinationGate::new(vec![]);
    g.allow("Dev.Local.", 3000);
    assert!(g.check("dev.local", 3000, &[ip("127.0.0.1")]).is_ok());
    assert!(g.check("other.local", 3000, &[ip("127.0.0.1")]).is_err());
    assert!(g.check("dev.local", 3001, &[ip("127.0.0.1")]).is_err());
}
