//! `/proc/net/route` parser: an interface's IPv4 gateway and on-link subnets.
//!
//! Used in Bridge mode, where the upstream router (the bridge's gateway)
//! hands out addresses and usually answers reverse DNS for its clients.

use std::net::Ipv4Addr;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IfaceRoutes {
    pub gateway: Option<Ipv4Addr>,
    /// Directly connected networks as (network address, prefix length).
    pub subnets: Vec<(Ipv4Addr, u8)>,
}

/// The kernel prints each address as the native-endian `u32` of the
/// network-order bytes, so `to_ne_bytes` gives the octets back.
fn addr(hex: &str) -> Option<Ipv4Addr> {
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(Ipv4Addr::from(v.to_ne_bytes()))
}

pub fn parse(text: &str, iface: &str) -> IfaceRoutes {
    let mut out = IfaceRoutes::default();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 8 || f[0] != iface {
            continue;
        }
        let (Some(dest), Some(gw), Some(mask)) = (addr(f[1]), addr(f[2]), addr(f[7])) else {
            continue;
        };
        if dest.is_unspecified() && mask.is_unspecified() {
            if !gw.is_unspecified() && out.gateway.is_none() {
                out.gateway = Some(gw);
            }
        } else if gw.is_unspecified() {
            let prefix = u32::from(mask).leading_ones() as u8;
            if !out.subnets.contains(&(dest, prefix)) {
                out.subnets.push((dest, prefix));
            }
        }
    }
    out
}

/// Host addresses of a subnet (no network/broadcast address); empty for
/// anything larger than `/min_prefix` so a sweep stays small.
pub fn hosts(net: Ipv4Addr, prefix: u8, min_prefix: u8) -> Vec<Ipv4Addr> {
    if !(min_prefix..=30).contains(&prefix) {
        return Vec::new();
    }
    let base = u32::from(net) & (u32::MAX << (32 - prefix));
    let size = 1u32 << (32 - prefix);
    (1..size - 1).map(|i| Ipv4Addr::from(base + i)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(ip: [u8; 4]) -> String {
        format!("{:08X}", u32::from_ne_bytes(ip))
    }

    #[test]
    fn gateway_and_subnet() {
        let text = format!(
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
             lehbr0\t00000000\t{}\t0003\t0\t0\t425\t00000000\t0\t0\t0\n\
             lehbr0\t{}\t00000000\t0001\t0\t0\t425\t{}\t0\t0\t0\n\
             enp0s1\t00000000\t{}\t0003\t0\t0\t100\t00000000\t0\t0\t0\n",
            hex([192, 168, 30, 1]),
            hex([192, 168, 30, 0]),
            hex([255, 255, 255, 0]),
            hex([10, 0, 0, 1]),
        );
        let r = parse(&text, "lehbr0");
        assert_eq!(r.gateway, Some(Ipv4Addr::new(192, 168, 30, 1)));
        assert_eq!(r.subnets, vec![(Ipv4Addr::new(192, 168, 30, 0), 24)]);
        assert_eq!(parse(&text, "wlp2s0"), IfaceRoutes::default());
        assert_eq!(parse("garbage\nx y z", "x"), IfaceRoutes::default());
    }

    #[test]
    fn host_ranges() {
        let h = hosts(Ipv4Addr::new(192, 168, 30, 0), 24, 23);
        assert_eq!(h.len(), 254);
        assert_eq!(h[0], Ipv4Addr::new(192, 168, 30, 1));
        assert_eq!(h[253], Ipv4Addr::new(192, 168, 30, 254));
        assert!(hosts(Ipv4Addr::new(10, 0, 0, 0), 8, 23).is_empty());
        assert!(hosts(Ipv4Addr::new(10, 0, 0, 0), 32, 23).is_empty());
    }
}
