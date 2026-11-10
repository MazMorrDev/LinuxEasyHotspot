//! Hand-written zbus proxies for the parts of the NetworkManager D-Bus API we use.
//! Reference: <https://networkmanager.dev/docs/api/latest/spec.html>

use std::collections::HashMap;

use zbus::proxy;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

/// `a{sa{sv}}` as sent to NM.
pub type SettingsDict<'a> = HashMap<String, HashMap<String, Value<'a>>>;
/// `a{sa{sv}}` as received from NM.
pub type OwnedSettingsDict = HashMap<String, HashMap<String, OwnedValue>>;

#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
pub trait NetworkManager {
    fn get_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    fn activate_connection(
        &self,
        connection: &zbus::zvariant::ObjectPath<'_>,
        device: &zbus::zvariant::ObjectPath<'_>,
        specific_object: &zbus::zvariant::ObjectPath<'_>,
    ) -> zbus::Result<OwnedObjectPath>;

    fn deactivate_connection(
        &self,
        active_connection: &zbus::zvariant::ObjectPath<'_>,
    ) -> zbus::Result<()>;

    #[zbus(property)]
    fn version(&self) -> zbus::Result<String>;

    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
pub trait Settings {
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// NM ≥ 1.20. `flags`: 0x1 = TO_DISK.
    fn add_connection2(
        &self,
        settings: SettingsDict<'_>,
        flags: u32,
        args: HashMap<String, Value<'_>>,
    ) -> zbus::Result<(OwnedObjectPath, HashMap<String, OwnedValue>)>;
}

#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait SettingsConnection {
    fn get_settings(&self) -> zbus::Result<OwnedSettingsDict>;

    /// NM ≥ 1.12. Replaces all settings. `flags`: 0x1 = TO_DISK.
    fn update2(
        &self,
        settings: SettingsDict<'_>,
        flags: u32,
        args: HashMap<String, Value<'_>>,
    ) -> zbus::Result<HashMap<String, OwnedValue>>;

    fn delete(&self) -> zbus::Result<()>;
}

pub mod device {
    use super::*;

    #[proxy(
        interface = "org.freedesktop.NetworkManager.Device",
        default_service = "org.freedesktop.NetworkManager"
    )]
    pub trait Device {
        #[zbus(property)]
        fn interface(&self) -> zbus::Result<String>;

        #[zbus(property)]
        fn device_type(&self) -> zbus::Result<u32>;

        #[zbus(property)]
        fn state(&self) -> zbus::Result<u32>;

        #[zbus(property)]
        fn managed(&self) -> zbus::Result<bool>;

        #[zbus(property)]
        fn active_connection(&self) -> zbus::Result<OwnedObjectPath>;

        #[zbus(property)]
        fn ip4_config(&self) -> zbus::Result<OwnedObjectPath>;

        #[zbus(property, name = "HwAddress")]
        fn hw_address(&self) -> zbus::Result<String>;

        #[zbus(signal, name = "StateChanged")]
        fn state_signal(&self, new_state: u32, old_state: u32, reason: u32) -> zbus::Result<()>;
    }
}
pub use device::DeviceProxy;

#[proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Wireless {
    #[zbus(property)]
    fn wireless_capabilities(&self) -> zbus::Result<u32>;
}

pub mod active {
    use super::*;

    #[proxy(
        interface = "org.freedesktop.NetworkManager.Connection.Active",
        default_service = "org.freedesktop.NetworkManager"
    )]
    pub trait ActiveConnection {
        #[zbus(property)]
        fn connection(&self) -> zbus::Result<OwnedObjectPath>;

        #[zbus(property)]
        fn uuid(&self) -> zbus::Result<String>;

        #[zbus(property)]
        fn id(&self) -> zbus::Result<String>;

        #[zbus(property)]
        fn state(&self) -> zbus::Result<u32>;

        #[zbus(property)]
        fn devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

        /// Whether this connection owns the IPv4 default route.
        #[zbus(property, name = "Default")]
        fn is_default(&self) -> zbus::Result<bool>;

        #[zbus(signal, name = "StateChanged")]
        fn state_signal(&self, state: u32, reason: u32) -> zbus::Result<()>;
    }
}
pub use active::ActiveConnectionProxy;

#[proxy(
    interface = "org.freedesktop.NetworkManager.IP4Config",
    default_service = "org.freedesktop.NetworkManager"
)]
pub trait Ip4Config {
    #[zbus(property)]
    fn address_data(&self) -> zbus::Result<Vec<HashMap<String, OwnedValue>>>;
}
