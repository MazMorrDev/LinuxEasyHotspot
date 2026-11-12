//! Spotting USB Wi-Fi adapters that Linux can't use yet.
//!
//! Many cheap USB Wi-Fi sticks first show up as a small USB disk holding the
//! Windows driver ("storage mode"); `usb_modeswitch` flips them into Wi-Fi
//! mode. Others are recognized as Wi-Fi but have no driver loaded. Neither
//! case creates a network interface, so NetworkManager never lists them and
//! the app would otherwise say nothing at all.
//!
//! Pure classification here; `leh-nm` reads the facts from sysfs.

/// One USB device as seen in `/sys/bus/usb/devices`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsbDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    /// `manufacturer` and `product` strings (either may be empty).
    pub manufacturer: String,
    pub product: String,
    pub interfaces: Vec<UsbInterface>,
    /// Some interface has a network device (or a Wi-Fi phy) already.
    pub has_netdev: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsbInterface {
    /// `bInterfaceClass`.
    pub class: u8,
    /// Bound kernel driver, if any.
    pub driver: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbWifiProblem {
    /// Showing up as a USB disk with the Windows driver; needs usb_modeswitch.
    StorageMode,
    /// Looks like a Wi-Fi adapter, but no driver is bound.
    NoDriver,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbWifiIssue {
    pub problem: UsbWifiProblem,
    /// "Realtek DISK" etc.; may be empty.
    pub name: String,
    /// `0bda:1a2b`
    pub id: String,
}

const CLASS_STORAGE: u8 = 0x08;
const CLASS_VENDOR: u8 = 0xff;

/// Vendor:product pairs of Wi-Fi adapters in storage mode (from the
/// usb_modeswitch database).
const STORAGE_MODE_IDS: &[(u16, u16)] = &[
    (0x0bda, 0x1a2b), // Realtek RTL8811CU / RTL8821CU / RTL8822BU and many clones
    (0x0bda, 0xa192), // Realtek RTL8723DU
    (0x0e8d, 0x2870), // MediaTek MT7601U ("MTK WLAN" / Comfast)
    (0x0e8d, 0x7961), // MediaTek MT7921AU
];

/// Chip makers and brands whose vendor-specific USB devices are almost
/// always Wi-Fi adapters.
const WIFI_VENDORS: &[u16] = &[
    0x0bda, // Realtek
    0x0e8d, // MediaTek
    0x148f, // Ralink
    0x0cf3, // Qualcomm Atheros
    0x7392, // Edimax
    0x2357, // TP-Link
    0x0846, // Netgear
    0x2001, // D-Link
    0x0b05, // ASUS
    0x13b1, // Linksys
    0x050d, // Belkin
    0x0df6, // Sitecom
    0x2604, // Tenda
    0x0411, // Buffalo
    0x20f4, // TRENDnet
    0x0586, // ZyXEL
    0x2019, // Planex
    0x056e, // Elecom
    0x2c4e, // Mercusys
    0x3574, // MediaTek (newer)
    0x35bc, // TP-Link (newer)
];

fn mentions_wifi(s: &str) -> bool {
    let s = s.to_ascii_lowercase();
    ["wlan", "wireless", "802.11", "wifi", "wi-fi"]
        .iter()
        .any(|w| s.contains(w))
}

impl UsbDevice {
    pub fn id(&self) -> String {
        format!("{:04x}:{:04x}", self.vendor_id, self.product_id)
    }

    pub fn name(&self) -> String {
        let (m, p) = (self.manufacturer.trim(), self.product.trim());
        match (m.is_empty(), p.is_empty()) {
            (false, false) if p.starts_with(m) => p.to_owned(),
            (false, false) => format!("{m} {p}"),
            (true, false) => p.to_owned(),
            (false, true) => m.to_owned(),
            (true, true) => String::new(),
        }
    }

    /// Why this device, if it is a Wi-Fi adapter, can't be used yet.
    pub fn wifi_issue(&self) -> Option<UsbWifiIssue> {
        if self.has_netdev {
            return None;
        }
        let ids = (self.vendor_id, self.product_id);
        let label = format!("{} {}", self.manufacturer, self.product);
        let wifi_vendor = WIFI_VENDORS.contains(&self.vendor_id);
        let has = |class: u8| self.interfaces.iter().any(|i| i.class == class);

        let storage = STORAGE_MODE_IDS.contains(&ids)
            || (has(CLASS_STORAGE)
                && wifi_vendor
                && (mentions_wifi(&label) || self.product.trim().eq_ignore_ascii_case("disk")));
        let no_driver = !storage
            && self
                .interfaces
                .iter()
                .any(|i| i.class == CLASS_VENDOR && i.driver.is_none())
            && self.interfaces.iter().all(|i| i.driver.is_none())
            && (wifi_vendor || mentions_wifi(&label));

        let problem = if storage {
            UsbWifiProblem::StorageMode
        } else if no_driver {
            UsbWifiProblem::NoDriver
        } else {
            return None;
        };
        Some(UsbWifiIssue {
            problem,
            name: self.name(),
            id: self.id(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(vid: u16, pid: u16, product: &str, ifaces: &[(u8, Option<&str>)]) -> UsbDevice {
        UsbDevice {
            vendor_id: vid,
            product_id: pid,
            manufacturer: "Realtek".into(),
            product: product.into(),
            interfaces: ifaces
                .iter()
                .map(|(c, d)| UsbInterface {
                    class: *c,
                    driver: d.map(Into::into),
                })
                .collect(),
            has_netdev: false,
        }
    }

    #[test]
    fn known_storage_mode_id() {
        let d = dev(0x0bda, 0x1a2b, "DISK", &[(0x08, Some("usb-storage"))]);
        let i = d.wifi_issue().unwrap();
        assert_eq!(i.problem, UsbWifiProblem::StorageMode);
        assert_eq!(i.id, "0bda:1a2b");
        assert_eq!(i.name, "Realtek DISK");
    }

    #[test]
    fn unknown_storage_mode_by_name() {
        let d = dev(0x0bda, 0x1234, "802.11ac WLAN Adapter", &[(0x08, None)]);
        assert_eq!(d.wifi_issue().unwrap().problem, UsbWifiProblem::StorageMode);
        // A Realtek card reader is a disk too, but not a Wi-Fi adapter.
        let reader = dev(0x0bda, 0x0153, "USB2.0-CRW", &[(0x08, Some("usb-storage"))]);
        assert_eq!(reader.wifi_issue(), None);
    }

    #[test]
    fn wifi_without_driver() {
        let d = dev(0x0bda, 0xc811, "802.11ac NIC", &[(0xff, None)]);
        assert_eq!(d.wifi_issue().unwrap().problem, UsbWifiProblem::NoDriver);
    }

    #[test]
    fn working_or_unrelated_devices_are_fine() {
        // Bluetooth radio with its driver.
        let bt = dev(0x0bda, 0xc829, "Bluetooth Radio", &[(0xe0, Some("btusb"))]);
        assert_eq!(bt.wifi_issue(), None);
        // Wi-Fi adapter that already has its interface.
        let mut ok = dev(
            0x0bda,
            0xc811,
            "802.11ac NIC",
            &[(0xff, Some("rtw88_8821cu"))],
        );
        ok.has_netdev = true;
        assert_eq!(ok.wifi_issue(), None);
        // A keyboard from an unrelated vendor.
        let mut kb = dev(0x046d, 0xc31c, "Keyboard K120", &[(0x03, Some("usbhid"))]);
        kb.manufacturer = "Logitech".into();
        assert_eq!(kb.wifi_issue(), None);
        // A vendor-specific device from an unrelated vendor without a driver.
        let mut other = dev(0x1a86, 0x7523, "USB Serial", &[(0xff, None)]);
        other.manufacturer = "QinHeng".into();
        assert_eq!(other.wifi_issue(), None);
    }
}
