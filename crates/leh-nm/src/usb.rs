//! USB Wi-Fi adapters the system can't use yet (storage mode, no driver).
//! Reads `/sys/bus/usb/devices`; the classification is in `leh_model::usb`.

use std::fs;
use std::path::Path;

use leh_model::usb::{UsbDevice, UsbInterface, UsbWifiIssue};

const USB_DEVICES: &str = "/sys/bus/usb/devices";

fn read(dir: &Path, file: &str) -> Option<String> {
    fs::read_to_string(dir.join(file))
        .ok()
        .map(|s| s.trim().to_owned())
}

fn hex(dir: &Path, file: &str) -> Option<u16> {
    u16::from_str_radix(&read(dir, file)?, 16).ok()
}

/// Reads one device (e.g. `1-2`) and its interfaces (`1-2:1.0`, …).
fn device(root: &Path, name: &str) -> Option<UsbDevice> {
    let dir = root.join(name);
    let mut dev = UsbDevice {
        vendor_id: hex(&dir, "idVendor")?,
        product_id: hex(&dir, "idProduct")?,
        manufacturer: read(&dir, "manufacturer").unwrap_or_default(),
        product: read(&dir, "product").unwrap_or_default(),
        ..UsbDevice::default()
    };
    let prefix = format!("{name}:");
    for entry in fs::read_dir(root).ok()?.flatten() {
        let iname = entry.file_name().to_string_lossy().into_owned();
        if !iname.starts_with(&prefix) {
            continue;
        }
        let idir = root.join(&iname);
        let class = u8::from_str_radix(&read(&idir, "bInterfaceClass").unwrap_or_default(), 16)
            .unwrap_or(0);
        let driver = fs::read_link(idir.join("driver"))
            .ok()
            .and_then(|l| l.file_name().map(|n| n.to_string_lossy().into_owned()));
        if idir.join("net").is_dir() || idir.join("ieee80211").is_dir() {
            dev.has_netdev = true;
        }
        dev.interfaces.push(UsbInterface { class, driver });
    }
    Some(dev)
}

/// USB devices that look like Wi-Fi adapters but have no network interface.
pub fn wifi_issues() -> Vec<UsbWifiIssue> {
    wifi_issues_in(Path::new(USB_DEVICES))
}

fn wifi_issues_in(root: &Path) -> Vec<UsbWifiIssue> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        // Devices are "1-2", "3-1.4"; interfaces have a ':'; hubs are "usb1".
        .filter(|n| !n.contains(':') && !n.starts_with("usb"))
        .collect();
    names.sort();
    names
        .iter()
        .filter_map(|n| device(root, n))
        .filter_map(|d| d.wifi_issue())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use leh_model::usb::UsbWifiProblem;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    #[test]
    fn finds_adapter_in_storage_mode() {
        let root = std::env::temp_dir().join(format!("leh-usb-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // A Realtek stick in disk mode…
        write(&root, "1-2/idVendor", "0bda\n");
        write(&root, "1-2/idProduct", "1a2b\n");
        write(&root, "1-2/manufacturer", "Realtek\n");
        write(&root, "1-2/product", "DISK\n");
        write(&root, "1-2:1.0/bInterfaceClass", "08\n");
        // …and a working keyboard.
        write(&root, "1-3/idVendor", "046d\n");
        write(&root, "1-3/idProduct", "c31c\n");
        write(&root, "1-3:1.0/bInterfaceClass", "03\n");
        fs::create_dir_all(root.join("usb1")).unwrap();

        let issues = wifi_issues_in(&root);
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].problem, UsbWifiProblem::StorageMode);
        assert_eq!(issues[0].id, "0bda:1a2b");
    }
}
