//! The worker that drives Windows' Mobile Hotspot through WinRT.
//!
//! Everything runs on one thread that never calls `CoInitialize`: on the
//! first WinRT activation windows-rs gets `CO_E_NOTINITIALIZED` and calls
//! `CoIncrementMTAUsage`, so the thread lives in the implicit MTA, where
//! waiting for async operations is allowed. WinRT objects never leave it.

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use leh_model::clients::{self, Client};
use leh_model::reasons::{Explanation, Suggestion};
use leh_model::{Band, ErrorKind, HotspotConfig, HotspotState};
use windows::Devices::Enumeration::DeviceInformation;
use windows::Devices::WiFi::WiFiAdapter;
use windows::Foundation::Metadata::ApiInformation;
use windows::Networking::Connectivity::{
    ConnectionProfile, NetworkAdapter, NetworkConnectivityLevel, NetworkInformation,
};
use windows::Networking::HostNameType;
use windows::Networking::NetworkOperators::{
    NetworkOperatorTetheringAccessPointConfiguration, NetworkOperatorTetheringManager,
    NetworkOperatorTetheringOperationResult, TetheringWiFiBand,
};
use windows::core::{GUID, HSTRING, RuntimeType};
use windows_collections::IVectorView;
use windows_future::{
    AsyncActionCompletedHandler, AsyncOperationCompletedHandler, IAsyncAction, IAsyncOperation,
};

use crate::logic::{self, Doing, Op};
use crate::{Running, Snapshot, Uplink, WifiAdapter};

/// Poll interval while the hotspot is on or changing.
const FAST: Duration = Duration::from_secs(2);
/// Poll interval while it is off, and for adapters and connections.
const SLOW: Duration = Duration::from_secs(5);

const QUERY_TIMEOUT: Duration = Duration::from_secs(15);
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(30);
const START_TIMEOUT: Duration = Duration::from_secs(60);
const STOP_TIMEOUT: Duration = Duration::from_secs(30);
const SETTING_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a restart waits for Windows to finish turning the hotspot off.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long dropping the [`Handle`] waits for the worker to finish.
const EXIT_WAIT: Duration = Duration::from_secs(2);

const ACCESS_POINT_CONFIGURATION: &str =
    "Windows.Networking.NetworkOperators.NetworkOperatorTetheringAccessPointConfiguration";
const ICS_PARAMETERS: &str = r"SYSTEM\CurrentControlSet\Services\SharedAccess\Parameters";

enum Cmd {
    Start(HotspotConfig),
    Stop,
    Refresh,
}

pub(crate) struct Handle {
    /// `None` only while dropping.
    commands: Option<Sender<Cmd>>,
    worker: Option<JoinHandle<()>>,
}

impl Handle {
    pub(crate) fn spawn(on_update: Box<dyn Fn(Snapshot) + Send>) -> Handle {
        let (commands, rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("hotspot-windows".into())
            .spawn(move || Worker::new(on_update).run(&rx))
            .expect("couldn't start the hotspot worker thread");
        Handle {
            commands: Some(commands),
            worker: Some(worker),
        }
    }

    pub(crate) fn start(&self, cfg: HotspotConfig) {
        self.send(Cmd::Start(cfg));
    }

    pub(crate) fn stop(&self) {
        self.send(Cmd::Stop);
    }

    pub(crate) fn refresh(&self) {
        self.send(Cmd::Refresh);
    }

    fn send(&self, cmd: Cmd) {
        // Only fails if the worker is gone (it panicked); nothing to do then.
        if let Some(commands) = &self.commands {
            let _ = commands.send(cmd);
        }
    }
}

impl Drop for Handle {
    /// Closing the channel stops the worker, which then puts Windows'
    /// power-saving setting back. Give it a moment to do so before the
    /// process exits, without hanging the UI if Windows is slow.
    fn drop(&mut self) {
        drop(self.commands.take());
        let Some(worker) = self.worker.take() else {
            return;
        };
        if worker.thread().id() == thread::current().id() {
            // Dropped from `on_update`: the worker ends after this call.
            return;
        }
        let deadline = Instant::now() + EXIT_WAIT;
        while !worker.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        if worker.is_finished() {
            let _ = worker.join();
        }
    }
}

// `Controller` must stay shareable between threads, as the types promise.
const _: () = {
    const fn shareable<T: Send + Sync>() {}
    shareable::<Handle>();
};

/// A Wi-Fi adapter and the WinRT object needed to target it.
struct Wifi {
    info: WifiAdapter,
    adapter: NetworkAdapter,
}

/// A connected connection profile.
struct Profile {
    profile: ConnectionProfile,
    uplink: Uplink,
}

/// A hotspot started by the app.
struct Session {
    manager: NetworkOperatorTetheringManager,
    cfg: HotspotConfig,
    adapter_id: Option<String>,
    uplink_id: String,
}

struct Worker {
    on_update: Box<dyn Fn(Snapshot) + Send>,
    snap: Snapshot,
    published: Option<Snapshot>,
    /// Cleared once the hotspot is seen off.
    session: Option<Session>,
    /// The manager that last reported the hotspot on or changing.
    live: Option<NetworkOperatorTetheringManager>,
    /// Since when the app's own successful start is being confirmed.
    confirm_since: Option<Instant>,
    /// The app turned off Windows' "turn off the hotspot when no devices are
    /// connected" and must turn it back on.
    restore_timeout: bool,
    wifi: Vec<Wifi>,
    /// Wi-Fi devices Windows lists (also those that couldn't be opened);
    /// `None` when the query failed.
    wifi_devices: Option<usize>,
    /// `IsBandSupported(5 GHz)` per adapter id.
    five_ghz: HashMap<String, bool>,
    /// ICS's address on the hotspot side.
    gateway: Ipv4Addr,
    /// `NetworkAdapterId` of the hotspot's virtual adapter, while it's up.
    hotspot_adapter: Option<String>,
    networks: Option<sysinfo::Networks>,
    last_fast: Instant,
    last_slow: Instant,
}

impl Worker {
    fn new(on_update: Box<dyn Fn(Snapshot) + Send>) -> Self {
        // `Band`/`IsBandSupported` (INetworkOperatorTetheringAccessPointConfiguration2)
        // only exist on Windows 11.
        let band_selectable = ApiInformation::IsPropertyPresent(
            &HSTRING::from(ACCESS_POINT_CONFIGURATION),
            &HSTRING::from("Band"),
        )
        .unwrap_or(false);
        let now = Instant::now();
        Self {
            on_update,
            snap: Snapshot {
                band_selectable,
                ..Snapshot::default()
            },
            published: None,
            session: None,
            live: None,
            confirm_since: None,
            restore_timeout: false,
            wifi: Vec::new(),
            wifi_devices: None,
            five_ghz: HashMap::new(),
            gateway: logic::ICS_GATEWAY,
            hotspot_adapter: None,
            networks: None,
            last_fast: now,
            last_slow: now,
        }
    }

    fn run(mut self, commands: &Receiver<Cmd>) {
        self.poll(true);
        self.publish();
        loop {
            let state = &self.snap.state;
            let every = if state.is_running() || state.is_busy() {
                FAST
            } else {
                SLOW
            };
            let next_slow = self.last_slow + SLOW;
            let next = (self.last_fast + every).min(next_slow);
            match commands.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(Cmd::Start(cfg)) => self.start(cfg),
                Ok(Cmd::Stop) => self.stop(),
                Ok(Cmd::Refresh) => {
                    self.five_ghz.clear();
                    self.poll(true);
                }
                Err(RecvTimeoutError::Timeout) => self.poll(Instant::now() >= next_slow),
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.publish();
        }
        // The hotspot is Windows' own and may keep running without the app,
        // but the power-saving setting must not stay changed.
        self.restore_timeout();
    }

    fn publish(&mut self) {
        if self.published.as_ref() != Some(&self.snap) {
            self.published = Some(self.snap.clone());
            (self.on_update)(self.snap.clone());
        }
    }

    fn poll(&mut self, slow: bool) {
        if slow {
            self.refresh_slow();
        }
        self.poll_state();
    }

    /// Adapters, connections and whether sharing is possible at all.
    fn refresh_slow(&mut self) -> Vec<Profile> {
        self.last_slow = Instant::now();
        self.gateway = read_gateway();
        self.hotspot_adapter = adapter_with_address(self.gateway);
        let profiles = connected_profiles(self.hotspot_adapter.as_deref());
        self.refresh_adapters(&profiles);
        self.snap.uplinks = profiles.iter().map(|p| p.uplink.clone()).collect();
        self.snap.unavailable = self.unavailable(&profiles);
        profiles
    }

    fn refresh_adapters(&mut self, profiles: &[Profile]) {
        match find_wifi() {
            Ok((devices, wifi)) => {
                self.wifi_devices = Some(devices);
                self.wifi = wifi;
            }
            // Keep the last list rather than claiming there's no adapter.
            Err(_) => self.wifi_devices = None,
        }
        if let Some(own) = &self.hotspot_adapter {
            self.wifi.retain(|w| &w.info.id != own);
        }
        self.five_ghz
            .retain(|id, _| self.wifi.iter().any(|w| &w.info.id == id));
        // `IsBandSupported` needs a manager, and a manager needs a profile.
        let probe = profiles
            .iter()
            .find(|p| p.uplink.is_default)
            .or(profiles.first());
        for w in &mut self.wifi {
            if self.snap.band_selectable
                && !self.five_ghz.contains_key(&w.info.id)
                && let Some(five) = probe.and_then(|p| supports_five_ghz(&p.profile, &w.adapter))
            {
                self.five_ghz.insert(w.info.id.clone(), five);
            }
            w.info.five_ghz = self.five_ghz.get(&w.info.id).copied();
        }
        self.snap.adapters = self.wifi.iter().map(|w| w.info.clone()).collect();
    }

    fn unavailable(&self, profiles: &[Profile]) -> Option<String> {
        if self.wifi_devices == Some(0) {
            return Some(logic::NO_WIFI.to_owned());
        }
        let profile = profiles.iter().find(|p| p.uplink.is_default)?;
        let message =
            match NetworkOperatorTetheringManager::GetTetheringCapabilityFromConnectionProfile(
                &profile.profile,
            ) {
                Ok(capability) => logic::capability_message(capability.0),
                Err(e) => logic::unavailable_from_hresult(e.code().0),
            };
        message.map(str::to_owned)
    }

    /// State, and while on: details, clients and traffic.
    fn poll_state(&mut self) {
        self.last_fast = Instant::now();
        let (op, manager) = self.read_op();
        let next = logic::follow(
            &self.snap.state,
            op,
            self.confirm_since.map(|t| t.elapsed()),
        );
        if next != HotspotState::Activating {
            self.confirm_since = None;
        }
        if op == Op::Off && next != HotspotState::Activating {
            // Off, whoever turned it off (the app, quick settings, Windows).
            self.session = None;
            self.restore_timeout();
        }
        self.live = manager;
        match (&self.live, op) {
            (Some(manager), Op::On) => {
                let manager = manager.clone();
                self.read_running(&manager);
            }
            _ => {
                self.snap.running = None;
                self.snap.clients.clear();
                self.snap.traffic = None;
            }
        }
        self.snap.state = next;
    }

    /// What Windows says about the hotspot, plus the manager that said it
    /// when it's on or changing.
    ///
    /// The hotspot is system-wide but the API wants a connection profile:
    /// the managers already known are asked first, then one per connected
    /// profile.
    fn read_op(&self) -> (Op, Option<NetworkOperatorTetheringManager>) {
        let mut changing = None;
        let mut off = false;
        let mut asked = false;
        let mut ask = |manager: NetworkOperatorTetheringManager| {
            asked = true;
            match operational_state(&manager) {
                Op::On => return Some(manager),
                Op::InTransition if changing.is_none() => changing = Some(manager),
                Op::Off => off = true,
                Op::InTransition | Op::Unknown => {}
            }
            None
        };
        let session = self.session.as_ref().map(|s| s.manager.clone());
        let live = self.live.clone().filter(|m| Some(m) != session.as_ref());
        let known = session.into_iter().chain(live);
        for manager in known {
            if let Some(on) = ask(manager) {
                return (Op::On, Some(on));
            }
        }
        for profile in connected_profiles(self.hotspot_adapter.as_deref()) {
            let manager =
                NetworkOperatorTetheringManager::CreateFromConnectionProfile(&profile.profile);
            if let Some(on) = manager.ok().and_then(&mut ask) {
                return (Op::On, Some(on));
            }
        }
        match changing {
            Some(manager) => (Op::InTransition, Some(manager)),
            // Nothing to ask means no connection to share: it can't be on.
            None if off || !asked => (Op::Off, None),
            None => (Op::Unknown, None),
        }
    }

    fn read_running(&mut self, manager: &NetworkOperatorTetheringManager) {
        let config = manager.GetCurrentAccessPointConfiguration().ok();
        let ssid = config
            .as_ref()
            .and_then(|c| c.Ssid().ok())
            .map(|s| s.to_string_lossy())
            .unwrap_or_default();
        let band = config
            .filter(|_| self.snap.band_selectable)
            .and_then(|c| c.Band().ok())
            .map_or(Band::Auto, |b| logic::band_from_winrt(b.0));
        let session = self.session.as_ref();
        // With a single Wi-Fi adapter there's no doubt which one runs it.
        let adapter_id = session.and_then(|s| s.adapter_id.clone()).or_else(|| {
            match self.snap.adapters.as_slice() {
                [only] => Some(only.id.clone()),
                _ => None,
            }
        });
        self.snap.running = Some(Running {
            ssid,
            adapter_id,
            uplink_id: session.map(|s| s.uplink_id.clone()),
            band,
            max_clients: manager.MaxClientCount().unwrap_or(0),
        });
        self.snap.clients = read_clients(manager);
        self.snap.traffic = self.read_traffic();
    }

    /// Byte counters of the interface holding ICS's hotspot-side address.
    fn read_traffic(&mut self) -> Option<(u64, u64)> {
        let gateway = IpAddr::V4(self.gateway);
        let networks = self.networks.get_or_insert_with(sysinfo::Networks::new);
        networks.refresh(true);
        networks
            .list()
            .values()
            .find(|data| data.ip_networks().iter().any(|net| net.addr == gateway))
            .map(|data| (data.total_received(), data.total_transmitted()))
    }

    fn start(&mut self, cfg: HotspotConfig) {
        // Already on (or being confirmed) with these settings.
        let unchanged = matches!(
            self.snap.state,
            HotspotState::Active | HotspotState::Activating
        ) && self
            .session
            .as_ref()
            .is_some_and(|s| logic::same_settings(&s.cfg, &cfg));
        if unchanged {
            self.poll_state();
            return;
        }
        self.snap.state = HotspotState::Activating;
        self.confirm_since = None;
        self.publish();
        match self.try_start(cfg) {
            Ok(()) => {
                self.confirm_since = Some(Instant::now());
                self.poll_state();
            }
            Err(kind) => {
                self.restore_timeout();
                self.snap.state = HotspotState::Error(kind);
                self.snap.running = None;
                self.snap.clients.clear();
                self.snap.traffic = None;
            }
        }
    }

    fn try_start(&mut self, cfg: HotspotConfig) -> Result<(), ErrorKind> {
        let profiles = self.refresh_slow();
        if self.wifi_devices == Some(0) {
            return Err(ErrorKind::NoApAdapter);
        }
        let candidates: Vec<(&str, bool)> = profiles
            .iter()
            .map(|p| (p.uplink.id.as_str(), p.uplink.is_default))
            .collect();
        let profile = logic::pick_uplink(&candidates, cfg.upstream_iface.as_deref())
            .map(|i| &profiles[i])
            .ok_or_else(logic::no_internet)?;

        // Already on (other settings, or started elsewhere): restart it so
        // the requested settings apply.
        if let (Op::On | Op::InTransition, Some(running)) = self.read_op() {
            stop_tethering(&running)?;
            wait_until_settled(&running);
        }
        self.session = None;
        self.live = None;

        let ids: Vec<&str> = self.wifi.iter().map(|w| w.info.id.as_str()).collect();
        let targeted = logic::pick_adapter(&ids, cfg.ap_iface.as_deref()).and_then(|i| {
            let wifi = &self.wifi[i];
            // Not on older Windows 10 builds: then Windows chooses.
            NetworkOperatorTetheringManager::CreateFromConnectionProfileWithTargetAdapter(
                &profile.profile,
                &wifi.adapter,
            )
            .ok()
            .map(|manager| (manager, Some(wifi.info.id.clone())))
        });
        let (manager, adapter_id) = match targeted {
            Some(found) => found,
            None => (
                NetworkOperatorTetheringManager::CreateFromConnectionProfile(&profile.profile)
                    .map_err(creation_error)?,
                None,
            ),
        };

        self.configure(&manager, &cfg)?;
        self.disable_timeout();
        let result = wait_op(manager.StartTetheringAsync(), START_TIMEOUT)
            .map_err(|f| f.kind(Doing::Start))?;
        check_result(&result, false)?;
        self.session = Some(Session {
            manager,
            cfg,
            adapter_id,
            uplink_id: profile.uplink.id.clone(),
        });
        Ok(())
    }

    /// Name, password and (Windows 11) band.
    fn configure(
        &self,
        manager: &NetworkOperatorTetheringManager,
        cfg: &HotspotConfig,
    ) -> Result<(), ErrorKind> {
        let config = manager.GetCurrentAccessPointConfiguration().map_err(call)?;
        config
            .SetSsid(&HSTRING::from(cfg.ssid.as_str()))
            .map_err(rejected)?;
        config
            .SetPassphrase(&HSTRING::from(cfg.password.as_str()))
            .map_err(rejected)?;
        if self.snap.band_selectable {
            set_band(&config, cfg.band)?;
        }
        wait_action(
            manager.ConfigureAccessPointAsync(&config),
            CONFIGURE_TIMEOUT,
        )
        .map_err(|f| match f {
            Failure::Call(e) => rejected(e),
            Failure::TimedOut => logic::timed_out(Doing::Configure),
        })
    }

    fn stop(&mut self) {
        let manager = self
            .session
            .as_ref()
            .map(|s| s.manager.clone())
            .or_else(|| self.live.clone())
            .or_else(|| self.read_op().1);
        let Some(manager) = manager else {
            // Nothing is running.
            self.poll_state();
            return;
        };
        self.snap.state = HotspotState::Deactivating;
        self.confirm_since = None;
        self.publish();
        match stop_tethering(&manager) {
            Ok(()) => {
                self.session = None;
                self.live = None;
                self.restore_timeout();
                self.poll_state();
            }
            Err(kind) => self.snap.state = HotspotState::Error(kind),
        }
    }

    /// Windows turns the hotspot off after a few minutes without devices
    /// ("power saving"). Keep it on while the app runs it; the setting is
    /// put back when the hotspot stops.
    fn disable_timeout(&mut self) {
        // Windows 11 only (INetworkOperatorTetheringManagerStatics4).
        if !self.restore_timeout
            && NetworkOperatorTetheringManager::IsNoConnectionsTimeoutEnabled().unwrap_or(false)
        {
            self.restore_timeout = wait_action(
                NetworkOperatorTetheringManager::DisableNoConnectionsTimeoutAsync(),
                SETTING_TIMEOUT,
            )
            .is_ok();
        }
    }

    fn restore_timeout(&mut self) {
        if std::mem::take(&mut self.restore_timeout) {
            let _ = wait_action(
                NetworkOperatorTetheringManager::EnableNoConnectionsTimeoutAsync(),
                SETTING_TIMEOUT,
            );
        }
    }
}

/// Wi-Fi adapters with their device names and `NetworkAdapterId`s, plus how
/// many Wi-Fi devices Windows lists (including any that couldn't be opened).
fn find_wifi() -> Result<(usize, Vec<Wifi>), ErrorKind> {
    let selector = WiFiAdapter::GetDeviceSelector().map_err(call)?;
    let infos = wait_op(
        DeviceInformation::FindAllAsyncAqsFilter(&selector),
        QUERY_TIMEOUT,
    )
    .map_err(|f| f.kind(Doing::Query))?;
    let mut devices = 0;
    let mut found: Vec<Wifi> = Vec::new();
    for i in 0..infos.Size().map_err(call)? {
        let Ok(info) = infos.GetAt(i) else { continue };
        let Ok(device_id) = info.Id() else { continue };
        let path = device_id.to_string_lossy();
        let name = info
            .Name()
            .map(|n| n.to_string_lossy())
            .ok()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Wi-Fi adapter".to_owned());
        if !info.IsEnabled().unwrap_or(true) || logic::is_virtual_wifi(&name, &path) {
            continue;
        }
        devices += 1;
        let Ok(wifi) = wait_op(WiFiAdapter::FromIdAsync(&device_id), QUERY_TIMEOUT) else {
            continue;
        };
        let Ok(adapter) = wifi.NetworkAdapter() else {
            continue;
        };
        let Ok(guid) = adapter.NetworkAdapterId() else {
            continue;
        };
        let id = guid_string(guid);
        if found.iter().any(|w| w.info.id == id) {
            continue;
        }
        found.push(Wifi {
            info: WifiAdapter {
                id,
                name,
                usb: logic::is_usb_device(&path),
                five_ghz: None,
            },
            adapter,
        });
    }
    found.sort_by(|a, b| {
        let key = |w: &Wifi| (w.info.name.to_lowercase(), w.info.id.clone());
        key(a).cmp(&key(b))
    });
    Ok((devices, found))
}

/// `IsBandSupported(5 GHz)` for a hotspot on this adapter (Windows 11).
fn supports_five_ghz(profile: &ConnectionProfile, adapter: &NetworkAdapter) -> Option<bool> {
    let manager = NetworkOperatorTetheringManager::CreateFromConnectionProfileWithTargetAdapter(
        profile, adapter,
    )
    .ok()?;
    let config = manager.GetCurrentAccessPointConfiguration().ok()?;
    config
        .IsBandSupported(TetheringWiFiBand::FiveGigahertz)
        .ok()
}

/// Connected profiles (with any connectivity), the internet one first,
/// leaving out the adapter `exclude` (the hotspot's own).
fn connected_profiles(exclude: Option<&str>) -> Vec<Profile> {
    let internet = NetworkInformation::GetInternetConnectionProfile()
        .ok()
        .and_then(|p| profile_adapter_id(&p));
    let Ok(all) = NetworkInformation::GetConnectionProfiles() else {
        return Vec::new();
    };
    let mut list: Vec<Profile> = items(&all)
        .into_iter()
        .filter_map(|profile| {
            let level = profile.GetNetworkConnectivityLevel().ok()?;
            if level == NetworkConnectivityLevel::None {
                return None;
            }
            let adapter = profile.NetworkAdapter().ok()?;
            let id = guid_string(adapter.NetworkAdapterId().ok()?);
            if exclude == Some(id.as_str()) {
                return None;
            }
            let kind = logic::uplink_kind(
                profile.IsWwanConnectionProfile().unwrap_or(false),
                profile.IsWlanConnectionProfile().unwrap_or(false),
                adapter.IanaInterfaceType().unwrap_or(0),
            );
            let uplink = Uplink {
                name: profile
                    .ProfileName()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default(),
                kind,
                internet: level == NetworkConnectivityLevel::InternetAccess,
                is_default: internet.as_deref() == Some(id.as_str()),
                id,
            };
            Some(Profile { profile, uplink })
        })
        .collect();
    list.sort_by(|a, b| logic::uplink_order(&a.uplink, &b.uplink));
    let mut seen = HashSet::new();
    list.retain(|p| seen.insert(p.uplink.id.clone()));
    list
}

fn profile_adapter_id(profile: &ConnectionProfile) -> Option<String> {
    let adapter = profile.NetworkAdapter().ok()?;
    Some(guid_string(adapter.NetworkAdapterId().ok()?))
}

/// `NetworkAdapterId` of the adapter that has this IPv4 address.
fn adapter_with_address(address: Ipv4Addr) -> Option<String> {
    let names = NetworkInformation::GetHostNames().ok()?;
    let wanted = address.to_string();
    items(&names).into_iter().find_map(|name| {
        if name.Type().ok()? != HostNameType::Ipv4
            || name.RawName().ok()?.to_string_lossy() != wanted
        {
            return None;
        }
        let adapter = name.IPInformation().ok()?.NetworkAdapter().ok()?;
        Some(guid_string(adapter.NetworkAdapterId().ok()?))
    })
}

/// ICS's hotspot-side address: `ScopeAddress` if someone changed it.
fn read_gateway() -> Ipv4Addr {
    windows_registry::LOCAL_MACHINE
        .open(ICS_PARAMETERS)
        .and_then(|key| key.get_string("ScopeAddress"))
        .ok()
        .and_then(|value| logic::parse_scope_address(&value))
        .unwrap_or(logic::ICS_GATEWAY)
}

fn read_clients(manager: &NetworkOperatorTetheringManager) -> Vec<Client> {
    let Ok(list) = manager.GetTetheringClients() else {
        return Vec::new();
    };
    clients::merge(items(&list).into_iter().filter_map(|client| {
        let mac = logic::normalize_mac(&client.MacAddress().ok()?.to_string_lossy());
        let mut found = Client {
            mac,
            ..Client::default()
        };
        let names = client.HostNames().map(|n| items(&n)).unwrap_or_default();
        for name in names {
            let (Ok(kind), Ok(raw)) = (name.Type(), name.RawName()) else {
                continue;
            };
            let raw = raw.to_string_lossy();
            if kind == HostNameType::DomainName && found.hostname.is_none() {
                found.hostname = logic::clean_hostname(&raw);
            } else if kind == HostNameType::Ipv4 && found.ip.is_none() {
                found.ip = raw.parse::<Ipv4Addr>().ok().map(IpAddr::V4);
            }
        }
        Some(found)
    }))
}

fn operational_state(manager: &NetworkOperatorTetheringManager) -> Op {
    manager
        .TetheringOperationalState()
        .map_or(Op::Unknown, |s| logic::op_from_winrt(s.0))
}

/// Gives Windows a few seconds to finish a transition, so the next request
/// isn't refused with "operation in progress".
fn wait_until_settled(manager: &NetworkOperatorTetheringManager) {
    let deadline = Instant::now() + SETTLE_TIMEOUT;
    while operational_state(manager) == Op::InTransition && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(250));
    }
}

fn set_band(
    config: &NetworkOperatorTetheringAccessPointConfiguration,
    band: Band,
) -> Result<(), ErrorKind> {
    let value = TetheringWiFiBand(logic::band_to_winrt(band));
    if band == Band::Auto {
        // Undoes an earlier choice; harmless if it fails.
        let _ = config.SetBand(value);
        return Ok(());
    }
    if matches!(config.IsBandSupported(value), Ok(false)) {
        return Err(logic::band_unsupported(band));
    }
    config.SetBand(value).map_err(call)
}

fn stop_tethering(manager: &NetworkOperatorTetheringManager) -> Result<(), ErrorKind> {
    let result =
        wait_op(manager.StopTetheringAsync(), STOP_TIMEOUT).map_err(|f| f.kind(Doing::Stop))?;
    match check_result(&result, true) {
        // A refusal that just means it was already off.
        Err(_) if operational_state(manager) == Op::Off => Ok(()),
        other => other,
    }
}

fn check_result(
    result: &NetworkOperatorTetheringOperationResult,
    stopping: bool,
) -> Result<(), ErrorKind> {
    let status = result.Status().map_err(call)?.0;
    let ok = if stopping {
        status == logic::status::SUCCESS
    } else {
        logic::start_succeeded(status)
    };
    if ok {
        return Ok(());
    }
    let additional = result
        .AdditionalErrorMessage()
        .map(|m| m.to_string_lossy())
        .unwrap_or_default();
    Err(ErrorKind::Activation(logic::explain_status(
        status,
        &additional,
        stopping,
    )))
}

fn call(e: windows::core::Error) -> ErrorKind {
    ErrorKind::Call(logic::call_text(&e.message(), e.code().0))
}

/// `E_INVALIDARG` while applying the name/password means Windows didn't
/// accept them.
fn rejected(e: windows::core::Error) -> ErrorKind {
    if e.code().0 == logic::hresult::E_INVALIDARG {
        logic::settings_rejected()
    } else {
        call(e)
    }
}

fn creation_error(e: windows::core::Error) -> ErrorKind {
    match logic::unavailable_from_hresult(e.code().0) {
        Some(message) => ErrorKind::Activation(Explanation {
            message: message.to_owned(),
            suggestion: Suggestion::None,
        }),
        None => call(e),
    }
}

fn guid_string(guid: GUID) -> String {
    logic::format_guid(guid.data1, guid.data2, guid.data3, guid.data4)
}

fn items<T: RuntimeType + 'static>(list: &IVectorView<T>) -> Vec<T> {
    let size = list.Size().unwrap_or(0);
    (0..size).filter_map(|i| list.GetAt(i).ok()).collect()
}

enum Failure {
    Call(windows::core::Error),
    TimedOut,
}

impl Failure {
    fn kind(self, doing: Doing) -> ErrorKind {
        match self {
            Failure::Call(e) => call(e),
            Failure::TimedOut => logic::timed_out(doing),
        }
    }
}

impl From<windows::core::Error> for Failure {
    fn from(e: windows::core::Error) -> Self {
        Failure::Call(e)
    }
}

/// Waits (at most `timeout`) for an async operation. Its completion handler
/// runs on a thread-pool thread and only signals; the results are read here.
fn wait_op<T: RuntimeType + 'static>(
    op: windows::core::Result<IAsyncOperation<T>>,
    timeout: Duration,
) -> Result<T, Failure> {
    let op = op?;
    let (done, finished) = mpsc::sync_channel(1);
    op.SetCompleted(&AsyncOperationCompletedHandler::new(move |_, _| {
        let _ = done.try_send(());
        Ok(())
    }))?;
    match finished.recv_timeout(timeout) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => Ok(op.GetResults()?),
        Err(RecvTimeoutError::Timeout) => {
            let _ = op.Cancel();
            Err(Failure::TimedOut)
        }
    }
}

/// [`wait_op`] for operations without a result.
fn wait_action(op: windows::core::Result<IAsyncAction>, timeout: Duration) -> Result<(), Failure> {
    let op = op?;
    let (done, finished) = mpsc::sync_channel(1);
    op.SetCompleted(&AsyncActionCompletedHandler::new(move |_, _| {
        let _ = done.try_send(());
        Ok(())
    }))?;
    match finished.recv_timeout(timeout) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => Ok(op.GetResults()?),
        Err(RecvTimeoutError::Timeout) => {
            let _ = op.Cancel();
            Err(Failure::TimedOut)
        }
    }
}
