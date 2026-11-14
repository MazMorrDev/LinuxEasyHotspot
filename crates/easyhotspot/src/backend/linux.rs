//! Linux backend: the NetworkManager controller from `leh-nm`, the client
//! watcher and the `leh-shaper` helper for per-device control.
//!
//! They run on a tokio runtime next to the UI's main loop; results come back
//! to the model as [`Msg::Backend`] messages.

use crate::i18n::{tr, trf};
use std::sync::OnceLock;
use std::time::Duration;

use leh_model::clients::Client;
use leh_model::{ErrorKind, HotspotConfig, HotspotState};
use leh_nm::{Controller, DeviceKind, Snapshot, clients};
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

use super::common::{self, HelperInstall, TrafficMeter};
use crate::msg::{Msg, Tx};
use crate::state::{
    AdapterProblem, AdapterVm, HelperStatus, Problem, RunState, Traffic, UiState, UplinkKind,
    UplinkVm,
};

/// What the Linux backend reports.
#[derive(Debug)]
pub enum Event {
    /// New controller snapshot.
    Snapshot(Box<Snapshot>),
    /// Connected clients changed.
    Clients(Vec<Client>),
    /// A debounced shaping update is due.
    ShapeFlush(u64),
    /// The helper answered an apply (true) or clear (false).
    ShapeResult(bool, Result<(), leh_nm::shaper_client::ShaperError>),
    /// Per-device byte counters.
    Counters(HashMap<String, (u64, u64)>),
    /// The hotspot interface's (rx, tx) byte counters.
    IfaceBytes(u64, u64),
}

fn emit(tx: &Tx, ev: Event) {
    tx.emit(Msg::Backend(ev));
}

pub fn runtime() -> &'static Runtime {
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
    controller: Controller,
    tx: Tx,
    clients_task: Option<JoinHandle<()>>,
    /// Polls the hotspot interface's byte counters while it runs.
    traffic_task: Option<JoinHandle<()>>,
    traffic: TrafficMeter,
    /// Error the user already dismissed; not shown again until it changes.
    dismissed: Option<HotspotState>,
    last: Snapshot,
    shaping: Shaping,
}

impl Live {
    pub fn start(tx: Tx) -> Self {
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
                emit(&fwd, Event::Snapshot(Box::new(snap)));
                if rx.changed().await.is_err() {
                    break;
                }
            }
        });
        Self {
            controller,
            tx,
            clients_task: None,
            traffic_task: None,
            traffic: TrafficMeter::default(),
            dismissed: None,
            last: Snapshot::default(),
            shaping: Shaping::default(),
        }
    }

    pub fn dismiss_error(&mut self) {
        self.dismissed = Some(self.last.state.clone());
    }

    pub fn stop_hotspot(&mut self) {
        self.controller.stop();
    }

    pub fn refresh(&self) {
        self.controller.refresh();
    }

    /// Handles an event; returns messages for the user.
    pub fn on_event(&mut self, st: &mut UiState, ev: Event) -> Vec<String> {
        match ev {
            Event::Snapshot(snap) => {
                self.apply(st, *snap);
                Vec::new()
            }
            Event::Clients(list) => common::merge_clients(st, &list)
                .into_iter()
                .map(|name| trf("“{name}” wants to join — see Clients", &[("name", &name)]))
                .collect(),
            Event::ShapeFlush(g) => {
                self.flush_shaping(g);
                Vec::new()
            }
            Event::ShapeResult(applied, res) => {
                self.on_shape_result(st, applied, res).into_iter().collect()
            }
            Event::Counters(map) => {
                self.on_counters(st, map);
                Vec::new()
            }
            Event::IfaceBytes(rx, tx) => {
                self.traffic.fold(st, rx, tx);
                Vec::new()
            }
        }
    }

    /// Keeps the helper in line with the view-model.
    pub fn sync(&mut self, st: &UiState) {
        self.sync_shaping(st);
    }

    pub fn helper_check(&self) {
        self.probe_helper();
    }

    /// Forget what was sent so the next sync re-applies (after "Authorize").
    pub fn helper_authorize(&mut self) {
        self.reset_shaping();
    }

    /// Folds a controller snapshot into the view-model.
    fn apply(&mut self, st: &mut UiState, snap: Snapshot) {
        st.backend_ready = true;
        if let Some(caps) = snap.caps {
            st.nm_version = caps.version.to_string();
            st.local_only_supported = caps.has_ipv4_forwarding;
        }
        st.adapters = snap
            .ap_devices()
            .map(|d| AdapterVm {
                product: d.product.clone(),
                ..AdapterVm::new(&d.iface, d.supports_5ghz(), d.is_usb())
            })
            .collect();
        st.auto_adapter = snap.pick_ap(None).map(|d| d.iface.clone());
        st.adapter_problems = adapter_problems(&snap);
        st.wifi_moved_to = snap.wifi_moved_to.as_deref().map(|iface| {
            let d = snap.devices.iter().find(|d| d.iface == iface);
            AdapterVm {
                product: d.and_then(|d| d.product.clone()),
                ..AdapterVm::new(iface, false, d.is_some_and(|d| d.is_usb()))
            }
        });
        st.ethernet = snap.ethernet_devices().map(|d| d.iface.clone()).collect();
        let ap_iface = snap
            .running
            .as_ref()
            .map(|r| r.ap_iface.clone())
            .or_else(|| st.cfg.ap_iface.clone())
            .or_else(|| st.auto_adapter.clone());
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
                            st.error = Some((
                                tr("Couldn't start the hotspot").into(),
                                crate::i18n::tr_dyn(&e.message),
                            ));
                        }
                        ErrorKind::Call(msg) => {
                            st.error =
                                Some((tr("NetworkManager reported an error").into(), msg.clone()));
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

        // Watch clients and traffic only while the hotspot is up.
        match (&snap.state, &snap.running) {
            (HotspotState::Active, Some(run)) if self.clients_task.is_none() => {
                let mut scope = run.client_scope();
                let tx = self.tx.clone();
                self.clients_task = Some(runtime().spawn(async move {
                    use futures_util::StreamExt as _;
                    scope.helper = system().await;
                    let mut w = clients::watch(scope, Duration::from_secs(3));
                    while let Some(list) = w.next().await {
                        emit(&tx, Event::Clients(list));
                    }
                }));
                self.start_traffic(run.ap_iface.clone());
            }
            (HotspotState::Active, _) => {}
            _ => {
                if let Some(t) = self.clients_task.take() {
                    t.abort();
                }
                if let Some(t) = self.traffic_task.take() {
                    t.abort();
                }
                self.traffic.reset();
                common::forget_session(st);
            }
        }
        self.last = snap;
    }

    pub fn start_hotspot(&mut self, cfg: &HotspotConfig) {
        self.dismissed = None;
        self.controller.start(cfg.clone());
    }

    /// Reads the AP interface's byte counters every 2 seconds. They count
    /// all hotspot traffic (every device, with or without the helper).
    fn start_traffic(&mut self, iface: String) {
        let tx = self.tx.clone();
        self.traffic.reset();
        self.traffic_task = Some(runtime().spawn(async move {
            loop {
                if let Some((rx, txb)) = iface_bytes(&iface).await {
                    emit(&tx, Event::IfaceBytes(rx, txb));
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }));
    }
}

/// `(rx_bytes, tx_bytes)` of a network interface.
async fn iface_bytes(iface: &str) -> Option<(u64, u64)> {
    let dir = std::path::Path::new("/sys/class/net")
        .join(iface)
        .join("statistics");
    let read = |f: &'static str| {
        let p = dir.join(f);
        async move {
            tokio::fs::read_to_string(p)
                .await
                .ok()?
                .trim()
                .parse::<u64>()
                .ok()
        }
    };
    Some((read("rx_bytes").await?, read("tx_bytes").await?))
}

/// Wi-Fi adapters and USB sticks that can't run the hotspot, and why.
fn adapter_problems(snap: &Snapshot) -> Vec<AdapterProblem> {
    let mut out = Vec::new();
    for d in snap.wifi_devices() {
        let supports_ap = matches!(
            d.kind,
            DeviceKind::Wifi {
                supports_ap: true,
                ..
            }
        );
        if !d.managed {
            out.push(AdapterProblem::Unmanaged {
                iface: d.iface.clone(),
            });
        } else if !supports_ap {
            out.push(AdapterProblem::NoApMode {
                iface: d.iface.clone(),
                usb: d.is_usb(),
                driver: d.driver.clone(),
            });
        } else if !d.is_usable() {
            out.push(AdapterProblem::Unavailable {
                iface: d.iface.clone(),
            });
        }
    }
    for i in &snap.usb_issues {
        let (name, id) = (i.name.clone(), i.id.clone());
        out.push(match i.problem {
            leh_model::usb::UsbWifiProblem::StorageMode => {
                AdapterProblem::UsbStorageMode { name, id }
            }
            leh_model::usb::UsbWifiProblem::NoDriver => AdapterProblem::UsbNoDriver { name, id },
        });
    }
    out
}

impl Live {
    /// View-model before the first snapshot arrives.
    pub fn initial_state(cfg: HotspotConfig) -> UiState {
        let mut st = crate::state::demo::initial();
        st.cfg = cfg;
        st.live = true;
        st.backend_ready = false;
        st.adapters.clear();
        st.auto_adapter = None;
        st.adapter_problems.clear();
        st.wifi_moved_to = None;
        st.traffic = Traffic::default();
        st.ethernet.clear();
        st.uplinks.clear();
        st.nm_version = "?".into();
        st.clients.clear();
        st.policies.clear();
        st.defaults = Default::default();
        st.helper = HelperStatus::Missing;
        st.tray_available = true;
        let store = crate::config_store::load_devices();
        st.defaults = store.new_devices;
        st.total_down_kbps = store.total_down_kbps;
        st.total_up_kbps = store.total_up_kbps;
        st.policies = store.devices;
        st.running_iface = None;
        st
    }
}

// ---------------------------------------------------------------------------
// Per-device shaping through the `leh-shaper` helper.

use std::collections::HashMap;
use std::time::Instant;

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
    /// Failed attempts at the current desired state.
    attempts: u32,
}

/// How often to retry an update the helper refused, and after how long.
/// A refusal is re-tried once before asking the user to authorize: it can
/// be a session that wasn't active for a moment (login, user switch).
const DENIED_RETRIES: u32 = 1;
/// "Not the hotspot interface yet/anymore": the hotspot is changing state.
const NOT_READY_RETRIES: u32 = 5;
const OTHER_RETRIES: u32 = 1;

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
        total_down_kbps: st.total_down_kbps,
        total_up_kbps: st.total_up_kbps,
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
    fn probe_helper(&self) {
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
    fn sync_shaping(&mut self, st: &UiState) {
        let d = desired(st);
        if d == self.shaping.wanted {
            return;
        }
        self.shaping.wanted = d;
        self.shaping.generation += 1;
        self.shaping.attempts = 0;
        let (tx, generation) = (self.tx.clone(), self.shaping.generation);
        runtime().spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            emit(&tx, Event::ShapeFlush(generation));
        });
        if self.shaping.wanted.is_none()
            && let Some(t) = self.shaping.counters_task.take()
        {
            t.abort();
        }
    }

    /// Forget what was sent so the next sync re-applies (e.g. after "Authorize").
    fn reset_shaping(&mut self) {
        self.shaping.wanted = None;
    }

    /// Sends the current desired state again after `delay`, unless it
    /// changes in the meantime.
    fn retry_shaping(&mut self, delay: Duration) {
        self.shaping.attempts += 1;
        let (tx, generation) = (self.tx.clone(), self.shaping.generation);
        runtime().spawn(async move {
            tokio::time::sleep(delay).await;
            emit(&tx, Event::ShapeFlush(generation));
        });
    }

    fn flush_shaping(&mut self, generation: u64) {
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
            emit(
                &tx,
                Event::ShapeResult(matches!(wanted, Desired::Apply(_)), res),
            );
        });
    }

    /// Handles the helper's answer. Returns a message for the user, if any.
    fn on_shape_result(
        &mut self,
        st: &mut UiState,
        applied: bool,
        res: Result<(), ShaperError>,
    ) -> Option<String> {
        let attempts = self.shaping.attempts;
        match res {
            Ok(()) => {
                self.shaping.attempts = 0;
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
            Err(ShaperError::Denied) if attempts < DENIED_RETRIES => {
                self.retry_shaping(Duration::from_secs(4));
                None
            }
            Err(ShaperError::Denied) => {
                st.helper = HelperStatus::Unauthorized;
                self.shaping.wanted = None;
                None
            }
            Err(ShaperError::NotReady) if attempts < NOT_READY_RETRIES => {
                self.retry_shaping(Duration::from_secs(2));
                None
            }
            Err(ShaperError::NotReady) => {
                eprintln!(
                    "shaping: the helper doesn't see the hotspot interface; giving up for now"
                );
                None
            }
            Err(ShaperError::Other(e)) if attempts < OTHER_RETRIES => {
                eprintln!("shaping: {e}; retrying");
                self.retry_shaping(Duration::from_secs(5));
                None
            }
            Err(ShaperError::Missing) => {
                st.helper = HelperStatus::Missing;
                self.shaping.wanted = None;
                None
            }
            Err(ShaperError::Other(e)) => Some(trf(
                "Couldn't apply speed limits: {error}",
                &[("error", &e)],
            )),
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
                    emit(&tx, Event::Counters(map));
                }
            }
        }));
    }

    /// Folds byte counters into per-device rates and session usage.
    fn on_counters(&mut self, st: &mut UiState, map: HashMap<String, (u64, u64)>) {
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

// ---------------------------------------------------------------------------
// Installing the helper from a source checkout.

/// The source checkout this binary was built from, if it still exists.
fn source_root() -> Option<std::path::PathBuf> {
    if packaged() {
        return None;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().ok()?;
    root.join("scripts/install-helper.sh")
        .exists()
        .then_some(root)
}

fn packaged() -> bool {
    crate::packaged()
}

/// How the helper can be installed, for the dialog.
pub fn helper_install() -> HelperInstall {
    HelperInstall {
        can_install: source_root().is_some() && !no_new_privs(),
        command: install_command(),
        no_new_privs: no_new_privs(),
    }
}

/// The terminal command shown in the "copy" box.
fn install_command() -> String {
    match source_root() {
        Some(r) => format!("cd '{}' && ./scripts/install-helper.sh", r.display()),
        None if packaged() => "sudo apt install --reinstall linuxeasyhotspot".into(),
        None => "./scripts/install-helper.sh".into(),
    }
}

/// True when this process can't gain privileges (sudo/pkexec will fail).
fn no_new_privs() -> bool {
    std::fs::read_to_string("/proc/self/status")
        .map(|s| {
            s.lines()
                .any(|l| l.starts_with("NoNewPrivs:") && l.trim_end().ends_with('1'))
        })
        .unwrap_or(false)
}

fn find_cargo() -> Option<std::path::PathBuf> {
    std::env::var_os("CARGO")
        .map(std::path::PathBuf::from)
        .into_iter()
        .chain(
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cargo/bin/cargo")),
        )
        .chain(["/usr/bin/cargo".into()])
        .find(|p| p.exists())
}

impl Live {
    /// Re-checks for the helper every few seconds until it is found.
    pub fn watch_helper(&self) {
        let tx = self.tx.clone();
        runtime().spawn(async move {
            loop {
                let found = match system().await {
                    Some(c) => shaper_client::probe(&c).await,
                    None => HelperProbe::Missing,
                };
                let status = match found {
                    HelperProbe::Ready { .. } => HelperStatus::Ready,
                    HelperProbe::TooOld => HelperStatus::TooOld,
                    HelperProbe::Missing => HelperStatus::Missing,
                };
                tx.emit(Msg::Helper(status));
                if status == HelperStatus::Ready {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    /// Builds the helper (as the user) and installs it with pkexec, which
    /// shows the desktop's password dialog. Reports via `Msg::InstallDone`.
    pub fn install_helper(&self) {
        let tx = self.tx.clone();
        runtime().spawn(async move {
            let res = async {
                let root = source_root().ok_or(tr("the source folder isn't available"))?;
                let cargo = find_cargo().ok_or(tr("cargo wasn't found"))?;
                tx.emit(Msg::InstallProgress(tr("Building the helper…").into()));
                let out = tokio::process::Command::new(&cargo)
                    .args(["build", "--release", "-p", "leh-shaper", "--manifest-path"])
                    .arg(root.join("Cargo.toml"))
                    .output()
                    .await
                    .map_err(|e| format!("cargo: {e}"))?;
                if !out.status.success() {
                    let err = String::from_utf8_lossy(&out.stderr);
                    let last = err
                        .lines()
                        .rev()
                        .find(|l| !l.trim().is_empty())
                        .unwrap_or(tr("build failed"));
                    return Err(format!("build failed: {last}"));
                }
                tx.emit(Msg::InstallProgress(
                    tr("Waiting for your password…").into(),
                ));
                let out = tokio::process::Command::new("pkexec")
                    .arg(root.join("scripts/install-helper.sh"))
                    .arg("--as-root")
                    .arg(root.join("target/release/leh-shaper"))
                    .output()
                    .await
                    .map_err(|e| format!("pkexec: {e}"))?;
                match out.status.code() {
                    Some(0) => Ok(()),
                    Some(126) => Err("cancelled".to_string()),
                    _ => {
                        let err = String::from_utf8_lossy(&out.stderr);
                        Err(err
                            .trim()
                            .lines()
                            .last()
                            .unwrap_or(tr("install failed"))
                            .to_owned())
                    }
                }
            }
            .await;
            tx.emit(Msg::InstallDone(res.map_err(|e: String| e)));
        });
    }
}
