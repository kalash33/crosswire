//! The destination gate (spec section 11.2).
//!
//! The gate decides on resolved addresses, so the spelling of the host
//! (`localhost`, `127.1`, `2130706433`, `::ffff:127.0.0.1`, a DNS name that
//! points at loopback) cannot get around it.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Why the gate refused a destination.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GateRefusal {
    /// A loopback (or unspecified) address.
    #[error("loopback address {0}")]
    Loopback(IpAddr),
    /// A link-local address.
    #[error("link-local address {0}")]
    LinkLocal(IpAddr),
    /// A cloud metadata service address.
    #[error("cloud metadata address {0}")]
    Metadata(IpAddr),
    /// One of this server's own listener ports on loopback.
    #[error("own listener port {0}")]
    OwnListener(u16),
}

/// Refuses loopback, link-local, cloud-metadata and own-listener destinations
/// unless a `(host, port)` pair is allowlisted.
///
/// The server's own listener ports are refused on loopback even when the
/// destination is allowlisted.
#[derive(Clone, Debug, Default)]
pub struct DestinationGate {
    allow: HashSet<(String, u16)>,
    own_ports: Vec<u16>,
}

fn norm_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

/// Maps IPv4-mapped IPv6 addresses to IPv4 so they get the IPv4 rules.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    }
}

fn is_loopbackish(ip: IpAddr) -> bool {
    ip.is_loopback() || ip.is_unspecified()
}

const AWS_V6_METADATA: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254);

fn is_metadata(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4 == Ipv4Addr::new(169, 254, 169, 254) || v4 == Ipv4Addr::new(100, 100, 100, 200)
        }
        IpAddr::V6(v6) => v6 == AWS_V6_METADATA,
    }
}

fn is_link_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.octets()[..2] == [169, 254],
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

impl DestinationGate {
    /// Creates a gate that protects the given own-listener ports.
    #[must_use]
    pub fn new(own_ports: Vec<u16>) -> DestinationGate {
        DestinationGate {
            allow: HashSet::new(),
            own_ports,
        }
    }

    /// Allowlists an exact `(host, port)` pair. Host comparison ignores case
    /// and a trailing dot.
    pub fn allow(&mut self, host: &str, port: u16) {
        self.allow.insert((norm_host(host), port));
    }

    /// Checks every resolved address of `requested_host:port`.
    ///
    /// An empty address list is refused: nothing to check is never allowed.
    ///
    /// # Errors
    /// The first [`GateRefusal`] found.
    pub fn check(
        &self,
        requested_host: &str,
        port: u16,
        resolved: &[IpAddr],
    ) -> Result<(), GateRefusal> {
        let allowed = self.allow.contains(&(norm_host(requested_host), port));
        if resolved.is_empty() {
            return Err(GateRefusal::Loopback(IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
        }
        for &raw in resolved {
            let ip = canonical(raw);
            if is_loopbackish(ip) && self.own_ports.contains(&port) {
                return Err(GateRefusal::OwnListener(port));
            }
            if allowed {
                continue;
            }
            if is_loopbackish(ip) {
                return Err(GateRefusal::Loopback(ip));
            }
            if is_metadata(ip) {
                return Err(GateRefusal::Metadata(ip));
            }
            if is_link_local(ip) {
                return Err(GateRefusal::LinkLocal(ip));
            }
        }
        Ok(())
    }
}
