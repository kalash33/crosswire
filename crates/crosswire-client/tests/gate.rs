//! Destination gate: refusal is decided on resolved addresses.
use crosswire_client::{DestinationGate, GateRefusal};

fn ip(s: &str) -> std::net::IpAddr {
    s.parse().unwrap()
}

/// A predicate on the refusal a case expects.
type Want = fn(&GateRefusal) -> bool;

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

#[test]
fn allowlist_cannot_exempt_metadata_multicast_or_own_ports() {
    let mut g = DestinationGate::new(vec![9000]);
    g.allow("x.example", 80);
    g.allow("x.example", 9000);
    g.allow("metadata.google.internal", 80);
    let cases: [(&str, u16, Want); 9] = [
        ("169.254.169.254", 80, |r| {
            matches!(r, GateRefusal::Metadata(_))
        }),
        ("100.100.100.200", 80, |r| {
            matches!(r, GateRefusal::Metadata(_))
        }),
        ("fd00:ec2::254", 80, |r| {
            matches!(r, GateRefusal::Metadata(_))
        }),
        ("fd20:ce::254", 80, |r| {
            matches!(r, GateRefusal::Metadata(_))
        }),
        ("224.0.0.1", 80, |r| matches!(r, GateRefusal::Multicast(_))),
        ("ff02::1", 80, |r| matches!(r, GateRefusal::Multicast(_))),
        ("255.255.255.255", 80, |r| {
            matches!(r, GateRefusal::Broadcast(_))
        }),
        ("0.0.0.0", 80, |r| matches!(r, GateRefusal::Unspecified(_))),
        ("127.0.0.1", 9000, |r| {
            matches!(r, GateRefusal::OwnListener(9000))
        }),
    ];
    for (a, port, want) in cases {
        let got = g.check("x.example", port, &[ip(a)]).unwrap_err();
        assert!(want(&got), "{a}:{port} -> {got:?}");
    }
    // Loopback and link-local stay allowlistable.
    assert!(g.check("x.example", 80, &[ip("127.0.0.1")]).is_ok());
    assert!(g.check("x.example", 80, &[ip("169.254.1.1")]).is_ok());
    assert!(g.check("x.example", 80, &[ip("fe80::1")]).is_ok());
    assert!(g.check("x.example", 80, &[ip("::1")]).is_ok());
    // An allowlisted metadata name is still refused.
    assert!(matches!(
        g.check("metadata.google.internal", 80, &[ip("203.0.113.5")]),
        Err(GateRefusal::MetadataName(_))
    ));
}

#[test]
fn allowlisted_name_resolving_to_metadata_is_refused() {
    let mut g = DestinationGate::new(vec![]);
    g.allow("evil.example", 80);
    assert!(matches!(
        g.check("evil.example", 80, &[ip("203.0.113.5"), ip("169.254.169.254")]),
        Err(GateRefusal::Metadata(a)) if a == ip("169.254.169.254")
    ));
}

#[test]
fn metadata_host_names_are_refused_before_resolution() {
    let mut g = DestinationGate::new(vec![]);
    for h in [
        "metadata.google.internal",
        "METADATA.Google.Internal.",
        "metadata",
        "Metadata.",
    ] {
        g.allow(h, 80);
        assert!(
            matches!(
                DestinationGate::check_name(h),
                Err(GateRefusal::MetadataName(_))
            ),
            "{h}"
        );
        assert!(
            matches!(
                g.check(h, 80, &[ip("203.0.113.5")]),
                Err(GateRefusal::MetadataName(_))
            ),
            "{h}"
        );
    }
    for h in [
        "metadata.example",
        "google.internal",
        "example.com",
        "x-metadata",
    ] {
        assert!(DestinationGate::check_name(h).is_ok(), "{h}");
    }
    let e = DestinationGate::check_name("metadata").unwrap_err();
    assert_eq!(e.to_string(), "cloud metadata host name metadata");
}

#[test]
fn empty_resolution_is_unresolved() {
    let g = DestinationGate::new(vec![]);
    let e = g.check("empty.example", 443, &[]).unwrap_err();
    assert_eq!(e, GateRefusal::Unresolved);
    assert_eq!(e.to_string(), "no addresses resolved");
}

#[test]
fn refuses_new_ranges_in_every_spelling() {
    let g = DestinationGate::new(vec![]);
    let cases: [(&str, Want); 11] = [
        ("0.1.2.3", |r| matches!(r, GateRefusal::Unspecified(_))),
        ("224.0.0.1", |r| matches!(r, GateRefusal::Multicast(_))),
        ("239.255.255.250", |r| {
            matches!(r, GateRefusal::Multicast(_))
        }),
        ("255.255.255.255", |r| {
            matches!(r, GateRefusal::Broadcast(_))
        }),
        ("ff02::1", |r| matches!(r, GateRefusal::Multicast(_))),
        ("fd20:ce::254", |r| matches!(r, GateRefusal::Metadata(_))),
        ("::ffff:e000:1", |r| matches!(r, GateRefusal::Multicast(_))),
        ("64:ff9b::ffff:ffff", |r| {
            matches!(r, GateRefusal::Broadcast(_))
        }),
        ("2002:a9fe:a9fe::", |r| {
            matches!(r, GateRefusal::Metadata(_))
        }),
        ("::7f00:1", |r| matches!(r, GateRefusal::Loopback(_))),
        ("2001:0:4136:e378:8000:63bf:80ff:fffe", |r| {
            matches!(r, GateRefusal::Loopback(_))
        }),
    ];
    for (a, want) in cases {
        let got = g.check("x.example", 443, &[ip(a)]).unwrap_err();
        assert!(want(&got), "{a} -> {got:?}");
    }
    assert_eq!(
        GateRefusal::Multicast(ip("224.0.0.1")).to_string(),
        "multicast address 224.0.0.1"
    );
    assert_eq!(
        GateRefusal::Broadcast(ip("255.255.255.255")).to_string(),
        "broadcast address 255.255.255.255"
    );
    assert_eq!(
        GateRefusal::Unspecified(ip("0.0.0.0")).to_string(),
        "unspecified address 0.0.0.0"
    );
}

#[test]
fn private_and_cgnat_and_ula_are_allowed() {
    let g = DestinationGate::new(vec![]);
    for a in [
        "10.1.2.3",
        "172.16.0.1",
        "192.168.1.1",
        "100.64.0.1",
        "fd12::1",
        "198.18.0.1",
        "2001:db8::1",
        "93.184.216.34",
    ] {
        assert!(g.check("x.example", 443, &[ip(a)]).is_ok(), "{a}");
    }
}

/// The machine's first non-loopback IPv4, found by "connecting" a UDP socket
/// (no packet is sent) and reading the local address the route picked.
fn first_non_loopback_ipv4() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let a = s.local_addr().ok()?.ip();
    (!a.is_loopback() && !a.is_unspecified()).then_some(a)
}

#[test]
fn own_port_on_local_interface_is_refused() {
    let Some(local) = first_non_loopback_ipv4() else {
        eprintln!("skipping: no non-loopback IPv4 address on this host");
        return;
    };
    let mut g = DestinationGate::new(vec![9000]);
    g.allow("lan.example", 9000);
    assert!(
        matches!(
            g.check("lan.example", 9000, &[local]),
            Err(GateRefusal::OwnListener(9000))
        ),
        "{local}"
    );
    // The same address on another port is a normal (private) destination.
    assert!(g.check("lan.example", 9001, &[local]).is_ok(), "{local}");
    // A documentation address is not ours, so the own port is fine there.
    assert!(g.check("doc.example", 9000, &[ip("192.0.2.1")]).is_ok());
}

#[test]
fn more_credential_endpoints_are_metadata() {
    let mut g = DestinationGate::new(vec![]);
    g.allow("x.example", 80);
    for a in [
        // ECS task credentials.
        "169.254.170.2",
        // EKS Pod Identity agent.
        "169.254.170.23",
        "fd00:ec2::23",
        // Azure wire server / IMDS host.
        "168.63.129.16",
        // The same through transition forms.
        "::ffff:a9fe:aa02",
        "64:ff9b:1::a83f:8110",
    ] {
        let got = g.check("x.example", 80, &[ip(a)]).unwrap_err();
        assert!(matches!(got, GateRefusal::Metadata(_)), "{a} -> {got:?}");
    }
}

#[test]
fn allowlisted_name_resolving_to_ecs_credentials_is_refused() {
    let mut g = DestinationGate::new(vec![]);
    g.allow("evil.example", 80);
    assert!(matches!(
        g.check("evil.example", 80, &[ip("203.0.113.5"), ip("169.254.170.2")]),
        Err(GateRefusal::Metadata(a)) if a == ip("169.254.170.2")
    ));
}

#[test]
fn own_port_on_link_local_is_refused_even_when_allowlisted() {
    let mut g = DestinationGate::new(vec![9000]);
    g.allow("ll.example", 9000);
    for a in ["fe80::1", "169.254.1.1"] {
        assert!(
            matches!(
                g.check("ll.example", 9000, &[ip(a)]),
                Err(GateRefusal::OwnListener(9000))
            ),
            "{a}"
        );
    }
    // Another port on the same allowlisted link-local address is fine.
    g.allow("ll.example", 9001);
    assert!(g.check("ll.example", 9001, &[ip("fe80::1")]).is_ok());
}
