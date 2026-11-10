//! Live mode: the real NetworkManager controller from `leh-nm`.
//!
//! The controller and client watcher run on a tokio runtime next to GTK's
//! main loop; snapshots are forwarded to the UI as messages.

use std::sync::OnceLock;
use std::time::Duration;

use leh_model::clients::Client;
use leh_model::{ErrorKind, HotspotConfig, HotspotState};
use leh_nm::{Controller, DeviceKind, Snapshot, clients};
use relm4::Sender;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use crate::app::Msg;
use crate::state::{
    ClientVm, DeviceIcon, HelperStatus, Problem, RunState, UiState, UplinkKind, UplinkVm,
};

fn runtime() -> &'static Runtime {
    static RT: OnceLock<Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

pub struct Live {
    pub controller: Controller,
    tx: Sender<Msg>,
    clients_task: Option<JoinHandle<()>>,
    /// Error the user already dismissed; not shown again until it changes.
    dismissed: Option<HotspotState>,
    last: Snapshot,
    shaping: Shaping,
}

impl Live {
    pub fn start(tx: Sender<Msg>) -> Self {
        let rt = runtime();
        let controller = {
            let _guard = rt.enter();
            Controller::spawn()
        };
        let mut rx = controller.subscribe();
        let fwd = tx.clone();
        rt.spawn(async move {
            loop {
                let snap = rx.borrow_and_update().clone();
                fwd.emit(Msg::Snapshot(Box::new(snap)));
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });
        Self {
            controller,
            tx,
            clients_task: None,
            dismissed: None,
            last: Snapshot::default(),
            shaping: Shaping::default(),
        }
    }

    pub fn dismiss_error(&mut self) {
        self.dismissed = Some(self.last.state.clone());
    }

    /// Folds a controller snapshot into the view-model.
    pub fn apply(&mut self, st: &mut UiState, snap: Snapshot) {
        if let Some(caps) = snap.caps {
            st.nm_version = caps.version.to_string();
            st.local_only_supported = caps.has_ipv4_forwarding;
        }
        st.adapters = snap
            .ap_devices()
            .map(|d| (d.iface.clone(), d.supports_5ghz()))
            .collect();
        st.ethernet = snap.ethernet_devices().map(|d| d.iface.clone()).collect();
        let ap_iface = snap
            .running
            .as_ref()
            .map(|r| r.ap_iface.clone())
            .or_else(|| st.cfg.ap_iface.clone())
            .or_else(|| st.adapters.first().map(|(i, _)| i.clone()));
        let mut uplinks: Vec<UplinkVm> = snap
            .uplinks(ap_iface.as_deref())
            .map(|d| UplinkVm {
                iface: d.iface.clone(),
                kind: match d.kind {
                    DeviceKind::Ethernet => UplinkKind::Ethernet,
                    DeviceKind::Wifi { .. } => UplinkKind::Wifi,
                    DeviceKind::Modem => UplinkKind::Modem,
                },
                connection: d.connection_name.clone(),
                connected: d.is_connected(),
                available: d.is_usable(),
                is_default: d.is_default,
            })
            .collect();
        uplinks.sort_by_key(|u| (u.kind as u8, u.iface.clone()));
        st.uplinks = uplinks;

        let dismissed = self.dismissed.as_ref() == Some(&snap.state);
        st.problem = None;
        st.run = match &snap.state {
            HotspotState::Idle => RunState::Off,
            HotspotState::Activating => RunState::Starting,
            HotspotState::Active => RunState::On,
            HotspotState::Deactivating => RunState::Stopping,
            HotspotState::Error(kind) => {
                if !dismissed {
                    match kind {
                        ErrorKind::NmUnavailable | ErrorKind::NmTooOld(_) => {
                            st.problem = Some(Problem::NmMissing);
                        }
                        ErrorKind::NoApAdapter => st.problem = Some(Problem::NoAdapter),
                        ErrorKind::BridgeNeedsEthernet => {
                            st.problem = Some(Problem::BridgeNoEthernet)
                        }
                        ErrorKind::DeviceUnavailable => st.problem = Some(Problem::AdapterGone),
                        ErrorKind::Activation(e) => {
                            st.error =
                                Some(("Couldn't start the hotspot".into(), e.message.clone()));
                        }
                        ErrorKind::Call(msg) => {
                            st.error =
                                Some(("NetworkManager reported an error".into(), msg.clone()));
                        }
                    }
                }
                RunState::Off
            }
        };
        if st.run != RunState::Off {
            st.error = None;
        }

        st.running_iface = snap.running.as_ref().map(|r| r.ap_iface.clone());

        // Remember which profile is ours so we find it again next time.
        if let Some(run) = &snap.running
            && st.cfg.profile_uuid.as_deref() != Some(run.profile_uuid.as_str())
        {
            st.cfg.profile_uuid = Some(run.profile_uuid.clone());
            crate::config_store::save(&st.cfg);
        }

        // Watch clients only while the hotspot is up.
        match (&snap.state, &snap.running) {
            (HotspotState::Active, Some(run)) if self.clients_task.is_none() => {
                let scope = run.client_scope();
                let tx = self.tx.clone();
                self.clients_task = Some(runtime().spawn(async move {
                    use futures_util::StreamExt as _;
                    let mut w = clients::watch(scope, Duration::from_secs(3));
                    while let Some(list) = w.next().await {
                        tx.emit(Msg::Clients(list));
                    }
                }));
            }
            (HotspotState::Active, _) => {}
            _ => {
                if let Some(t) = self.clients_task.take() {
                    t.abort();
                }
                st.clients.clear();
            }
        }
        self.last = snap;
    }

    pub fn start_hotspot(&mut self, cfg: &HotspotConfig) {
        self.dismissed = None;
        self.controller.start(cfg.clone());
    }
}

fn guess_icon(c: &Client) -> DeviceIcon {
    let h = c
        .hostname
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| h.contains(w));
    if any(&[
        "iphone", "ipad", "pixel", "galaxy", "android", "phone", "redmi", "moto", "oneplus",
    ]) {
        DeviceIcon::Phone
    } else if any(&[
        "laptop", "book", "pc", "desktop", "thinkpad", "mac", "ubuntu", "mint",
    ]) {
        DeviceIcon::Laptop
    } else if any(&["tv", "roku", "chromecast", "firetv"]) {
        DeviceIcon::Tv
    } else {
        DeviceIcon::Unknown
    }
}

pub fn client_vm(c: &Client) -> ClientVm {
    ClientVm {
        mac: c.mac.clone(),
        hostname: c.hostname.clone(),
        ip: c.ip.map(|ip| ip.to_string()),
        vendor: c.vendor(),
        icon: guess_icon(c),
        down_kbps: 0.0,
        up_kbps: 0.0,
        session_bytes: 0,
        demand_kbps: (0.0, 0.0),
    }
}

/// View-model before the first snapshot arrives.
pub fn initial_state(cfg: HotspotConfig) -> UiState {
    let mut st = crate::state::demo::initial();
    st.cfg = cfg;
    st.live = true;
    st.adapters.clear();
    st.ethernet.clear();
    st.uplinks.clear();
    st.nm_version = "?".into();
    st.clients.clear();
    st.policies.clear();
    st.defaults = Default::default();
    st.helper = HelperStatus::Missing;
    st.tray_available = false;
    let store = crate::config_store::load_devices();
    st.defaults = store.new_devices;
    st.total_down_mbps = store.total_down_mbps;
    st.total_up_mbps = store.total_up_mbps;
    st.policies = store.devices;
    st.running_iface = None;
    st
}

// ---------------------------------------------------------------------------
// Per-device shaping through the `leh-shaper` helper.

use std::collections::HashMap;
use std::time::Instant;

use leh_model::policy::Access;
use leh_model::tc::{self, ShapeDevice, ShapeState};
use leh_nm::shaper_client::{self, HelperProbe, ShaperError};

async fn system() -> Option<zbus::Connection> {
    static CONN: tokio::sync::OnceCell<Option<zbus::Connection>> =
        tokio::sync::OnceCell::const_new();
    CONN.get_or_init(|| async { zbus::Connection::system().await.ok() })
        .await
        .clone()
}

/// What the helper should be doing right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Desired {
    Apply(ShapeState),
    Clear(String),
}

#[derive(Default)]
pub struct Shaping {
    wanted: Option<Desired>,
    generation: u64,
    counters_task: Option<JoinHandle<()>>,
    last_raw: HashMap<String, (u64, u64)>,
    last_sample: Option<Instant>,
}

/// Computes the desired helper state from the view-model.
pub fn desired(st: &UiState) -> Option<Desired> {
    if !st.live || st.helper != HelperStatus::Ready || st.run != RunState::On {
        return None;
    }
    let iface = st.running_iface.clone()?;
    let mut any_cap = false;
    let devices = st
        .clients
        .iter()
        .filter(|c| tc::valid_mac(&c.mac))
        .map(|c| {
            let pol = st.policy(&c.mac);
            any_cap |= pol.limits(&st.defaults).cap_mb.is_some();
            let e = st.effective(c);
            ShapeDevice {
                mac: c.mac.clone(),
                access: e.access,
                down_kbps: e.down_kbps,
                up_kbps: e.up_kbps,
                priority: e.priority,
            }
        })
        .collect();
    let state = ShapeState {
        iface: iface.clone(),
        total_down_kbps: st.total_down_mbps.map(|m| m * 1000),
        total_up_kbps: st.total_up_mbps.map(|m| m * 1000),
        devices,
    };
    // Data caps need the counters, which need the HTB classes.
    if tc::is_noop(&state) && !any_cap {
        Some(Desired::Clear(iface))
    } else {
        Some(Desired::Apply(state))
    }
}

impl Live {
    pub fn probe_helper(&self) {
        let tx = self.tx.clone();
        runtime().spawn(async move {
            let status = match system().await {
                Some(c) => match shaper_client::probe(&c).await {
                    HelperProbe::Ready { .. } => HelperStatus::Ready,
                    HelperProbe::TooOld => HelperStatus::TooOld,
                    HelperProbe::Missing => HelperStatus::Missing,
                },
                None => HelperStatus::Missing,
            };
            tx.emit(Msg::Helper(status));
        });
    }

    /// Schedules an update if the desired state changed (debounced 300 ms).
    pub fn sync_shaping(&mut self, st: &UiState) {
        let d = desired(st);
        if d == self.shaping.wanted {
            return;
        }
        self.shaping.wanted = d;
        self.shaping.generation += 1;
        let (tx, generation) = (self.tx.clone(), self.shaping.generation);
        runtime().spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            tx.emit(Msg::ShapeFlush(generation));
        });
        if self.shaping.wanted.is_none()
            && let Some(t) = self.shaping.counters_task.take()
        {
            t.abort();
        }
    }

    /// Forget what was sent so the next sync re-applies (e.g. after "Authorize").
    pub fn reset_shaping(&mut self) {
        self.shaping.wanted = None;
    }

    pub fn flush_shaping(&mut self, generation: u64) {
        if generation != self.shaping.generation {
            return;
        }
        let Some(wanted) = self.shaping.wanted.clone() else {
            return;
        };
        let tx = self.tx.clone();
        runtime().spawn(async move {
            let Some(conn) = system().await else {
                return;
            };
            let res = match &wanted {
                Desired::Apply(s) => {
                    shaper_client::apply(
                        &conn,
                        &s.iface,
                        s.total_down_kbps,
                        s.total_up_kbps,
                        &s.devices,
                    )
                    .await
                }
                Desired::Clear(iface) => shaper_client::clear(&conn, iface).await,
            };
            tx.emit(Msg::ShapeResult(matches!(wanted, Desired::Apply(_)), res));
        });
    }

    /// Handles the helper's answer. Returns a message for the user, if any.
    pub fn on_shape_result(
        &mut self,
        st: &mut UiState,
        applied: bool,
        res: Result<(), ShaperError>,
    ) -> Option<String> {
        match res {
            Ok(()) => {
                self.shaping.last_raw.clear();
                self.shaping.last_sample = Some(Instant::now());
                if applied {
                    self.start_counters(st);
                } else if let Some(t) = self.shaping.counters_task.take() {
                    t.abort();
                    for c in &mut st.clients {
                        c.down_kbps = 0.0;
                        c.up_kbps = 0.0;
                    }
                }
                None
            }
            Err(ShaperError::Denied) => {
                st.helper = HelperStatus::Unauthorized;
                self.shaping.wanted = None;
                None
            }
            Err(ShaperError::Missing) => {
                st.helper = HelperStatus::Missing;
                self.shaping.wanted = None;
                None
            }
            Err(ShaperError::Other(e)) => Some(format!("Couldn't apply speed limits: {e}")),
        }
    }

    fn start_counters(&mut self, st: &UiState) {
        if self.shaping.counters_task.is_some() {
            return;
        }
        let Some(iface) = st.running_iface.clone() else {
            return;
        };
        let tx = self.tx.clone();
        self.shaping.counters_task = Some(runtime().spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3)).await;
                let Some(conn) = system().await else { continue };
                if let Ok(map) = shaper_client::counters(&conn, &iface).await {
                    tx.emit(Msg::Counters(map));
                }
            }
        }));
    }

    /// Folds byte counters into per-device rates and session usage.
    pub fn on_counters(&mut self, st: &mut UiState, map: HashMap<String, (u64, u64)>) {
        let now = Instant::now();
        let secs = self
            .shaping
            .last_sample
            .map_or(3.0, |t| now.duration_since(t).as_secs_f64())
            .max(0.5);
        self.shaping.last_sample = Some(now);
        for c in &mut st.clients {
            let Some(&(d, u)) = map.get(&c.mac) else {
                c.down_kbps = 0.0;
                c.up_kbps = 0.0;
                continue;
            };
            let (ld, lu) = self.shaping.last_raw.get(&c.mac).copied().unwrap_or((0, 0));
            // Counters restart from 0 whenever the rules are re-applied.
            let dd = if d >= ld { d - ld } else { d };
            let du = if u >= lu { u - lu } else { u };
            c.session_bytes += dd + du;
            c.down_kbps = dd as f64 * 8.0 / 1000.0 / secs;
            c.up_kbps = du as f64 * 8.0 / 1000.0 / secs;
            self.shaping.last_raw.insert(c.mac.clone(), (d, u));
        }
    }
}

/// Merges a fresh client list into the view-model, keeping usage stats and
/// creating policies for devices seen for the first time. Returns the names
/// of new devices that are waiting for approval.
pub fn merge_clients(st: &mut UiState, list: &[Client]) -> Vec<String> {
    let mut waiting = Vec::new();
    let old: HashMap<String, ClientVm> = st.clients.drain(..).map(|c| (c.mac.clone(), c)).collect();
    for c in list {
        let mut vm = client_vm(c);
        if let Some(prev) = old.get(&vm.mac) {
            vm.session_bytes = prev.session_bytes;
            vm.down_kbps = prev.down_kbps;
            vm.up_kbps = prev.up_kbps;
        }
        if !st.policies.contains_key(&vm.mac) {
            let p = leh_model::policy::DevicePolicy::new_device(&st.defaults);
            if p.access == Access::Pending && st.helper == HelperStatus::Ready {
                waiting.push(st.display_name(&vm));
            }
            st.policies.insert(vm.mac.clone(), p);
        }
        st.clients.push(vm);
    }
    waiting
}
