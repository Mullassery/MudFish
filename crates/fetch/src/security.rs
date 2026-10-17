use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use tokio::net::lookup_host;
use url::Url;

/// Resolves DNS names the same way the OS resolver would, then drops any
/// resolved address that points at loopback, private, link-local, or
/// multicast space. This is what actually stops SSRF via DNS rebinding
/// (a public hostname that resolves to `127.0.0.1` or a cloud metadata
/// IP) — a hostname-string blocklist alone cannot catch that, since the
/// dangerous IP only appears after resolution.
#[derive(Debug, Default, Clone, Copy)]
pub struct SsrfGuardResolver;

impl Resolve for SsrfGuardResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            let resolved = lookup_host((host.as_str(), 0)).await?;
            let allowed: Vec<SocketAddr> = resolved.filter(|a| !is_blocked_ip(a.ip())).collect();
            if allowed.is_empty() {
                return Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!("all addresses for '{host}' are blocked by network policy"),
                ))
                    as Box<dyn std::error::Error + Send + Sync>);
            }
            let iter: Addrs = Box::new(allowed.into_iter());
            Ok(iter)
        })
    }
}

/// True if `ip` falls in a range that must never be reached from a crawl
/// (loopback, RFC1918/ULA private space, link-local — which covers the
/// `169.254.169.254` cloud metadata endpoint, multicast, unspecified).
pub fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_blocked_v4(v4),
        IpAddr::V6(v6) => is_blocked_v6(v6),
    }
}

/// URL-level SSRF pre-check. [`SsrfGuardResolver`] only ever sees hosts that
/// actually go through DNS resolution; reqwest skips the resolver entirely
/// when the URL host is already an IP literal, so `http://127.0.0.1/` and
/// `http://169.254.169.254/` would otherwise slip straight past it. This
/// rejects an IP-literal host that lands in blocked space, and is cheap
/// enough to run on the initial request and on every redirect hop.
pub fn ip_literal_is_blocked(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(v4)) => is_blocked_v4(v4),
        Some(url::Host::Ipv6(v6)) => is_blocked_v6(v6),
        _ => false,
    }
}

fn is_blocked_v4(ip: Ipv4Addr) -> bool {
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_multicast()
        || ip.is_documentation()
        || is_cgnat_v4(ip)
        || is_benchmark_v4(ip)
        || is_reserved_v4(ip)
}

/// `100.64.0.0/10` — RFC 6598 carrier-grade NAT shared address space, used
/// internally by many cloud/container networks.
fn is_cgnat_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (64..=127).contains(&o[1])
}

/// `198.18.0.0/15` — RFC 2544 network-interconnect benchmarking space.
fn is_benchmark_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 198 && (o[1] == 18 || o[1] == 19)
}

/// Other IANA-reserved v4 blocks that must never be reachable from a crawl:
/// `240.0.0.0/4` (reserved for future use, includes `255.255.255.255`),
/// `0.0.0.0/8` ("this network"), `192.0.0.0/24` (IETF protocol assignments)
/// and the deprecated `192.88.99.0/24` 6to4 relay anycast prefix.
fn is_reserved_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] >= 240
        || o[0] == 0
        || (o[0] == 192 && o[1] == 0 && o[2] == 0)
        || (o[0] == 192 && o[1] == 88 && o[2] == 99)
}

fn is_blocked_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_blocked_v4(v4);
    }
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || ip.is_unicast_link_local()
        || is_unique_local_v6(ip)
        || is_documentation_v6(ip)
}

/// `2001:db8::/32` — IPv6 documentation space.
fn is_documentation_v6(ip: Ipv6Addr) -> bool {
    ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8
}

/// `fc00::/7` (ULA) has no stable std helper as of this Rust version.
fn is_unique_local_v6(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xfe00) == 0xfc00
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_loopback_and_private_v4() {
        assert!(is_blocked_ip("127.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("10.0.0.5".parse().unwrap()));
        assert!(is_blocked_ip("192.168.1.1".parse().unwrap()));
        assert!(is_blocked_ip("172.16.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_cloud_metadata_endpoint() {
        assert!(is_blocked_ip("169.254.169.254".parse().unwrap()));
    }

    #[test]
    fn allows_public_v4() {
        assert!(!is_blocked_ip("93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn blocks_loopback_v6_and_ula() {
        assert!(is_blocked_ip("::1".parse().unwrap()));
        assert!(is_blocked_ip("fc00::1".parse().unwrap()));
        assert!(is_blocked_ip("fe80::1".parse().unwrap()));
    }

    #[test]
    fn blocks_v4_mapped_private_v6() {
        assert!(is_blocked_ip("::ffff:127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn allows_public_v6() {
        assert!(!is_blocked_ip(
            "2606:2800:220:1:248:1893:25c8:1946".parse().unwrap()
        ));
    }

    #[test]
    fn blocks_cgnat_benchmark_and_reserved_v4() {
        assert!(is_blocked_ip("100.64.0.1".parse().unwrap()));
        assert!(is_blocked_ip("100.127.255.254".parse().unwrap()));
        assert!(is_blocked_ip("198.18.0.1".parse().unwrap()));
        assert!(is_blocked_ip("198.19.255.255".parse().unwrap()));
        assert!(is_blocked_ip("240.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("192.0.0.1".parse().unwrap()));
        assert!(is_blocked_ip("192.88.99.1".parse().unwrap()));
        assert!(is_blocked_ip("0.1.2.3".parse().unwrap()));
    }

    #[test]
    fn allows_public_v4_just_outside_blocked_ranges() {
        assert!(!is_blocked_ip("100.63.255.255".parse().unwrap()));
        assert!(!is_blocked_ip("100.128.0.1".parse().unwrap()));
        assert!(!is_blocked_ip("198.20.0.1".parse().unwrap()));
    }

    #[test]
    fn blocks_documentation_v6() {
        assert!(is_blocked_ip("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn ip_literal_hosts_are_checked() {
        assert!(ip_literal_is_blocked(
            &Url::parse("http://127.0.0.1/").unwrap()
        ));
        assert!(ip_literal_is_blocked(
            &Url::parse("http://169.254.169.254/latest/meta-data/").unwrap()
        ));
        assert!(ip_literal_is_blocked(&Url::parse("http://[::1]/").unwrap()));
        assert!(ip_literal_is_blocked(
            &Url::parse("http://[::ffff:127.0.0.1]/").unwrap()
        ));
        assert!(!ip_literal_is_blocked(
            &Url::parse("http://93.184.216.34/").unwrap()
        ));
        // Hostnames are the resolver's job, not this pre-check's.
        assert!(!ip_literal_is_blocked(
            &Url::parse("http://example.com/").unwrap()
        ));
    }
}
