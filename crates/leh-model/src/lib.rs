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

pub use caps::{NmCapabilities, NmVersion};
pub use config::{Band, ConfigError, HotspotConfig, Mode};
pub use settings::{NmSettings, ProfileIds, SettingValue};
pub use state::{ErrorKind, Event, HotspotState};

/// Fixed `connection.id` of the access-point profile managed by the app.
pub const AP_PROFILE_ID: &str = "LinuxEasyHotspot";
/// Fixed `connection.id` of the bridge master profile (Bridge mode).
pub const BRIDGE_PROFILE_ID: &str = "LinuxEasyHotspot Bridge";
/// Fixed `connection.id` of the Ethernet bridge port profile (Bridge mode).
pub const BRIDGE_PORT_PROFILE_ID: &str = "LinuxEasyHotspot Bridge Port";
/// Kernel interface name used for the bridge.
pub const BRIDGE_IFACE: &str = "lehbr0";
