//! Pure data model for LinuxEasyHotspot.
//!
//! This crate deliberately has no D-Bus, async runtime or GUI dependencies so
//! everything here is cheap to unit-test. The `leh-nm` crate turns these types
//! into NetworkManager calls and the `linuxeasyhotspot` crate renders them.

pub mod caps;
pub mod clients;
pub mod config;
pub mod parse;
pub mod password;
pub mod policy;
pub mod qr;
pub mod reasons;
pub mod settings;
pub mod ssid;
pub mod state;
pub mod tc;
pub mod units;
pub mod usb;

pub use caps::{NmCapabilities, NmVersion};
pub use config::{Band, ConfigError, HotspotConfig, Language, Mode};
pub use settings::{NmSettings, ProfileIds, SettingValue};
pub use state::{ErrorKind, Event, HotspotState};

/// The app's name as shown to people (window title, menus, NM profiles).
pub const APP_NAME: &str = "Easy Hotspot";

/// Fixed `connection.id` of the access-point profile managed by the app.
pub const AP_PROFILE_ID: &str = "Easy Hotspot";
/// Fixed `connection.id` of the bridge master profile (Bridge mode).
pub const BRIDGE_PROFILE_ID: &str = "Easy Hotspot Bridge";
/// Fixed `connection.id` of the Ethernet bridge port profile (Bridge mode).
pub const BRIDGE_PORT_PROFILE_ID: &str = "Easy Hotspot Bridge Port";
/// Kernel interface name used for the bridge.
pub const BRIDGE_IFACE: &str = "lehbr0";

/// Profile names used by earlier versions ("LinuxEasyHotspot" up to 0.1.1,
/// "Linux Easy Hotspot" in 0.2). Profiles with these names are still
/// recognized (and renamed the next time the hotspot starts).
pub mod legacy {
    pub const AP_PROFILE_IDS: &[&str] = &["Linux Easy Hotspot", "LinuxEasyHotspot"];
    pub const BRIDGE_PROFILE_IDS: &[&str] =
        &["Linux Easy Hotspot Bridge", "LinuxEasyHotspot Bridge"];
    pub const BRIDGE_PORT_PROFILE_IDS: &[&str] = &[
        "Linux Easy Hotspot Bridge Port",
        "LinuxEasyHotspot Bridge Port",
    ];
}

/// Every `connection.id` a profile of this kind may have, current first.
fn with_legacy(current: &'static str, old: &'static [&'static str]) -> Vec<&'static str> {
    std::iter::once(current)
        .chain(old.iter().copied())
        .collect()
}

/// `connection.id`s of our access-point profile, current first.
pub fn ap_profile_ids() -> Vec<&'static str> {
    with_legacy(AP_PROFILE_ID, legacy::AP_PROFILE_IDS)
}

/// `connection.id`s of our bridge master profile, current first.
pub fn bridge_profile_ids() -> Vec<&'static str> {
    with_legacy(BRIDGE_PROFILE_ID, legacy::BRIDGE_PROFILE_IDS)
}

/// `connection.id`s of our bridge port profile, current first.
pub fn bridge_port_profile_ids() -> Vec<&'static str> {
    with_legacy(BRIDGE_PORT_PROFILE_ID, legacy::BRIDGE_PORT_PROFILE_IDS)
}

/// `connection.id` of our access-point profile, current or legacy.
pub fn is_ap_profile_id(id: &str) -> bool {
    ap_profile_ids().contains(&id)
}

/// `connection.id` of our bridge master profile, current or legacy.
pub fn is_bridge_profile_id(id: &str) -> bool {
    bridge_profile_ids().contains(&id)
}

/// `connection.id` of our bridge port profile, current or legacy.
pub fn is_bridge_port_profile_id(id: &str) -> bool {
    bridge_port_profile_ids().contains(&id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_old_profile_names() {
        for id in ["Easy Hotspot", "Linux Easy Hotspot", "LinuxEasyHotspot"] {
            assert!(is_ap_profile_id(id), "{id}");
        }
        assert!(!is_ap_profile_id("Easy Hotspot Bridge"));
        assert!(is_bridge_profile_id("LinuxEasyHotspot Bridge"));
        assert!(is_bridge_port_profile_id("Linux Easy Hotspot Bridge Port"));
        assert_eq!(ap_profile_ids()[0], AP_PROFILE_ID);
    }
}
