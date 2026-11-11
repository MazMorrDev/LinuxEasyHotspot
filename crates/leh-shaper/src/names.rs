//! Client names, for devices the unprivileged app can't name by itself.
//!
//! - **Bridge mode:** the upstream router hands out addresses, so the only
//!   place this computer sees a device's own name is the DHCP request it
//!   broadcasts through the bridge (option 12). We listen for those on UDP
//!   port 67 of the bridge. The socket uses `SO_REUSEADDR`/`SO_REUSEPORT`
//!   (as dnsmasq does with `--bind-interfaces`), and we never answer, so a
//!   DHCP server on this machine is unaffected.
//! - **Router mode:** NM's dnsmasq lease file, which is root-only on
//!   Ubuntu/Mint.
//!
//! Devices that were already connected before the helper started are named
//! when they next send a request (reconnect or lease rebinding).

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use leh_model::parse::{dhcp, leases};
use leh_model::tc;
use tokio::task::JoinHandle;

const DHCP_SERVER_PORT: u16 = 67;
/// A bridge sees the whole LAN's requests; don't grow without bound.
const MAX_ENTRIES: usize = 512;

/// MAC → (hostname, IPv4 or "") as sent over D-Bus.
pub type Names = HashMap<String, (String, String)>;

type Seen = Arc<Mutex<HashMap<String, (Option<String>, Option<Ipv4Addr>)>>>;

/// The bridge `ap_iface` is a port of, if any (Bridge mode).
pub fn bridge_of(ap_iface: &str) -> Option<String> {
    let link = std::fs::read_link(format!("/sys/class/net/{ap_iface}/master")).ok()?;
    let name = link.file_name()?.to_str()?.to_owned();
    tc::valid_iface(&name).then_some(name)
}

fn open(bridge: &str) -> std::io::Result<tokio::net::UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s.set_reuse_address(true)?;
    s.set_reuse_port(true)?;
    s.bind_device(Some(bridge.as_bytes()))?;
    s.set_nonblocking(true)?;
    s.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, DHCP_SERVER_PORT).into())?;
    tokio::net::UdpSocket::from_std(s.into())
}

/// Listens for DHCP requests on a bridge until dropped.
pub struct Snooper {
    pub ap_iface: String,
    pub bridge: String,
    seen: Seen,
    task: JoinHandle<()>,
}

impl Snooper {
    pub fn start(ap_iface: String, bridge: String) -> std::io::Result<Self> {
        let sock = open(&bridge)?;
        let seen = Seen::default();
        let map = seen.clone();
        let task = tokio::spawn(async move {
            let mut buf = [0u8; 1500];
            loop {
                let n = match sock.recv(&mut buf).await {
                    Ok(n) => n,
                    Err(e) => {
                        tracing::debug!(error = %e, "dhcp snoop recv");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                };
                let Some(s) = dhcp::parse_request(&buf[..n]) else {
                    continue;
                };
                tracing::debug!(mac = s.mac, hostname = ?s.hostname, ip = ?s.ip, "dhcp request");
                let mut m = map
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if m.len() >= MAX_ENTRIES && !m.contains_key(&s.mac) {
                    continue;
                }
                let e = m.entry(s.mac).or_default();
                if s.hostname.is_some() {
                    e.0 = s.hostname;
                }
                if s.ip.is_some() {
                    e.1 = s.ip;
                }
            }
        });
        tracing::info!(ap_iface, bridge, "listening for client names");
        Ok(Self {
            ap_iface,
            bridge,
            seen,
            task,
        })
    }

    pub fn names(&self) -> Names {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .map(|(mac, (host, ip))| {
                (
                    mac.clone(),
                    (
                        host.clone().unwrap_or_default(),
                        ip.map(|i| i.to_string()).unwrap_or_default(),
                    ),
                )
            })
            .collect()
    }
}

impl Drop for Snooper {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Router mode: NM's dnsmasq leases for the hotspot interface.
pub async fn lease_names(ap_iface: &str) -> Names {
    let path = format!("/var/lib/NetworkManager/dnsmasq-{ap_iface}.leases");
    let text = tokio::fs::read_to_string(path).await.unwrap_or_default();
    leases::parse(&text)
        .into_iter()
        .map(|c| {
            (
                c.mac,
                (
                    c.hostname.unwrap_or_default(),
                    c.ip.map(|i| i.to_string()).unwrap_or_default(),
                ),
            )
        })
        .collect()
}
