//! Connected-client discovery.
//!
//! Sources, merged by MAC:
//! 1. Kernel neighbor table (rtnetlink) on the layer-3 interface: MAC + IP.
//! 2. In Bridge mode, the bridge forwarding database restricts that list to
//!    MACs learned on the Wi-Fi port (otherwise the whole LAN would show up).
//! 3. The `leh-shaper` helper, when installed: the name each device sends in
//!    its DHCP request (Bridge mode) or NM's root-only lease file (Router mode).
//! 4. Reverse DNS: NM's dnsmasq in Router mode, the bridge's gateway (the
//!    upstream router, which hands out the addresses) in Bridge mode.
//! 5. The dnsmasq lease file, when readable (root-only on many distros).
//!
//! In Bridge mode this computer never talks to the Wi-Fi clients, so they
//! have no neighbor entry (no IP, so no reverse DNS). A one-off UDP probe of
//! the subnet makes the kernel resolve them.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use futures_util::{StreamExt, TryStreamExt, future::join_all};
use leh_model::clients::{self, Client};
use leh_model::parse::{dns, leases, route};
use rtnetlink::packet_route::AddressFamily;
use rtnetlink::packet_route::neighbour::{
    NeighbourAddress, NeighbourAttribute, NeighbourMessage, NeighbourState,
};
use tokio::net::UdpSocket;

use crate::shaper_client;

const DNS_TIMEOUT: Duration = Duration::from_millis(400);
/// How long to wait for ARP after probing the subnet.
const SWEEP_SETTLE: Duration = Duration::from_secs(1);
/// Re-probe at most this often while some Wi-Fi client still has no IP.
const SWEEP_EVERY: Duration = Duration::from_secs(60);
/// Larger subnets are not probed (too many neighbor entries).
const SWEEP_MIN_PREFIX: u8 = 23;

/// Where to look for clients.
#[derive(Debug, Clone)]
pub struct ClientScope {
    /// The Wi-Fi interface running the AP.
    pub ap_iface: String,
    /// The interface holding the IP: same as `ap_iface` in Router mode,
    /// the bridge in Bridge mode.
    pub l3_iface: String,
    /// dnsmasq address for hostname lookups (Router mode). In Bridge mode
    /// the bridge's default gateway is used instead.
    pub dns_server: Option<Ipv4Addr>,
    /// System bus, to ask the `leh-shaper` helper for client names. Errors
    /// (helper missing or too old) are ignored.
    pub helper: Option<zbus::Connection>,
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

/// State kept between polls (when the subnet was last probed).
#[derive(Debug, Default)]
pub struct Discovery {
    swept: HashSet<String>,
    last_sweep: Option<Instant>,
}

impl Discovery {
    /// Probe when a new IP-less client shows up, or again after a while.
    fn should_sweep(&mut self, missing: &[String]) -> bool {
        if missing.is_empty() {
            return false;
        }
        let new = missing.iter().any(|m| !self.swept.contains(m));
        let stale = self.last_sweep.is_none_or(|t| t.elapsed() >= SWEEP_EVERY);
        if !(new || stale) {
            return false;
        }
        self.swept.extend(missing.iter().cloned());
        self.last_sweep = Some(Instant::now());
        true
    }
}

/// IPv4 neighbors on `l3_index` in a usable state, as MAC → IP.
async fn neighbour_clients(l3_index: u32) -> Vec<Client> {
    match neighbours(AddressFamily::Inet).await {
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
    }
}

/// Sends one tiny UDP datagram to every host of the subnets so the kernel
/// ARPs for them, then waits for the answers to land in the neighbor table.
async fn sweep(subnets: &[(Ipv4Addr, u8)]) {
    let Ok(sock) = UdpSocket::bind(("0.0.0.0", 0)).await else {
        return;
    };
    let mut sent = 0;
    for &(net, prefix) in subnets {
        for ip in route::hosts(net, prefix, SWEEP_MIN_PREFIX) {
            // Port 9 is "discard"; delivery doesn't matter, only the ARP.
            if sock.send_to(&[0], (ip, 9)).await.is_ok() {
                sent += 1;
            }
        }
    }
    tracing::debug!(sent, "probed subnet for client addresses");
    if sent > 0 {
        tokio::time::sleep(SWEEP_SETTLE).await;
    }
}

/// Names the helper learned, as MAC → (hostname, IP).
async fn helper_names(scope: &ClientScope) -> HashMap<String, (Option<String>, Option<IpAddr>)> {
    let Some(conn) = &scope.helper else {
        return HashMap::new();
    };
    match shaper_client::client_names(conn, &scope.ap_iface).await {
        Ok(map) => map
            .into_iter()
            .map(|(mac, (host, ip))| {
                let host = (!host.is_empty()).then_some(host);
                (mac.to_ascii_lowercase(), (host, ip.parse().ok()))
            })
            .collect(),
        Err(e) => {
            tracing::trace!(error = %e, "helper client names unavailable");
            HashMap::new()
        }
    }
}

/// Lists clients. Never fails: unavailable sources are skipped and logged.
pub async fn list(scope: &ClientScope) -> Vec<Client> {
    list_with(scope, &mut Discovery::default()).await
}

async fn list_with(scope: &ClientScope, disc: &mut Discovery) -> Vec<Client> {
    let Some(l3_index) = ifindex(&scope.l3_iface) else {
        return Vec::new();
    };
    let bridged = scope.l3_iface != scope.ap_iface;
    let routes = if bridged {
        tokio::fs::read_to_string("/proc/net/route")
            .await
            .map(|t| route::parse(&t, &scope.l3_iface))
            .unwrap_or_default()
    } else {
        route::IfaceRoutes::default()
    };

    let mut found = neighbour_clients(l3_index).await;

    if bridged && let Some(ap_index) = ifindex(&scope.ap_iface) {
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

        let missing: Vec<String> = found
            .iter()
            .filter(|c| c.ip.is_none())
            .map(|c| c.mac.clone())
            .collect();
        if disc.should_sweep(&missing) {
            sweep(&routes.subnets).await;
            let ips: HashMap<String, Option<IpAddr>> = neighbour_clients(l3_index)
                .await
                .into_iter()
                .map(|c| (c.mac, c.ip))
                .collect();
            for c in found.iter_mut().filter(|c| c.ip.is_none()) {
                c.ip = ips.get(&c.mac).copied().flatten();
            }
        }
    }

    // The device's own name (from the helper) beats reverse DNS.
    let from_helper = helper_names(scope).await;
    for c in &mut found {
        if let Some((host, ip)) = from_helper.get(&c.mac) {
            c.ip = c.ip.or(*ip);
            c.hostname = host.clone();
        }
    }

    if let Some(server) = scope.dns_server.or(routes.gateway) {
        let wanted: Vec<_> = found.iter().map(|c| (c.ip, c.hostname.is_some())).collect();
        let lookups = wanted
            .into_iter()
            .enumerate()
            .map(|(i, (ip, named))| async move {
                match ip {
                    Some(IpAddr::V4(ip)) if !named => {
                        ptr_lookup(server, ip, 0x4c00 + i as u16).await
                    }
                    _ => None,
                }
            });
        let names = join_all(lookups).await;
        for (c, name) in found.iter_mut().zip(names) {
            if c.hostname.is_none() {
                c.hostname = name;
            }
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
        (scope, None::<Vec<Client>>, Discovery::default()),
        move |(scope, last, mut disc)| async move {
            loop {
                if last.is_some() {
                    tokio::time::sleep(every).await;
                }
                let now = list_with(&scope, &mut disc).await;
                if last.as_ref() != Some(&now) {
                    return Some((now.clone(), (scope, Some(now), disc)));
                }
            }
        },
    )
    .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweeps_for_new_clients_only() {
        let mut d = Discovery::default();
        assert!(!d.should_sweep(&[]));
        assert!(d.should_sweep(&["a".into()]));
        assert!(!d.should_sweep(&["a".into()]), "same client, too soon");
        assert!(d.should_sweep(&["a".into(), "b".into()]), "new client");
    }
}
