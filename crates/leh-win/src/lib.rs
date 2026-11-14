//! Windows backend for Easy Hotspot: the built-in Mobile Hotspot
//! (`Windows.Networking.NetworkOperators.NetworkOperatorTetheringManager`).
//!
//! The hotspot is Windows' own: it keeps running if the app closes and can
//! also be stopped from the quick settings. No administrator rights needed.
//!
//! The UI only uses [`Controller`] and the types below. On other systems the
//! controller reports that it isn't supported, so the crate builds everywhere.

use leh_model::clients::Client;
use leh_model::{Band, HotspotConfig, HotspotState};

#[cfg(windows)]
mod imp;
mod logic;

/// A Wi-Fi adapter that could run the hotspot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiAdapter {
    /// `NetworkAdapter.NetworkAdapterId` (a GUID, lower case, no braces).
    /// Stored in [`HotspotConfig::ap_iface`].
    pub id: String,
    /// Device name, e.g. "Intel(R) Wi-Fi 6 AX201 160MHz".
    pub name: String,
    /// Plugged in over USB.
    pub usb: bool,
    /// Can run the hotspot on 5 GHz; `None` when Windows can't tell
    /// (band selection needs Windows 11).
    pub five_ghz: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UplinkKind {
    Ethernet,
    Wifi,
    Cellular,
    Other,
}

/// A connection whose internet the hotspot can share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uplink {
    /// `NetworkAdapterId` of the connection's adapter (GUID, lower case, no
    /// braces). Stored in [`HotspotConfig::upstream_iface`].
    pub id: String,
    /// Connection (profile) name: "Ethernet 2", the Wi-Fi network's name…
    pub name: String,
    pub kind: UplinkKind,
    /// Has internet access right now.
    pub internet: bool,
    /// Carries this computer's internet (Windows' internet profile).
    pub is_default: bool,
}

/// Details about the running hotspot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Running {
    pub ssid: String,
    /// Adapter running it, when known.
    pub adapter_id: Option<String>,
    /// Connection being shared, when known.
    pub uplink_id: Option<String>,
    /// `Band::Auto` when Windows doesn't say.
    pub band: Band,
    /// `MaxClientCount`.
    pub max_clients: u32,
}

/// Everything the UI needs, published whenever something changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub state: HotspotState,
    /// Why Windows won't share at all (group policy, edition, no Wi-Fi…),
    /// in plain English. `None` when tethering is possible.
    pub unavailable: Option<String>,
    pub adapters: Vec<WifiAdapter>,
    pub uplinks: Vec<Uplink>,
    pub running: Option<Running>,
    /// Windows can be told which band to use (Windows 11).
    pub band_selectable: bool,
    /// Devices connected right now (empty while off).
    pub clients: Vec<Client>,
    /// `(rx, tx)` byte counters of the hotspot's virtual adapter while it
    /// runs: rx is what devices sent, tx what they received.
    pub traffic: Option<(u64, u64)>,
}

/// Owns all interaction with Windows on a worker thread. Commands are handled
/// one at a time; every change is passed to the `on_update` callback.
pub struct Controller {
    #[cfg(windows)]
    inner: imp::Handle,
}

impl Controller {
    /// Starts the worker. `on_update` runs on the worker thread with the first
    /// snapshot and then on every change.
    pub fn spawn(on_update: impl Fn(Snapshot) + Send + 'static) -> Self {
        #[cfg(windows)]
        {
            Self {
                inner: imp::Handle::spawn(Box::new(on_update)),
            }
        }
        #[cfg(not(windows))]
        {
            on_update(Snapshot {
                unavailable: Some("This backend only works on Windows.".into()),
                ..Snapshot::default()
            });
            Self {}
        }
    }

    /// Configures (name, password, band) and starts the hotspot, sharing
    /// `cfg.upstream_iface` (or this computer's internet) over `cfg.ap_iface`
    /// (or Windows' choice).
    pub fn start(&self, cfg: HotspotConfig) {
        #[cfg(windows)]
        self.inner.start(cfg);
        #[cfg(not(windows))]
        drop(cfg);
    }

    pub fn stop(&self) {
        #[cfg(windows)]
        self.inner.stop();
    }

    /// Re-reads adapters, connections and state now.
    pub fn refresh(&self) {
        #[cfg(windows)]
        self.inner.refresh();
    }
}
