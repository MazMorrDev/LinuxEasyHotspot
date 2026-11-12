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

pub use caps::{NmCapabilities, NmVersion};
pub use config::{Band, ConfigError, HotspotConfig, Language, Mode};
pub use settings::{NmSettings, ProfileIds, SettingValue};
pub use state::{ErrorKind, Event, HotspotState};

/// The app's name as shown to people (window title, menus, NM profiles).
pub const APP_NAME: &str = "Linux Easy Hotspot";

/// Fixed `connection.id` of the access-point profile managed by the app.
pub const AP_PROFILE_ID: &str = "Linux Easy Hotspot";
/// Fixed `connection.id` of the bridge master profile (Bridge mode).
pub const BRIDGE_PROFILE_ID: &str = "Linux Easy Hotspot Bridge";
/// Fixed `connection.id` of the Ethernet bridge port profile (Bridge mode).
pub const BRIDGE_PORT_PROFILE_ID: &str = "Linux Easy Hotspot Bridge Port";
/// Kernel interface name used for the bridge.
pub const BRIDGE_IFACE: &str = "lehbr0";

/// Profile names used up to 0.1.1. Profiles with these names are still
/// recognized (and renamed the next time the hotspot starts).
pub mod legacy {
    pub const AP_PROFILE_ID: &str = "LinuxEasyHotspot";
    pub const BRIDGE_PROFILE_ID: &str = "LinuxEasyHotspot Bridge";
    pub const BRIDGE_PORT_PROFILE_ID: &str = "LinuxEasyHotspot Bridge Port";
}

/// `connection.id` of our access-point profile, current or legacy.
pub fn is_ap_profile_id(id: &str) -> bool {
    id == AP_PROFILE_ID || id == legacy::AP_PROFILE_ID
}

/// `connection.id` of our bridge master profile, current or legacy.
pub fn is_bridge_profile_id(id: &str) -> bool {
    id == BRIDGE_PROFILE_ID || id == legacy::BRIDGE_PROFILE_ID
}

/// `connection.id` of our bridge port profile, current or legacy.
pub fn is_bridge_port_profile_id(id: &str) -> bool {
    id == BRIDGE_PORT_PROFILE_ID || id == legacy::BRIDGE_PORT_PROFILE_ID
}
