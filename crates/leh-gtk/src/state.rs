//! View-model: everything the UI renders, independent of where it comes from.
//! In `--demo` mode it is filled with fake data; later the `leh-nm`
//! controller snapshot is mapped into it.

use crate::i18n::{n_, tr, tr_n, trf};
use std::collections::BTreeMap;

use leh_model::policy::{
    self, Access, CapAction, DefaultPolicy, DevicePolicy, Effective, Limits, Priority,
};
use leh_model::{Band, HotspotConfig, Mode};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Off,
    Starting,
    On,
    Stopping,
}

/// Blocking problems shown as a full status page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    NmMissing,
    NoAdapter,
    BridgeNoEthernet,
    AdapterGone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelperStatus {
    Ready,
    Missing,
    Unauthorized,
    TooOld,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceIcon {
    Phone,
    Laptop,
    Tv,
    Unknown,
}

impl DeviceIcon {
    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Phone => "phone-symbolic",
            Self::Laptop => "computer-symbolic",
            Self::Tv => "video-display-symbolic",
            Self::Unknown => "dialog-question-symbolic",
        }
    }
}

/// Traffic of the whole hotspot while it runs (all devices together).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Traffic {
    /// Bytes sent to devices / received from them this session.
    pub down_bytes: u64,
    pub up_bytes: u64,
    /// Current rates, kbit/s.
    pub down_kbps: f64,
    pub up_kbps: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UplinkKind {
    Ethernet,
    Wifi,
    Modem,
}

/// A connection that can provide the internet to share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UplinkVm {
    pub iface: String,
    pub kind: UplinkKind,
    /// Active profile name, e.g. "Wired connection 1".
    pub connection: Option<String>,
    pub connected: bool,
    /// The device can be connected (cable plugged, SIM present...).
    pub available: bool,
    /// Carries this computer's internet right now (default route).
    pub is_default: bool,
}

impl UplinkVm {
    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            UplinkKind::Ethernet => tr("Ethernet"),
            UplinkKind::Wifi => tr("Wi-Fi"),
            UplinkKind::Modem => tr("Mobile broadband"),
        }
    }

    /// "Ethernet · Wired connection 1" or "Mobile broadband (cdc-wdm1)".
    pub fn label(&self) -> String {
        match &self.connection {
            Some(c) => format!("{} · {c}", self.kind_label()),
            None => format!("{} ({})", self.kind_label(), self.iface),
        }
    }

    pub fn status(&self) -> &'static str {
        if self.is_default {
            tr("in use")
        } else if self.connected {
            "connected"
        } else if self.available {
            tr("not connected, connects when the hotspot starts")
        } else {
            "unavailable"
        }
    }

    /// Row subtitle for the selected source: "Wired connection 1 · in use".
    pub fn detail(&self) -> String {
        match &self.connection {
            Some(c) => format!("{c} · {}", self.status()),
            None => format!("{} · {}", self.iface, self.status()),
        }
    }

    pub fn can_bridge(&self) -> bool {
        self.kind == UplinkKind::Ethernet
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClientVm {
    pub mac: String,
    pub hostname: Option<String>,
    pub ip: Option<String>,
    pub vendor: Option<&'static str>,
    pub icon: DeviceIcon,
    /// Live rates in kbit/s (helper counters).
    pub down_kbps: f64,
    pub up_kbps: f64,
    /// Bytes used this session.
    pub session_bytes: u64,
    /// Demo only: what the device would use if unlimited.
    pub demand_kbps: (f64, f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    pub run: RunState,
    pub problem: Option<Problem>,
    /// Short-lived failure shown in a banner on Home: (title, details).
    pub error: Option<(String, String)>,
    pub cfg: HotspotConfig,
    /// (iface, supports 5 GHz)
    pub adapters: Vec<(String, bool)>,
    /// Ethernet devices available as upstream / bridge port.
    pub ethernet: Vec<String>,
    /// Every connection that could provide internet (not the AP adapter).
    pub uplinks: Vec<UplinkVm>,
    pub nm_version: String,
    pub local_only_supported: bool,
    /// Total download limit for the whole hotspot (NM tbf), Mbps.
    pub total_down_kbps: Option<u32>,
    pub total_up_kbps: Option<u32>,
    /// Interface running the hotspot right now (live mode).
    pub running_iface: Option<String>,
    pub defaults: DefaultPolicy,
    pub helper: HelperStatus,
    pub tray_available: bool,
    pub clients: Vec<ClientVm>,
    /// Whole-hotspot usage this session.
    pub traffic: Traffic,
    pub policies: BTreeMap<String, DevicePolicy>,
    pub open_device: Option<String>,
    /// Driven by the real NetworkManager (not `--demo`).
    pub live: bool,
}

impl UiState {
    pub fn policy(&self, mac: &str) -> DevicePolicy {
        self.policies
            .get(mac)
            .cloned()
            .unwrap_or_else(|| DevicePolicy::new_device(&self.defaults))
    }

    pub fn effective(&self, c: &ClientVm) -> Effective {
        policy::resolve(&self.defaults, &self.policy(&c.mac), c.session_bytes)
    }

    pub fn display_name(&self, c: &ClientVm) -> String {
        self.policies
            .get(&c.mac)
            .and_then(|p| p.alias.clone())
            .or_else(|| c.hostname.clone())
            .unwrap_or_else(|| tr("Unknown device").into())
    }

    /// Name of a known device, connected or not.
    pub fn known_name(&self, mac: &str) -> String {
        if let Some(c) = self.client(mac) {
            return self.display_name(c);
        }
        self.policies
            .get(mac)
            .and_then(|p| p.alias.clone().or_else(|| p.hostname.clone()))
            .unwrap_or_else(|| tr("Unknown device").into())
    }

    /// Devices connected right now (none while the hotspot is off).
    pub fn online_clients(&self) -> &[ClientVm] {
        if self.run == RunState::On {
            &self.clients
        } else {
            &[]
        }
    }

    /// Devices remembered in `devices.toml` that aren't connected now,
    /// most recently seen first.
    pub fn offline_devices(&self) -> Vec<(&String, &DevicePolicy)> {
        let online = self.online_clients();
        let mut v: Vec<_> = self
            .policies
            .iter()
            .filter(|(mac, _)| !online.iter().any(|c| &c.mac == *mac))
            .collect();
        v.sort_by(|a, b| b.1.last_seen.cmp(&a.1.last_seen).then(a.0.cmp(b.0)));
        v
    }

    pub fn client(&self, mac: &str) -> Option<&ClientVm> {
        self.clients.iter().find(|c| c.mac == mac)
    }

    pub fn ap_supports_5ghz(&self) -> bool {
        let wanted = self.cfg.ap_iface.as_deref();
        self.adapters
            .iter()
            .find(|(i, _)| Some(i.as_str()) == wanted)
            .or(self.adapters.first())
            .is_some_and(|(_, five)| *five)
    }

    pub fn helper_ready(&self) -> bool {
        self.helper == HelperStatus::Ready
    }

    /// Shown under "Connected": allowed devices, plus ones blocked only by
    /// their data cap (they are still associated and visible).
    pub fn is_connected(&self, c: &ClientVm) -> bool {
        let e = self.effective(c);
        match e.access {
            Access::Allowed => true,
            Access::Blocked => e.cap_reached,
            Access::Pending => false,
        }
    }

    pub fn connected_count(&self) -> usize {
        self.clients.iter().filter(|c| self.is_connected(c)).count()
    }

    pub fn uplink(&self, iface: &str) -> Option<&UplinkVm> {
        self.uplinks.iter().find(|u| u.iface == iface)
    }

    pub fn default_uplink(&self) -> Option<&UplinkVm> {
        self.uplinks.iter().find(|u| u.is_default)
    }

    /// "Sharing internet from Ethernet · Wired connection 1" etc.
    pub fn status_title(&self) -> String {
        match self.cfg.mode {
            Mode::Router => match self.default_uplink() {
                Some(u) => trf("Sharing internet from {source}", &[("source", &u.label())]),
                None => tr("Hotspot on, but this computer has no internet right now").into(),
            },
            Mode::Bridge => {
                let iface = self
                    .cfg
                    .upstream_iface
                    .as_deref()
                    .unwrap_or(tr("your wired network"));
                trf(
                    "Bridged to {source}",
                    &[(
                        "source",
                        &self.uplink(iface).map_or(iface.to_owned(), UplinkVm::label),
                    )],
                )
            }
            Mode::LocalOnly => tr("Local network only (no internet)").into(),
        }
    }

    pub fn status_caption(&self) -> String {
        let n = self.connected_count();
        let mut parts = vec![tr_n("{n} device", "{n} devices", n)];
        let (band, ch) = match (self.cfg.band, self.cfg.channel) {
            (Band::Ghz5, 0) => (tr("5 GHz"), 36),
            (Band::Ghz5, c) => (tr("5 GHz"), c),
            (_, 0) => (tr("2.4 GHz"), 6),
            (_, c) => (tr("2.4 GHz"), c),
        };
        parts.push(band.into());
        parts.push(trf("ch {n}", &[("n", &ch.to_string())]));
        if let Some(t) = self.total_down_kbps {
            parts.push(format!("≤{}", leh_model::units::format_rate(t)));
        }
        parts.join(" · ")
    }
}

pub fn fmt_rate(kbps: f64) -> String {
    if kbps >= 1000.0 {
        format!("{:.1} Mbps", kbps / 1000.0)
    } else {
        format!("{kbps:.0} kbps")
    }
}

pub fn fmt_limit(kbps: Option<u32>) -> String {
    match kbps {
        None => tr("Unlimited").into(),
        Some(k) => leh_model::units::format_rate(k),
    }
}

pub fn fmt_bytes(b: u64) -> String {
    let mb = b as f64 / 1_000_000.0;
    if mb >= 1000.0 {
        format!("{:.1} GB", mb / 1000.0)
    } else if mb >= 10.0 {
        format!("{mb:.0} MB")
    } else {
        format!("{mb:.1} MB")
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// "Last connected 5 minutes ago" for devices that aren't connected.
pub fn fmt_last_seen(last_seen: Option<u64>) -> String {
    let Some(t) = last_seen else {
        return tr("Not connected").into();
    };
    let ago = now_secs().saturating_sub(t);
    if ago < 60 {
        tr("Last connected just now").into()
    } else if ago < 3600 {
        let n = (ago / 60) as usize;
        tr_n(
            "Last connected {n} minute ago",
            "Last connected {n} minutes ago",
            n,
        )
    } else if ago < 86_400 {
        let n = (ago / 3600) as usize;
        tr_n(
            "Last connected {n} hour ago",
            "Last connected {n} hours ago",
            n,
        )
    } else {
        let n = (ago / 86_400) as usize;
        tr_n(
            "Last connected {n} day ago",
            "Last connected {n} days ago",
            n,
        )
    }
}

/// Short summary of a device's effective policy for list rows.
pub fn policy_summary(state: &UiState, c: &ClientVm) -> String {
    let eff = state.effective(c);
    let pol = state.policy(&c.mac);
    let l = pol.limits(&state.defaults);
    let mut parts = Vec::new();
    if eff.cap_reached {
        match l.cap_action {
            CapAction::Throttle { kbps } => parts.push(trf(
                "{rate} (throttled)",
                &[("rate", &fmt_limit(Some(kbps)))],
            )),
            CapAction::Block => parts.push(tr("Blocked by data cap").into()),
        }
    } else if eff.down_kbps.is_none() && eff.up_kbps.is_none() {
        parts.push(tr("Unlimited").into());
    } else {
        parts.push(trf("{rate} limit", &[("rate", &fmt_limit(eff.down_kbps))]));
    }
    match eff.priority {
        Priority::High => parts.push(tr("High priority").into()),
        Priority::Low => parts.push(tr("Low priority").into()),
        Priority::Normal => {}
    }
    if let Some(cap) = l.cap_mb {
        parts.push(format!("{}/{cap} MB", c.session_bytes / 1_000_000));
    }
    parts.join(" · ")
}

/// Fake data for `--demo`.
pub mod demo {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Scenario {
        Normal,
        StartFails,
        NoAdapter,
        NmMissing,
        BridgeNoEthernet,
        AdapterGone,
        HelperReady,
        HelperMissing,
        HelperUnauthorized,
        HelperTooOld,
        NoDevices,
        NewDevice,
    }

    impl Scenario {
        pub const ALL: [(Scenario, &'static str, &'static str); 12] = [
            (Scenario::Normal, "normal", n_("Normal")),
            (Scenario::StartFails, "start-fails", n_("Next start fails")),
            (
                Scenario::NoAdapter,
                "no-adapter",
                n_("No AP-capable adapter"),
            ),
            (
                Scenario::NmMissing,
                "nm-missing",
                n_("NetworkManager missing"),
            ),
            (
                Scenario::BridgeNoEthernet,
                "bridge-no-eth",
                n_("Bridge without Ethernet"),
            ),
            (
                Scenario::AdapterGone,
                "adapter-gone",
                n_("Adapter unplugged"),
            ),
            (Scenario::HelperReady, "helper-ready", n_("Helper: ready")),
            (
                Scenario::HelperMissing,
                "helper-missing",
                n_("Helper: not installed"),
            ),
            (
                Scenario::HelperUnauthorized,
                "helper-unauth",
                n_("Helper: not authorized"),
            ),
            (Scenario::HelperTooOld, "helper-old", n_("Helper: too old")),
            (Scenario::NoDevices, "no-devices", n_("No devices")),
            (Scenario::NewDevice, "new-device", n_("A new device joins")),
        ];

        pub fn from_id(id: &str) -> Option<Self> {
            Self::ALL
                .iter()
                .find(|(_, i, _)| *i == id)
                .map(|(s, _, _)| *s)
        }
    }

    fn client(
        mac: &str,
        host: Option<&str>,
        ip: &str,
        icon: DeviceIcon,
        demand: (f64, f64),
        mb: u64,
    ) -> ClientVm {
        let c = leh_model::clients::Client {
            mac: mac.into(),
            ip: ip.parse().ok(),
            hostname: host.map(Into::into),
        };
        ClientVm {
            mac: mac.into(),
            hostname: host.map(Into::into),
            ip: Some(ip.into()),
            vendor: c.vendor(),
            icon,
            down_kbps: 0.0,
            up_kbps: 0.0,
            session_bytes: mb * 1_000_000,
            demand_kbps: demand,
        }
    }

    pub fn clients() -> Vec<ClientVm> {
        vec![
            client(
                "3c:28:6d:11:22:33",
                Some("Pixel-8"),
                "10.42.0.23",
                DeviceIcon::Phone,
                (8_000.0, 900.0),
                312,
            ),
            client(
                "ac:bc:32:00:00:41",
                Some("thinkpad"),
                "10.42.0.41",
                DeviceIcon::Laptop,
                (25_000.0, 3_000.0),
                1_204,
            ),
            client(
                "a4:cf:12:9e:00:57",
                None,
                "10.42.0.57",
                DeviceIcon::Unknown,
                (4_000.0, 400.0),
                500,
            ),
            client(
                "da:a1:19:42:00:88",
                Some("Galaxy-S24"),
                "10.42.0.88",
                DeviceIcon::Phone,
                (6_000.0, 600.0),
                0,
            ),
            client(
                "5c:0a:5b:00:00:99",
                Some("Living-room-TV"),
                "10.42.0.99",
                DeviceIcon::Tv,
                (15_000.0, 200.0),
                2_000,
            ),
        ]
    }

    pub fn policies() -> BTreeMap<String, DevicePolicy> {
        let mut m = BTreeMap::new();
        m.insert(
            "3c:28:6d:11:22:33".into(),
            DevicePolicy {
                follow_defaults: false,
                limits: Limits {
                    down_kbps: Some(5_000),
                    up_kbps: Some(1_000),
                    priority: Priority::High,
                    cap_mb: Some(1_000),
                    cap_action: CapAction::Throttle { kbps: 256 },
                },
                ..DevicePolicy::default()
            },
        );
        m.insert(
            "ac:bc:32:00:00:41".into(),
            DevicePolicy {
                follow_defaults: false,
                limits: Limits::default(),
                ..DevicePolicy::default()
            },
        );
        m.insert(
            "a4:cf:12:9e:00:57".into(),
            DevicePolicy {
                follow_defaults: false,
                limits: Limits {
                    cap_mb: Some(500),
                    cap_action: CapAction::Throttle { kbps: 1_000 },
                    ..Limits::default()
                },
                ..DevicePolicy::default()
            },
        );
        m.insert(
            "da:a1:19:42:00:88".into(),
            DevicePolicy {
                access: Access::Pending,
                ..DevicePolicy::default()
            },
        );
        m.insert(
            "5c:0a:5b:00:00:99".into(),
            DevicePolicy {
                alias: Some("Living-room TV".into()),
                access: Access::Blocked,
                ..DevicePolicy::default()
            },
        );
        // Known devices that aren't connected right now.
        m.insert(
            "f0:18:98:00:00:10".into(),
            DevicePolicy {
                hostname: Some("MacBook-de-Ana".into()),
                last_seen: Some(super::now_secs() - 2 * 3600),
                ..DevicePolicy::default()
            },
        );
        m.insert(
            "a8:5e:45:00:00:11".into(),
            DevicePolicy {
                hostname: Some("ESP32-sensor".into()),
                last_seen: Some(super::now_secs() - 3 * 86_400),
                access: Access::Blocked,
                ..DevicePolicy::default()
            },
        );
        m
    }

    pub fn initial() -> UiState {
        let mut cfg = HotspotConfig::first_run("mazmorr-laptop");
        cfg.password = "k7Rp-x4Mz-q9Tw".into();
        cfg.ap_iface = Some("wlp2s0".into());
        UiState {
            run: RunState::Off,
            problem: None,
            error: None,
            cfg,
            adapters: vec![("wlp2s0".into(), true)],
            ethernet: vec!["enp0s31f6".into()],
            uplinks: vec![
                UplinkVm {
                    iface: "enp0s31f6".into(),
                    kind: UplinkKind::Ethernet,
                    connection: Some("Wired connection 1".into()),
                    connected: true,
                    available: true,
                    is_default: true,
                },
                UplinkVm {
                    iface: "wlx00c0ca".into(),
                    kind: UplinkKind::Wifi,
                    connection: Some("Office-WiFi".into()),
                    connected: true,
                    available: true,
                    is_default: false,
                },
                UplinkVm {
                    iface: "cdc-wdm1".into(),
                    kind: UplinkKind::Modem,
                    connection: None,
                    connected: false,
                    available: true,
                    is_default: false,
                },
            ],
            nm_version: "1.46.0".into(),
            local_only_supported: false,
            total_down_kbps: None,
            total_up_kbps: None,
            running_iface: Some("wlp2s0".into()),
            defaults: DefaultPolicy {
                require_approval: true,
                limits: Limits {
                    down_kbps: Some(10_000),
                    up_kbps: Some(2_000),
                    priority: Priority::Normal,
                    cap_mb: Some(500),
                    cap_action: CapAction::Block,
                },
            },
            helper: HelperStatus::Ready,
            tray_available: true,
            clients: clients(),
            traffic: Traffic::default(),
            policies: policies(),
            open_device: None,
            live: false,
        }
    }

    /// One second of fake traffic.
    pub fn tick(state: &mut UiState) {
        use rand::RngExt as _;
        let mut rng = rand::rng();
        let snapshot = state.clone();
        for c in &mut state.clients {
            let eff = snapshot.effective(c);
            if state.run != RunState::On || eff.access != Access::Allowed {
                c.down_kbps = 0.0;
                c.up_kbps = 0.0;
                continue;
            }
            let jitter = rng.random_range(0.55..1.0);
            let cap = |v: f64, lim: Option<u32>| lim.map_or(v, |l| v.min(f64::from(l) * 0.97));
            let mut down = cap(c.demand_kbps.0 * jitter, eff.down_kbps);
            if let Some(t) = snapshot.total_down_kbps {
                down = down.min(f64::from(t) / 2.0);
            }
            c.down_kbps = down;
            c.up_kbps = cap(c.demand_kbps.1 * jitter, eff.up_kbps);
            c.session_bytes += ((c.down_kbps + c.up_kbps) * 1000.0 / 8.0) as u64;
        }
        let t = &mut state.traffic;
        t.down_kbps = state.clients.iter().map(|c| c.down_kbps).sum();
        t.up_kbps = state.clients.iter().map(|c| c.up_kbps).sum();
        t.down_bytes += (t.down_kbps * 1000.0 / 8.0) as u64;
        t.up_bytes += (t.up_kbps * 1000.0 / 8.0) as u64;
    }

    pub fn new_device() -> ClientVm {
        client(
            "e2:5f:01:aa:bb:cc",
            Some("iPad-de-Ana"),
            "10.42.0.120",
            DeviceIcon::Phone,
            (5_000.0, 500.0),
            0,
        )
    }
}
