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

/// How the adapter is attached to the computer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Bus {
    Usb,
    Pci,
    /// SDIO, platform devices and anything else.
    #[default]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub path: OwnedObjectPath,
    pub iface: String,
    pub kind: DeviceKind,
    pub state: u32,
    /// NetworkManager controls the device. Unmanaged Wi-Fi adapters are
    /// listed (so the app can explain why it can't use them); unmanaged
    /// wired and mobile devices are skipped.
    pub managed: bool,
    /// Current MAC address (`aa:bb:cc:dd:ee:ff`), empty if unknown.
    pub hw_address: String,
    pub bus: Bus,
    /// Kernel driver, e.g. `rtw88_8821cu`.
    pub driver: Option<String>,
    /// Product name of a USB adapter, e.g. "802.11ac NIC".
    pub product: Option<String>,
    /// `/` when nothing is active.
    pub active_connection: OwnedObjectPath,
    /// Name (`connection.id`) of the active profile, e.g. "Wired connection 1".
    pub connection_name: Option<String>,
    /// The active connection owns the IPv4 default route (carries internet).
    pub is_default: bool,
}

impl DeviceInfo {
    pub fn is_wifi(&self) -> bool {
        matches!(self.kind, DeviceKind::Wifi { .. })
    }

    pub fn is_ap_capable(&self) -> bool {
        self.managed
            && matches!(
                self.kind,
                DeviceKind::Wifi {
                    supports_ap: true,
                    ..
                }
            )
    }

    pub fn is_usb(&self) -> bool {
        self.bus == Bus::Usb
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
        self.managed
            && matches!(
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

/// Where the kernel says `iface` is attached, its driver and (USB) product name.
fn sysfs_info(iface: &str) -> (Bus, Option<String>, Option<String>) {
    let dev = std::path::Path::new("/sys/class/net")
        .join(iface)
        .join("device");
    let name_of = |p: std::path::PathBuf| {
        std::fs::read_link(p)
            .ok()
            .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()))
    };
    let bus = match name_of(dev.join("subsystem")).as_deref() {
        Some("usb") => Bus::Usb,
        Some("pci") => Bus::Pci,
        _ => Bus::Other,
    };
    let driver = name_of(dev.join("driver"));
    // `device` is the USB interface; the product string is on its parent.
    let product = (bus == Bus::Usb)
        .then(|| std::fs::canonicalize(&dev).ok())
        .flatten()
        .and_then(|p| std::fs::read_to_string(p.parent()?.join("product")).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    (bus, driver, product)
}

/// Lists Wi-Fi, Ethernet and mobile devices, Wi-Fi first. Unmanaged
/// Ethernet and mobile devices are skipped, unmanaged Wi-Fi is kept.
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
        let managed = dev.managed().await.unwrap_or(false);
        if !managed && !matches!(kind, DeviceKind::Wifi { .. }) {
            continue;
        }
        let active_connection = dev.active_connection().await?;
        let (connection_name, is_default) = active_info(conn, &active_connection).await;
        let iface = dev.interface().await?;
        let (bus, driver, product) = sysfs_info(&iface);
        out.push(DeviceInfo {
            path,
            kind,
            state: dev.state().await?,
            managed,
            hw_address: dev
                .hw_address()
                .await
                .unwrap_or_default()
                .to_ascii_lowercase(),
            bus,
            driver,
            product,
            active_connection,
            connection_name,
            is_default,
            iface,
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

/// The device the hotspot should run on: the configured one, else the best
/// AP-capable Wi-Fi device: usable, not busy as this computer's own Wi-Fi
/// connection, and preferably a USB adapter (usually the one with the bigger
/// antenna, plugged in precisely to run the hotspot).
pub fn pick_ap<'a>(devices: &'a [DeviceInfo], wanted: Option<&str>) -> Option<&'a DeviceInfo> {
    match wanted {
        Some(name) => devices
            .iter()
            .find(|d| d.iface == name && d.is_ap_capable()),
        None => devices
            .iter()
            .filter(|d| d.is_ap_capable())
            .min_by_key(|d| (!d.is_usable(), d.is_connected(), !d.is_usb())),
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
            managed: true,
            hw_address: String::new(),
            bus: Bus::Pci,
            driver: None,
            product: None,
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

    #[test]
    fn prefers_free_usb_adapter() {
        let wifi = DeviceKind::Wifi {
            supports_ap: true,
            supports_5ghz: true,
        };
        let mut usb = dev("wlx00c0ca", wifi.clone(), dev_state::DISCONNECTED);
        usb.bus = Bus::Usb;
        let internal = dev("wlp3s0", wifi.clone(), dev_state::DISCONNECTED);
        let devs = vec![internal.clone(), usb.clone()];
        assert_eq!(pick_ap(&devs, None).unwrap().iface, "wlx00c0ca");
        // The internal card still wins if the USB one is unavailable...
        let mut gone = usb.clone();
        gone.state = dev_state::UNAVAILABLE;
        assert_eq!(
            pick_ap(&[internal.clone(), gone], None).unwrap().iface,
            "wlp3s0"
        );
        // ...and unmanaged adapters are never picked.
        let mut unmanaged = usb;
        unmanaged.managed = false;
        assert_eq!(
            pick_ap(&[internal, unmanaged], None).unwrap().iface,
            "wlp3s0"
        );
    }
}
