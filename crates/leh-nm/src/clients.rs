//! Connected-client discovery.
//!
//! Sources, merged by MAC:
//! 1. Kernel neighbor table (rtnetlink) on the layer-3 interface: MAC + IP.
//! 2. In Bridge mode, the bridge forwarding database restricts that list to
//!    MACs learned on the Wi-Fi port (otherwise the whole LAN would show up).
//! 3. Reverse DNS against NM's dnsmasq (Router mode): hostname.
//! 4. The dnsmasq lease file, when readable (root-only on many distros).

use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use futures_util::{StreamExt, TryStreamExt, future::join_all};
use leh_model::clients::{self, Client};
use leh_model::parse::{dns, leases};
use rtnetlink::packet_route::AddressFamily;
use rtnetlink::packet_route::neighbour::{
    NeighbourAddress, NeighbourAttribute, NeighbourMessage, NeighbourState,
};
use tokio::net::UdpSocket;

const DNS_TIMEOUT: Duration = Duration::from_millis(400);

/// Where to look for clients.
#[derive(Debug, Clone)]
pub struct ClientScope {
    /// The Wi-Fi interface running the AP.
    pub ap_iface: String,
    /// The interface holding the IP: same as `ap_iface` in Router mode,
    /// the bridge in Bridge mode.
    pub l3_iface: String,
    /// dnsmasq address for hostname lookups (Router mode).
    pub dns_server: Option<Ipv4Addr>,
}

fn ifindex(iface: &str) -> Option<u32> {
    std::fs::read_to_string(format!("/sys/class/net/{iface}/ifindex"))
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn mac_string(bytes: &[u8]) -> Option<String> {
    (bytes.len() == 6).then(|| {
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":")
    })
}

fn attrs(msg: &NeighbourMessage) -> (Option<String>, Option<IpAddr>) {
    let mut mac = None;
    let mut ip = None;
    for a in &msg.attributes {
        match a {
            NeighbourAttribute::LinkLayerAddress(b) => mac = mac_string(b),
            NeighbourAttribute::Destination(NeighbourAddress::Inet(v4)) => {
                ip = Some(IpAddr::V4(*v4))
            }
            _ => {}
        }
    }
    (mac, ip)
}

async fn neighbours(family: AddressFamily) -> std::io::Result<Vec<NeighbourMessage>> {
    let (conn, handle, _) = rtnetlink::new_connection()?;
    let task = tokio::spawn(conn);
    let res = handle
        .neighbours()
        .get()
        .set_address_family(family)
        .execute()
        .try_collect::<Vec<_>>()
        .await
        .map_err(std::io::Error::other);
    task.abort();
    res
}

async fn ptr_lookup(server: Ipv4Addr, ip: Ipv4Addr, id: u16) -> Option<String> {
    let sock = UdpSocket::bind(("0.0.0.0", 0)).await.ok()?;
    sock.send_to(&dns::ptr_query(id, ip), SocketAddr::from((server, 53)))
        .await
        .ok()?;
    let mut buf = [0u8; 512];
    let n = tokio::time::timeout(DNS_TIMEOUT, sock.recv(&mut buf))
        .await
        .ok()?
        .ok()?;
    dns::parse_ptr_response(id, &buf[..n])
}

/// Lists clients. Never fails: unavailable sources are skipped and logged.
pub async fn list(scope: &ClientScope) -> Vec<Client> {
    let Some(l3_index) = ifindex(&scope.l3_iface) else {
        return Vec::new();
    };

    let mut found: Vec<Client> = match neighbours(AddressFamily::Inet).await {
        Ok(msgs) => msgs
            .iter()
            .filter(|m| m.header.ifindex == l3_index)
            .filter(|m| {
                matches!(
                    m.header.state,
                    NeighbourState::Reachable
                        | NeighbourState::Stale
                        | NeighbourState::Delay
                        | NeighbourState::Probe
                )
            })
            .filter_map(|m| {
                let (mac, ip) = attrs(m);
                Some(Client {
                    mac: mac?,
                    ip,
                    hostname: None,
                })
            })
            .collect(),
        Err(e) => {
            tracing::warn!(error = %e, "reading neighbor table failed");
            Vec::new()
        }
    };

    if scope.l3_iface != scope.ap_iface
        && let Some(ap_index) = ifindex(&scope.ap_iface)
    {
        let wifi_macs: HashSet<String> = neighbours(AddressFamily::Bridge)
            .await
            .unwrap_or_default()
            .iter()
            .filter(|m| m.header.ifindex == ap_index && m.header.state != NeighbourState::Permanent)
            .filter_map(|m| attrs(m).0)
            .collect();
        found.retain(|c| wifi_macs.contains(&c.mac));
        // Wi-Fi clients without an IPv4 neighbor entry yet still count.
        for mac in wifi_macs {
            if !found.iter().any(|c| c.mac == mac) {
                found.push(Client {
                    mac,
                    ip: None,
                    hostname: None,
                });
            }
        }
    }

    if let Some(server) = scope.dns_server {
        let ips: Vec<_> = found.iter().map(|c| c.ip).collect();
        let lookups = ips.into_iter().enumerate().map(|(i, ip)| async move {
            match ip {
                Some(IpAddr::V4(ip)) => ptr_lookup(server, ip, 0x4c00 + i as u16).await,
                _ => None,
            }
        });
        for (c, name) in found.iter_mut().zip(join_all(lookups).await) {
            c.hostname = name;
        }
    }

    let lease_path = format!("/var/lib/NetworkManager/dnsmasq-{}.leases", scope.ap_iface);
    let from_leases = match tokio::fs::read_to_string(&lease_path).await {
        Ok(text) => {
            tracing::trace!(path = lease_path, "read lease file");
            leases::parse(&text)
        }
        Err(_) => Vec::new(),
    };
    // Lease entries only fill in hostnames for clients we can see right now.
    let present: HashSet<_> = found.iter().map(|c| c.mac.clone()).collect();
    let extra = from_leases.into_iter().filter(|c| present.contains(&c.mac));

    clients::merge(found.into_iter().chain(extra))
}

/// Polls `list` forever, yielding only when the client set changes.
pub fn watch(scope: ClientScope, every: Duration) -> impl futures_util::Stream<Item = Vec<Client>> {
    futures_util::stream::unfold(
        (scope, None::<Vec<Client>>),
        move |(scope, last)| async move {
            loop {
                if last.is_some() {
                    tokio::time::sleep(every).await;
                }
                let now = list(&scope).await;
                if last.as_ref() != Some(&now) {
                    return Some((now.clone(), (scope, Some(now))));
                }
            }
        },
    )
    .boxed()
}
