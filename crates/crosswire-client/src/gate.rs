//! The destination gate (spec section 11.2).
//!
//! The gate decides on resolved addresses, so the spelling of the host
//! (`localhost`, `127.1`, `2130706433`, `::ffff:127.0.0.1`, a DNS name that
//! points at loopback) cannot get around it. IPv6 transition forms that carry
//! an IPv4 address (IPv4-mapped, SIIT, IPv4-compatible, NAT64, 6to4, Teredo) are
//! unwrapped by [`canonical`] first, and the rules run on both the unwrapped
//! address and the original.
//!
//! Two classes of refusal exist:
//!
//! - **Always refused**, whatever the allowlist says: unspecified
//!   (`0.0.0.0/8`, `::`), multicast (`224.0.0.0/4`, `ff00::/8`), broadcast
//!   (`255.255.255.255`), the cloud metadata addresses and host names, and the
//!   server's own listener ports on any local address (loopback, link-local,
//!   or one a UDP bind probe shows this host owns).
//! - **Allowlistable** per exact `(host, port)`: loopback (`127.0.0.0/8`,
//!   `::1`) and link-local (`169.254.0.0/16`, `fe80::/10`).
//!
//! Private ranges (RFC 1918, CGNAT `100.64.0.0/10`, ULA `fc00::/7`, the
//! benchmarking range `198.18.0.0/15`) are legitimate internal targets and are
//! allowed.

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};

/// Why the gate refused a destination.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GateRefusal {
    /// A loopback address (`127.0.0.0/8`, `::1`). Allowlistable.
    #[error("loopback address {0}")]
    Loopback(IpAddr),
    /// A link-local address (`169.254.0.0/16`, `fe80::/10`). Allowlistable.
    #[error("link-local address {0}")]
    LinkLocal(IpAddr),
    /// A cloud metadata service address. Never allowlistable.
    #[error("cloud metadata address {0}")]
    Metadata(IpAddr),
    /// A cloud metadata service host name (`metadata.google.internal`,
    /// `metadata`), refused before resolution. Never allowlistable.
    #[error("cloud metadata host name {0}")]
    MetadataName(String),
    /// One of this server's own listener ports on a local address. Never
    /// allowlistable.
    #[error("own listener port {0}")]
    OwnListener(u16),
    /// Name resolution produced no address, so there is nothing to check.
    #[error("no addresses resolved")]
    Unresolved,
    /// A multicast address (`224.0.0.0/4`, `ff00::/8`). Never allowlistable.
    #[error("multicast address {0}")]
    Multicast(IpAddr),
    /// The limited broadcast address `255.255.255.255`. Never allowlistable.
    #[error("broadcast address {0}")]
    Broadcast(IpAddr),
    /// An unspecified address (`0.0.0.0/8`, `::`). Never allowlistable.
    #[error("unspecified address {0}")]
    Unspecified(IpAddr),
}

/// Refuses dangerous destinations after resolution.
///
/// Always refused: unspecified, multicast, broadcast, cloud metadata addresses
/// and host names, and the own listener ports on any local address. Refused
/// unless the exact `(host, port)` is allowlisted: loopback and link-local.
/// IPv6 transition forms are unwrapped with [`canonical`] first.
#[derive(Clone, Debug, Default)]
pub struct DestinationGate {
    allow: HashSet<(String, u16)>,
    own_ports: Vec<u16>,
}

fn norm_host(host: &str) -> String {
    host.trim_end_matches('.').to_ascii_lowercase()
}

fn v4_from(hi: u16, lo: u16) -> Ipv4Addr {
    Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo))
}

/// Unwraps an IPv6 transition form to the IPv4 address it carries, so it gets
/// the IPv4 rules. Any other address is returned unchanged.
///
/// Handled forms: IPv4-mapped `::ffff:a.b.c.d`, SIIT IPv4-translated
/// `::ffff:0:a.b.c.d`, IPv4-compatible `::a.b.c.d` (`::/96`, except `::` and
/// `::1`), NAT64 `64:ff9b::/96` and local-use NAT64 `64:ff9b:1::/48` (last 32
/// bits), 6to4 `2002::/16` (bits 16 to 47) and Teredo `2001:0::/32` (last 32 bits,
/// XOR `0xffffffff`).
#[must_use]
pub fn canonical(ip: IpAddr) -> IpAddr {
    let IpAddr::V6(v6) = ip else {
        return ip;
    };
    if let Some(v4) = v6.to_ipv4_mapped() {
        return IpAddr::V4(v4);
    }
    let s = v6.segments();
    let v4 = match s {
        [0, 0, 0, 0, 0, 0, hi, lo] if !(hi == 0 && lo <= 1) => v4_from(hi, lo),
        [0, 0, 0, 0, 0xffff, 0, hi, lo] => v4_from(hi, lo),
        [0x0064, 0xff9b, 0, 0, 0, 0, hi, lo] => v4_from(hi, lo),
        [0x0064, 0xff9b, 0x0001, _, _, _, hi, lo] => v4_from(hi, lo),
        [0x2002, hi, lo, ..] => v4_from(hi, lo),
        [0x2001, 0, _, _, _, _, hi, lo] => Ipv4Addr::from(!u32::from(v4_from(hi, lo))),
        _ => return ip,
    };
    IpAddr::V4(v4)
}

const AWS_V6_METADATA: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254);
const GCP_V6_METADATA: Ipv6Addr = Ipv6Addr::new(0xfd20, 0x00ce, 0, 0, 0, 0, 0, 0x0254);
const EKS_V6_POD_IDENTITY: Ipv6Addr = Ipv6Addr::new(0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0023);

/// Cloud credential and metadata endpoints: AWS/GCP/Azure/OpenStack IMDS
/// (`169.254.169.254`), Alibaba (`100.100.100.200`), ECS task credentials
/// (`169.254.170.2`), EKS Pod Identity (`169.254.170.23`) and the Azure wire
/// server (`168.63.129.16`).
const V4_METADATA: [Ipv4Addr; 5] = [
    Ipv4Addr::new(169, 254, 169, 254),
    Ipv4Addr::new(100, 100, 100, 200),
    Ipv4Addr::new(169, 254, 170, 2),
    Ipv4Addr::new(169, 254, 170, 23),
    Ipv4Addr::new(168, 63, 129, 16),
];

/// AWS IMDS and EKS Pod Identity over IPv6, and the GCP metadata server.
const V6_METADATA: [Ipv6Addr; 3] = [AWS_V6_METADATA, EKS_V6_POD_IDENTITY, GCP_V6_METADATA];

/// Host names of cloud metadata services, compared after [`norm_host`].
const METADATA_NAMES: [&str; 2] = ["metadata.google.internal", "metadata"];

fn is_metadata(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => V4_METADATA.contains(&v4),
        IpAddr::V6(v6) => V6_METADATA.contains(&v6),
    }
}

fn is_unspecified(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.octets()[0] == 0,
        IpAddr::V6(v6) => v6.is_unspecified(),
    }
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

fn is_link_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_link_local(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// The refusals no allowlist entry exempts.
fn hard_refusal(ip: IpAddr) -> Option<GateRefusal> {
    if is_unspecified(ip) {
        Some(GateRefusal::Unspecified(ip))
    } else if ip.is_multicast() {
        Some(GateRefusal::Multicast(ip))
    } else if ip == IpAddr::V4(Ipv4Addr::BROADCAST) {
        Some(GateRefusal::Broadcast(ip))
    } else if is_metadata(ip) {
        Some(GateRefusal::Metadata(ip))
    } else {
        None
    }
}

/// The refusals an allowlisted `(host, port)` exempts.
fn soft_refusal(ip: IpAddr) -> Option<GateRefusal> {
    if is_loopback(ip) {
        Some(GateRefusal::Loopback(ip))
    } else if is_link_local(ip) {
        Some(GateRefusal::LinkLocal(ip))
    } else {
        None
    }
}

/// Whether this host owns `ip`: binding a UDP socket to a specific address
/// only succeeds for a local one. The socket is dropped at once and nothing is
/// sent. Any error (including a host that forbids the bind) means "not local".
fn is_local_address(ip: IpAddr) -> bool {
    UdpSocket::bind(SocketAddr::new(ip, 0)).is_ok()
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
    /// and a trailing dot. The entry exempts only loopback and link-local
    /// refusals.
    pub fn allow(&mut self, host: &str, port: u16) {
        self.allow.insert((norm_host(host), port));
    }

    /// Refuses cloud metadata host names before any resolution
    /// (`metadata.google.internal` and the bare label `metadata`, ignoring
    /// case and a trailing dot). Not allowlistable.
    ///
    /// # Errors
    /// [`GateRefusal::MetadataName`] for a metadata host name.
    pub fn check_name(host: &str) -> Result<(), GateRefusal> {
        let h = norm_host(host);
        if METADATA_NAMES.contains(&h.as_str()) {
            return Err(GateRefusal::MetadataName(h));
        }
        Ok(())
    }

    /// Checks every resolved address of `requested_host:port`.
    ///
    /// The host name is checked first ([`DestinationGate::check_name`]). An
    /// empty address list is refused as [`GateRefusal::Unresolved`]: nothing
    /// to check is never allowed. Per address the order is: canonicalise,
    /// always-refused classes, the own-port rule, then the allowlistable
    /// classes unless `(requested_host, port)` is allowlisted.
    ///
    /// # Errors
    /// The first [`GateRefusal`] found.
    pub fn check(
        &self,
        requested_host: &str,
        port: u16,
        resolved: &[IpAddr],
    ) -> Result<(), GateRefusal> {
        Self::check_name(requested_host)?;
        if resolved.is_empty() {
            return Err(GateRefusal::Unresolved);
        }
        let allowed = self.allow.contains(&(norm_host(requested_host), port));
        for &raw in resolved {
            let ip = canonical(raw);
            // The rules run on the unwrapped address and, for a transition
            // form, on the original IPv6 too.
            let forms: &[IpAddr] = if ip == raw { &[ip] } else { &[ip, raw] };
            if let Some(r) = forms.iter().find_map(|&a| hard_refusal(a)) {
                return Err(r);
            }
            // Unspecified already returned above. Link-local counts as ours:
            // the probe cannot bind `fe80::` without a scope id.
            if self.own_ports.contains(&port)
                && forms
                    .iter()
                    .any(|&a| is_loopback(a) || is_link_local(a) || is_local_address(a))
            {
                return Err(GateRefusal::OwnListener(port));
            }
            if allowed {
                continue;
            }
            if let Some(r) = forms.iter().find_map(|&a| soft_refusal(a)) {
                return Err(r);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::is_local_address;

    #[test]
    fn local_address_probe() {
        assert!(is_local_address("127.0.0.1".parse().unwrap()));
        assert!(!is_local_address("192.0.2.1".parse().unwrap()));
    }
}
