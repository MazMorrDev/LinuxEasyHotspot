//! Device discovery.

use zbus::Connection;
use zbus::zvariant::OwnedObjectPath;

use crate::proxies::{ActiveConnectionProxy, DeviceProxy, NetworkManagerProxy, WirelessProxy};

const NM_DEVICE_TYPE_ETHERNET: u32 = 1;
const NM_DEVICE_TYPE_WIFI: u32 = 2;
const NM_DEVICE_TYPE_MODEM: u32 = 8;

const NM_WIFI_DEVICE_CAP_AP: u32 = 0x40;
const NM_WIFI_DEVICE_CAP_FREQ_VALID: u32 = 0x100;
const NM_WIFI_DEVICE_CAP_FREQ_5GHZ: u32 = 0x400;

/// `NMDeviceState` values we care about.
pub mod dev_state {
    pub const UNMANAGED: u32 = 10;
    pub const UNAVAILABLE: u32 = 20;
    pub const DISCONNECTED: u32 = 30;
    pub const ACTIVATED: u32 = 100;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceKind {
    Wifi {
        supports_ap: bool,
        supports_5ghz: bool,
    },
    Ethernet,
    /// Mobile broadband (GSM/LTE/CDMA).
    Modem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub path: OwnedObjectPath,
    pub iface: String,
    pub kind: DeviceKind,
    pub state: u32,
    /// `/` when nothing is active.
    pub active_connection: OwnedObjectPath,
    /// Name (`connection.id`) of the active profile, e.g. "Wired connection 1".
    pub connection_name: Option<String>,
    /// The active connection owns the IPv4 default route (carries internet).
    pub is_default: bool,
}

impl DeviceInfo {
    pub fn is_ap_capable(&self) -> bool {
        matches!(
            self.kind,
            DeviceKind::Wifi {
                supports_ap: true,
                ..
            }
        )
    }

    pub fn supports_5ghz(&self) -> bool {
        matches!(
            self.kind,
            DeviceKind::Wifi {
                supports_5ghz: true,
                ..
            }
        )
    }

    pub fn is_ethernet(&self) -> bool {
        self.kind == DeviceKind::Ethernet
    }

    /// Can provide internet to share (anything but the device running the AP).
    pub fn can_be_uplink(&self) -> bool {
        matches!(
            self.kind,
            DeviceKind::Ethernet | DeviceKind::Modem | DeviceKind::Wifi { .. }
        )
    }

    pub fn is_usable(&self) -> bool {
        self.state >= dev_state::DISCONNECTED
    }

    pub fn is_connected(&self) -> bool {
        self.state == dev_state::ACTIVATED
    }
}

/// Lists managed Wi-Fi and Ethernet devices, Wi-Fi first.
pub async fn list(conn: &Connection) -> zbus::Result<Vec<DeviceInfo>> {
    let nm = NetworkManagerProxy::new(conn).await?;
    let mut out = Vec::new();
    for path in nm.get_devices().await? {
        let dev = DeviceProxy::builder(conn)
            .path(path.clone())?
            .build()
            .await?;
        let kind = match dev.device_type().await? {
            NM_DEVICE_TYPE_WIFI => {
                let w = WirelessProxy::builder(conn)
                    .path(path.clone())?
                    .build()
                    .await?;
                let caps = w.wireless_capabilities().await.unwrap_or(0);
                DeviceKind::Wifi {
                    supports_ap: caps & NM_WIFI_DEVICE_CAP_AP != 0,
                    supports_5ghz: caps & NM_WIFI_DEVICE_CAP_FREQ_VALID != 0
                        && caps & NM_WIFI_DEVICE_CAP_FREQ_5GHZ != 0,
                }
            }
            NM_DEVICE_TYPE_ETHERNET => DeviceKind::Ethernet,
            NM_DEVICE_TYPE_MODEM => DeviceKind::Modem,
            _ => continue,
        };
        if !dev.managed().await.unwrap_or(false) {
            continue;
        }
        let active_connection = dev.active_connection().await?;
        let (connection_name, is_default) = active_info(conn, &active_connection).await;
        out.push(DeviceInfo {
            path,
            iface: dev.interface().await?,
            kind,
            state: dev.state().await?,
            active_connection,
            connection_name,
            is_default,
        });
    }
    out.sort_by_key(|d| (d.is_ethernet(), d.iface.clone()));
    Ok(out)
}

async fn active_info(conn: &Connection, ac: &OwnedObjectPath) -> (Option<String>, bool) {
    if ac.as_str() == "/" {
        return (None, false);
    }
    let Ok(b) = ActiveConnectionProxy::builder(conn).path(ac.clone()) else {
        return (None, false);
    };
    let Ok(p) = b.build().await else {
        return (None, false);
    };
    (p.id().await.ok(), p.is_default().await.unwrap_or(false))
}

/// The device the hotspot should run on: the configured one, else the first
/// usable AP-capable Wi-Fi device.
pub fn pick_ap<'a>(devices: &'a [DeviceInfo], wanted: Option<&str>) -> Option<&'a DeviceInfo> {
    match wanted {
        Some(name) => devices
            .iter()
            .find(|d| d.iface == name && d.is_ap_capable()),
        None => devices
            .iter()
            .filter(|d| d.is_ap_capable())
            .min_by_key(|d| (!d.is_usable(), d.is_connected())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(iface: &str, kind: DeviceKind, state: u32) -> DeviceInfo {
        DeviceInfo {
            path: OwnedObjectPath::try_from(format!("/dev/{iface}")).unwrap(),
            iface: iface.into(),
            kind,
            state,
            active_connection: OwnedObjectPath::try_from("/").unwrap(),
            connection_name: None,
            is_default: false,
        }
    }

    #[test]
    fn picks_idle_ap_adapter() {
        let wifi = |ap| DeviceKind::Wifi {
            supports_ap: ap,
            supports_5ghz: false,
        };
        let devs = vec![
            dev("wlan0", wifi(true), dev_state::ACTIVATED),
            dev("wlan1", wifi(true), dev_state::DISCONNECTED),
            dev("wlan2", wifi(false), dev_state::DISCONNECTED),
            dev("eth0", DeviceKind::Ethernet, dev_state::ACTIVATED),
        ];
        assert_eq!(pick_ap(&devs, None).unwrap().iface, "wlan1");
        assert_eq!(pick_ap(&devs, Some("wlan0")).unwrap().iface, "wlan0");
        assert!(pick_ap(&devs, Some("wlan2")).is_none());
        assert!(pick_ap(&devs, Some("eth0")).is_none());
    }
}
