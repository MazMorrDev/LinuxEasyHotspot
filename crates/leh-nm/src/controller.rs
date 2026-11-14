//! The hotspot controller: a single actor task that owns all NM interaction.
//!
//! Commands arrive over an mpsc channel and are handled one at a time, so a
//! double-clicked Start can never create two profiles. NM signals are folded
//! through [`leh_model::state::transition`] and the result is published on a
//! `watch` channel the UI subscribes to.

use std::net::Ipv4Addr;
use std::pin::Pin;

use futures_util::{Stream, StreamExt, stream};
use leh_model::state::{self, ac_state};
use leh_model::usb::UsbWifiIssue;
use leh_model::{
    ErrorKind, Event, HotspotConfig, HotspotState, Mode, NmCapabilities, NmVersion, ProfileIds,
    is_ap_profile_id, is_bridge_port_profile_id, is_bridge_profile_id, settings,
};
use tokio::sync::{mpsc, watch};
use zbus::Connection;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

use crate::clients::ClientScope;
use crate::devices::{self, DeviceInfo, dev_state};
use crate::profiles::{self, Profile};
use crate::proxies::{ActiveConnectionProxy, DeviceProxy, Ip4ConfigProxy, NetworkManagerProxy};

const NM_BUS_NAME: &str = "org.freedesktop.NetworkManager";

/// Details about the running hotspot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningInfo {
    pub ap_iface: String,
    pub mode: Mode,
    /// Interface holding the IP (AP iface, or the bridge).
    pub l3_iface: String,
    /// The AP's own IPv4 address in Router mode (dnsmasq listens there).
    pub gateway: Option<Ipv4Addr>,
    pub profile_uuid: String,
}

impl RunningInfo {
    pub fn client_scope(&self) -> ClientScope {
        ClientScope {
            ap_iface: self.ap_iface.clone(),
            l3_iface: self.l3_iface.clone(),
            dns_server: self.gateway,
            helper: None,
        }
    }
}

/// Everything the UI needs to render.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub state: HotspotState,
    pub caps: Option<NmCapabilities>,
    pub devices: Vec<DeviceInfo>,
    pub running: Option<RunningInfo>,
    /// USB Wi-Fi adapters plugged in but not usable yet (no driver, or
    /// still in "storage mode").
    pub usb_issues: Vec<UsbWifiIssue>,
    /// Adapter that took over this computer's Wi-Fi connection while the
    /// hotspot runs on the one it was on.
    pub wifi_moved_to: Option<String>,
}

impl Snapshot {
    pub fn ap_devices(&self) -> impl Iterator<Item = &DeviceInfo> {
        self.devices.iter().filter(|d| d.is_ap_capable())
    }

    /// Every Wi-Fi adapter, including ones that can't run a hotspot.
    pub fn wifi_devices(&self) -> impl Iterator<Item = &DeviceInfo> {
        self.devices.iter().filter(|d| d.is_wifi())
    }

    /// The adapter a start with `wanted` would use.
    pub fn pick_ap(&self, wanted: Option<&str>) -> Option<&DeviceInfo> {
        devices::pick_ap(&self.devices, wanted)
    }

    pub fn ethernet_devices(&self) -> impl Iterator<Item = &DeviceInfo> {
        self.devices.iter().filter(|d| d.is_ethernet())
    }

    /// Devices that could provide internet, excluding the AP device itself.
    pub fn uplinks(&self, ap_iface: Option<&str>) -> impl Iterator<Item = &DeviceInfo> {
        self.devices
            .iter()
            .filter(move |d| d.can_be_uplink() && Some(d.iface.as_str()) != ap_iface)
    }

    /// The device currently carrying this computer's internet (default route).
    pub fn default_uplink(&self) -> Option<&DeviceInfo> {
        self.devices.iter().find(|d| d.is_default)
    }
}

#[derive(Debug)]
enum Command {
    Start(Box<HotspotConfig>),
    Stop,
    Refresh,
}

/// Cheap, cloneable handle to the controller task.
#[derive(Clone)]
pub struct Controller {
    tx: mpsc::UnboundedSender<Command>,
    rx: watch::Receiver<Snapshot>,
}

impl Controller {
    /// Spawns the actor on the current tokio runtime.
    pub fn spawn() -> Self {
        let (tx, cmd_rx) = mpsc::unbounded_channel();
        let (out, rx) = watch::channel(Snapshot::default());
        tokio::spawn(Actor::new(out).run(cmd_rx));
        Self { tx, rx }
    }

    pub fn start(&self, cfg: HotspotConfig) {
        let _ = self.tx.send(Command::Start(Box::new(cfg)));
    }

    pub fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }

    /// Re-reads devices and NM state (also retries a lost NM connection).
    pub fn refresh(&self) {
        let _ = self.tx.send(Command::Refresh);
    }

    pub fn subscribe(&self) -> watch::Receiver<Snapshot> {
        self.rx.clone()
    }

    pub fn snapshot(&self) -> Snapshot {
        self.rx.borrow().clone()
    }
}

type EventStream = Pin<Box<dyn Stream<Item = Event> + Send>>;

struct BridgeSession {
    eth_device: OwnedObjectPath,
    /// Profile that was active on the Ethernet device before bridging.
    restore: Option<OwnedObjectPath>,
}

/// This computer's Wi-Fi connection, moved off the adapter that now runs the
/// hotspot so the computer stays online.
struct WifiHandoff {
    profile: OwnedObjectPath,
    /// The adapter it was on; it goes back there when the hotspot stops.
    from_device: OwnedObjectPath,
    /// Where it is now, once connected there.
    to_iface: Option<String>,
}

/// How long the other adapter gets to join the network before the hotspot
/// starts anyway.
const HANDOFF_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

struct Actor {
    conn: Option<Connection>,
    snap: Snapshot,
    out: watch::Sender<Snapshot>,
    /// Active-connection path of our AP profile while it exists.
    active: Option<OwnedObjectPath>,
    events: Option<EventStream>,
    nm_owner: Option<EventStream>,
    /// NM's DeviceAdded/DeviceRemoved (adapters plugged in or out).
    hotplug: Option<Pin<Box<dyn Stream<Item = ()> + Send>>>,
    bridge: Option<BridgeSession>,
    wifi: Option<WifiHandoff>,
}

async fn next_hotplug(s: &mut Option<Pin<Box<dyn Stream<Item = ()> + Send>>>) -> Option<()> {
    match s {
        Some(s) => s.next().await,
        None => std::future::pending().await,
    }
}

async fn next_or_pending(s: &mut Option<EventStream>) -> Option<Event> {
    match s {
        Some(s) => s.next().await,
        None => std::future::pending().await,
    }
}

fn call_error(e: impl std::fmt::Display) -> Event {
    Event::Failed(ErrorKind::Call(e.to_string()))
}

fn root() -> ObjectPath<'static> {
    ObjectPath::from_static_str_unchecked("/")
}

impl Actor {
    fn new(out: watch::Sender<Snapshot>) -> Self {
        Self {
            conn: None,
            snap: Snapshot::default(),
            out,
            active: None,
            events: None,
            nm_owner: None,
            hotplug: None,
            bridge: None,
            wifi: None,
        }
    }

    async fn run(mut self, mut cmds: mpsc::UnboundedReceiver<Command>) {
        self.init().await;
        loop {
            tokio::select! {
                cmd = cmds.recv() => match cmd {
                    Some(Command::Start(cfg)) => self.start(*cfg).await,
                    Some(Command::Stop) => self.stop().await,
                    Some(Command::Refresh) => self.refresh().await,
                    None => break,
                },
                Some(ev) = next_or_pending(&mut self.events) => self.apply(ev).await,
                Some(()) = next_hotplug(&mut self.hotplug) => {
                    // Let the new device settle (driver, NM state) first.
                    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
                    tracing::info!("network devices changed");
                    self.refresh().await;
                }
                Some(ev) = next_or_pending(&mut self.nm_owner) => {
                    let appeared = ev == Event::NmAppeared;
                    self.apply(ev).await;
                    if appeared {
                        self.refresh().await;
                    } else {
                        self.events = None;
                        self.active = None;
                    }
                }
            }
        }
    }

    fn publish(&self) {
        self.out.send_replace(self.snap.clone());
    }

    async fn apply(&mut self, ev: Event) {
        let before = self.snap.state.clone();
        tracing::trace!(?ev, "event");
        self.snap.state = state::transition(&before, ev);
        if self.snap.state != before {
            tracing::info!(from = ?before, to = ?self.snap.state, "hotspot state");
        }
        match &self.snap.state {
            HotspotState::Active if before != HotspotState::Active => self.fill_gateway().await,
            HotspotState::Idle | HotspotState::Error(_)
                if before.is_running() || before == HotspotState::Deactivating =>
            {
                self.events = None;
                self.active = None;
                self.snap.running = None;
                self.teardown_bridge().await;
                self.restore_wifi().await;
            }
            _ => {}
        }
        self.publish();
    }

    /// Connects to the system bus, reads capabilities and devices, and picks
    /// up a hotspot that is already running (e.g. the app was restarted).
    async fn init(&mut self) {
        let conn = match Connection::system().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "no system bus");
                self.apply(Event::NmVanished).await;
                return;
            }
        };
        if self.nm_owner.is_none() {
            self.nm_owner = watch_nm_owner(&conn).await;
        }
        if self.hotplug.is_none() {
            self.hotplug = watch_hotplug(&conn).await;
        }
        self.conn = Some(conn);
        self.refresh().await;
    }

    async fn refresh(&mut self) {
        let Some(conn) = self.conn.clone() else {
            return self.init_boxed().await;
        };
        let nm = match NetworkManagerProxy::new(&conn).await {
            Ok(nm) => nm,
            Err(e) => return self.apply(call_error(e)).await,
        };
        let version = match nm.version().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "NetworkManager not reachable");
                return self.apply(Event::NmVanished).await;
            }
        };
        if matches!(
            self.snap.state,
            HotspotState::Error(ErrorKind::NmUnavailable)
        ) {
            self.apply(Event::NmAppeared).await;
        }
        let caps = NmCapabilities::from_version(
            NmVersion::parse(&version).unwrap_or(NmVersion::new(0, 0, 0)),
        );
        self.snap.caps = Some(caps);
        if !caps.is_supported() {
            return self
                .apply(Event::Failed(ErrorKind::NmTooOld(version)))
                .await;
        }
        match devices::list(&conn).await {
            Ok(d) => self.snap.devices = d,
            Err(e) => tracing::warn!(error = %e, "listing devices failed"),
        }
        self.snap.usb_issues = tokio::task::spawn_blocking(crate::usb::wifi_issues)
            .await
            .unwrap_or_default();

        if self.active.is_none() {
            self.adopt_running(&conn, &nm).await;
        }
        if self.active.is_none() && self.snap.ap_devices().next().is_none() {
            return self.apply(Event::Failed(ErrorKind::NoApAdapter)).await;
        }
        if matches!(self.snap.state, HotspotState::Error(ErrorKind::NoApAdapter))
            && self.snap.ap_devices().next().is_some()
        {
            self.snap.state = HotspotState::Idle;
        }
        self.publish();
    }

    fn init_boxed(&mut self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(self.init())
    }

    /// Finds an already-active AP profile and starts watching it.
    async fn adopt_running(&mut self, conn: &Connection, nm: &NetworkManagerProxy<'_>) {
        let Ok(paths) = nm.active_connections().await else {
            return;
        };
        let mut bridge_active = false;
        let mut ours = None;
        for path in paths {
            let Ok(ac) = ActiveConnectionProxy::builder(conn).path(path.clone()) else {
                continue;
            };
            let Ok(ac) = ac.build().await else { continue };
            match ac.id().await.as_deref() {
                Ok(id) if is_ap_profile_id(id) => ours = Some((path, ac)),
                Ok(id) if is_bridge_profile_id(id) => bridge_active = true,
                _ => {}
            }
        }
        let Some((path, ac)) = ours else { return };
        let (Ok(uuid), Ok(devs)) = (ac.uuid().await, ac.devices().await) else {
            return;
        };
        let Some(dev) = devs
            .first()
            .and_then(|p| self.snap.devices.iter().find(|d| &d.path == p))
            .cloned()
        else {
            return;
        };
        let mode = if bridge_active {
            Mode::Bridge
        } else {
            Mode::Router
        };
        if bridge_active && let Some(eth) = self.snap.ethernet_devices().next() {
            self.bridge = Some(BridgeSession {
                eth_device: eth.path.clone(),
                restore: None,
            });
        }
        tracing::info!(iface = dev.iface, "found running hotspot");
        self.snap.running = Some(RunningInfo {
            ap_iface: dev.iface.clone(),
            mode,
            l3_iface: if bridge_active {
                leh_model::BRIDGE_IFACE.into()
            } else {
                dev.iface.clone()
            },
            gateway: None,
            profile_uuid: uuid,
        });
        self.watch_active(conn, path.clone(), dev.path.clone())
            .await;
        let st = ac.state().await.unwrap_or(ac_state::UNKNOWN);
        let ev = if st == ac_state::ACTIVATED {
            Event::AlreadyActive
        } else {
            Event::ActiveConnection {
                state: st,
                reason: 0,
            }
        };
        self.apply(ev).await;
    }

    async fn watch_active(
        &mut self,
        conn: &Connection,
        ac_path: OwnedObjectPath,
        dev_path: OwnedObjectPath,
    ) {
        let mut streams: Vec<EventStream> = Vec::new();
        if let Ok(b) = ActiveConnectionProxy::builder(conn).path(ac_path.clone())
            && let Ok(ac) = b.build().await
            && let Ok(s) = ac.receive_state_signal().await
        {
            streams.push(Box::pin(s.filter_map(|sig| async move {
                let a = sig.args().ok()?;
                Some(Event::ActiveConnection {
                    state: a.state,
                    reason: a.reason,
                })
            })));
        }
        if let Ok(b) = DeviceProxy::builder(conn).path(dev_path)
            && let Ok(dev) = b.build().await
            && let Ok(s) = dev.receive_state_signal().await
        {
            streams.push(Box::pin(s.filter_map(|sig| async move {
                let a = sig.args().ok()?;
                (a.new_state <= dev_state::UNAVAILABLE)
                    .then_some(Event::DeviceUnavailable { reason: a.reason })
            })));
        }
        self.active = Some(ac_path);
        self.events = Some(Box::pin(stream::select_all(streams)));
    }

    async fn fill_gateway(&mut self) {
        let (Some(conn), Some(run)) = (self.conn.clone(), self.snap.running.as_mut()) else {
            return;
        };
        if run.mode == Mode::Bridge {
            return;
        }
        let Some(dev) = self.snap.devices.iter().find(|d| d.iface == run.ap_iface) else {
            return;
        };
        run.gateway = async {
            let d = DeviceProxy::builder(&conn)
                .path(dev.path.clone())
                .ok()?
                .build()
                .await
                .ok()?;
            let cfg_path = d.ip4_config().await.ok()?;
            let ip4 = Ip4ConfigProxy::builder(&conn)
                .path(cfg_path)
                .ok()?
                .build()
                .await
                .ok()?;
            let data = ip4.address_data().await.ok()?;
            let addr: &str = data.first()?.get("address")?.downcast_ref().ok()?;
            addr.parse().ok()
        }
        .await;
    }

    #[tracing::instrument(skip_all, fields(mode = ?cfg.mode))]
    async fn start(&mut self, cfg: HotspotConfig) {
        if self.snap.state.is_running() || self.snap.state.is_busy() {
            tracing::debug!("start ignored, already running");
            return;
        }
        if let Err(errs) = cfg.validate() {
            let msg = errs
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n");
            return self.apply(Event::Failed(ErrorKind::Call(msg))).await;
        }
        self.refresh().await;
        let (Some(conn), Some(caps)) = (self.conn.clone(), self.snap.caps) else {
            return self.apply(Event::NmVanished).await;
        };
        if !caps.supports_mode(cfg.mode) {
            let msg = format!(
                "Local-only mode needs NetworkManager 1.52 or newer (you have {}).",
                caps.version
            );
            return self.apply(Event::Failed(ErrorKind::Call(msg))).await;
        }
        let Some(dev) = devices::pick_ap(&self.snap.devices, cfg.ap_iface.as_deref()).cloned()
        else {
            let kind = if cfg.ap_iface.is_some() {
                ErrorKind::DeviceUnavailable
            } else {
                ErrorKind::NoApAdapter
            };
            return self.apply(Event::Failed(kind)).await;
        };
        if !dev.is_usable() {
            return self
                .apply(Event::Failed(ErrorKind::DeviceUnavailable))
                .await;
        }
        self.apply(Event::StartRequested).await;
        if let Err(ev) = self.activate(&conn, &caps, cfg, &dev).await {
            self.teardown_bridge().await;
            self.restore_wifi().await;
            self.apply(ev).await;
        }
    }

    async fn activate(
        &mut self,
        conn: &Connection,
        caps: &NmCapabilities,
        mut cfg: HotspotConfig,
        dev: &DeviceInfo,
    ) -> Result<(), Event> {
        cfg.ap_iface = Some(dev.iface.clone());
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .map_err(|_| call_error("could not determine the current user name"))?;

        let find = |uuid: Option<String>, ids: Vec<&'static str>| async move {
            profiles::find(conn, uuid.as_deref(), &ids)
                .await
                .map_err(call_error)
        };
        let ap = find(cfg.profile_uuid.clone(), leh_model::ap_profile_ids()).await?;
        let (bridge, port) = if cfg.mode == Mode::Bridge {
            (
                find(None, leh_model::bridge_profile_ids()).await?,
                find(None, leh_model::bridge_port_profile_ids()).await?,
            )
        } else {
            (None, None)
        };
        let uuid_of = |p: &Option<Profile>| {
            p.as_ref()
                .map_or_else(|| uuid::Uuid::new_v4().to_string(), |p| p.uuid.clone())
        };
        let ids = ProfileIds {
            ap_uuid: uuid_of(&ap),
            bridge_uuid: uuid_of(&bridge),
            port_uuid: uuid_of(&port),
            user,
        };

        let nm = NetworkManagerProxy::new(conn).await.map_err(call_error)?;
        let mut l3_iface = dev.iface.clone();

        if cfg.mode == Mode::Bridge {
            let upstream = cfg.upstream_iface.as_deref().unwrap_or_default();
            let Some(eth) = self
                .snap
                .ethernet_devices()
                .find(|d| d.iface == upstream)
                .cloned()
            else {
                return Err(Event::Failed(ErrorKind::BridgeNeedsEthernet));
            };
            let restore = previous_profile(conn, &eth).await;
            // Keep this computer's place on the LAN: the bridge takes over the
            // Ethernet card's MAC (same DHCP lease, same IP) and the wired
            // profile's IP settings (static address, DNS…).
            let mut dict = profiles::to_dict(&settings::bridge_profile(
                &ids,
                Some(eth.hw_address.as_str()),
            ));
            if let Some(path) = &restore
                && let Some(ip) = profiles::ip_settings(conn, path).await
            {
                tracing::info!("bridge reuses the wired connection's IP settings");
                dict.extend(ip);
            }
            let bridge_path = profiles::upsert_dict(conn, dict, bridge.as_ref())
                .await
                .map_err(call_error)?;
            let port_path = profiles::upsert(
                conn,
                &settings::bridge_port_profile(&eth.iface, &ids),
                port.as_ref(),
            )
            .await
            .map_err(call_error)?;
            self.bridge = Some(BridgeSession {
                eth_device: eth.path.clone(),
                restore,
            });
            nm.activate_connection(&bridge_path.as_ref(), &root(), &root())
                .await
                .map_err(call_error)?;
            nm.activate_connection(&port_path.as_ref(), &eth.path.as_ref(), &root())
                .await
                .map_err(call_error)?;
            l3_iface = leh_model::BRIDGE_IFACE.into();
        }

        // The adapter is this computer's own Wi-Fi connection: starting the
        // hotspot on it would drop that connection, so move it to another
        // adapter first if there is one.
        if dev.is_connected() && dev.active_connection.as_str() != "/" {
            self.wifi = hand_off_wifi(
                conn,
                &nm,
                dev,
                &self.snap.devices,
                cfg.upstream_iface.as_deref(),
            )
            .await;
            self.snap.wifi_moved_to = self.wifi.as_ref().and_then(|h| h.to_iface.clone());
            self.publish();
        }

        // Router/Local-only: make sure the chosen internet source is up. Which
        // connection hotspot traffic actually uses still follows the default
        // route; routing only the hotspot through it needs the helper.
        if cfg.mode != Mode::Bridge
            && self.wifi.is_none()
            && let Some(up) = cfg.upstream_iface.as_deref()
            && let Some(dev) = self.snap.devices.iter().find(|d| d.iface == up)
            && !dev.is_connected()
        {
            tracing::info!(iface = up, "connecting chosen internet source");
            if let Err(e) = nm
                .activate_connection(&root(), &dev.path.as_ref(), &root())
                .await
            {
                tracing::warn!(error = %e, iface = up, "could not connect internet source");
            }
        }

        let ap_path = profiles::upsert(conn, &settings::ap_profile(&cfg, caps, &ids), ap.as_ref())
            .await
            .map_err(call_error)?;
        let ac = nm
            .activate_connection(&ap_path.as_ref(), &dev.path.as_ref(), &root())
            .await
            .map_err(call_error)?;

        self.snap.running = Some(RunningInfo {
            ap_iface: dev.iface.clone(),
            mode: cfg.mode,
            l3_iface,
            gateway: None,
            profile_uuid: ids.ap_uuid,
        });
        self.watch_active(conn, ac.clone(), dev.path.clone()).await;
        // Catch up on a state change that happened before we subscribed.
        if let Ok(b) = ActiveConnectionProxy::builder(conn).path(ac)
            && let Ok(p) = b.build().await
            && let Ok(st) = p.state().await
        {
            self.apply(Event::ActiveConnection {
                state: st,
                reason: 0,
            })
            .await;
        }
        Ok(())
    }

    async fn stop(&mut self) {
        let Some(conn) = self.conn.clone() else {
            return;
        };
        let Some(ac) = self.active.clone() else {
            if self.snap.state.is_running() {
                self.snap.state = HotspotState::Idle;
                self.publish();
            }
            return;
        };
        // Send the request before publishing "Deactivating", so a UI that quits
        // as soon as it sees the stop acknowledged can't drop the call.
        let res = async {
            NetworkManagerProxy::new(&conn)
                .await?
                .deactivate_connection(&ac.as_ref())
                .await
        }
        .await;
        match res {
            Ok(()) => self.apply(Event::StopRequested).await,
            Err(e) => {
                tracing::warn!(error = %e, "deactivate failed");
                // The connection may already be gone; treat as stopped.
                self.apply(Event::ActiveConnection {
                    state: ac_state::DEACTIVATED,
                    reason: 2,
                })
                .await;
            }
        }
    }

    /// Deactivates our bridge (if any) and gives the Ethernet device back its
    /// previous profile. Safe to call when nothing is bridged.
    async fn teardown_bridge(&mut self) {
        let (Some(conn), Some(session)) = (self.conn.clone(), self.bridge.take()) else {
            return;
        };
        tracing::info!("restoring Ethernet after bridge");
        let Ok(nm) = NetworkManagerProxy::new(&conn).await else {
            return;
        };
        if let Ok(paths) = nm.active_connections().await {
            for path in paths {
                let Ok(b) = ActiveConnectionProxy::builder(&conn).path(path.clone()) else {
                    continue;
                };
                let Ok(ac) = b.build().await else { continue };
                if ac
                    .id()
                    .await
                    .is_ok_and(|id| is_bridge_profile_id(&id) || is_bridge_port_profile_id(&id))
                {
                    let _ = nm.deactivate_connection(&path.as_ref()).await;
                }
            }
        }
        // "/" lets NM pick the best profile for the device.
        let restore = session
            .restore
            .unwrap_or_else(|| OwnedObjectPath::from(root()));
        if let Err(e) = nm
            .activate_connection(&restore.as_ref(), &session.eth_device.as_ref(), &root())
            .await
        {
            tracing::warn!(error = %e, "could not restore Ethernet profile");
        }
    }

    /// Gives the Wi-Fi connection moved by [`hand_off_wifi`] back to the
    /// adapter it was on. Safe to call when nothing was moved.
    async fn restore_wifi(&mut self) {
        self.snap.wifi_moved_to = None;
        let (Some(conn), Some(h)) = (self.conn.clone(), self.wifi.take()) else {
            return;
        };
        tracing::info!("moving the Wi-Fi connection back");
        let Ok(nm) = NetworkManagerProxy::new(&conn).await else {
            return;
        };
        if let Err(e) = nm
            .activate_connection(&h.profile.as_ref(), &h.from_device.as_ref(), &root())
            .await
        {
            tracing::warn!(error = %e, "could not move the Wi-Fi connection back");
        }
    }
}

/// Moves the Wi-Fi connection active on `ap` to another free Wi-Fi adapter
/// and waits (up to [`HANDOFF_TIMEOUT`]) until it is connected there. NM
/// deactivates it on `ap` by itself, since a profile is active on one device
/// at a time.
async fn hand_off_wifi(
    conn: &Connection,
    nm: &NetworkManagerProxy<'_>,
    ap: &DeviceInfo,
    devices: &[DeviceInfo],
    wanted: Option<&str>,
) -> Option<WifiHandoff> {
    let ac = ActiveConnectionProxy::builder(conn)
        .path(ap.active_connection.clone())
        .ok()?
        .build()
        .await
        .ok()?;
    if is_ap_profile_id(&ac.id().await.ok()?) {
        return None;
    }
    let profile = ac.connection().await.ok()?;
    let Some(target) = devices::pick_wifi_client(devices, &ap.iface, wanted) else {
        tracing::info!(
            iface = ap.iface,
            "the hotspot adapter is this computer's Wi-Fi connection and no other adapter can take it over"
        );
        return None;
    };
    tracing::info!(
        from = ap.iface,
        to = target.iface,
        "moving the Wi-Fi connection"
    );
    if let Err(e) = nm
        .activate_connection(&profile.as_ref(), &target.path.as_ref(), &root())
        .await
    {
        // E.g. the profile is locked to the first adapter's MAC address.
        tracing::warn!(error = %e, to = target.iface, "could not move the Wi-Fi connection");
        return None;
    }
    let mut handoff = WifiHandoff {
        profile,
        from_device: ap.path.clone(),
        to_iface: None,
    };
    let dev = DeviceProxy::builder(conn)
        .path(target.path.clone())
        .ok()?
        .build()
        .await
        .ok()?;
    let joined = tokio::time::timeout(HANDOFF_TIMEOUT, async {
        let since = std::time::Instant::now();
        let mut trying = false;
        loop {
            match dev.state().await {
                Ok(dev_state::ACTIVATED) => return true,
                // Back to disconnected after trying (or never started): failed.
                Ok(s) if s <= dev_state::DISCONNECTED => {
                    if trying || since.elapsed() > std::time::Duration::from_secs(5) {
                        return false;
                    }
                }
                Ok(_) => trying = true,
                Err(_) => return false,
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }
    })
    .await
    .unwrap_or(false);
    if joined {
        tracing::info!(iface = target.iface, "Wi-Fi connection moved");
        handoff.to_iface = Some(target.iface.clone());
    } else {
        tracing::warn!(iface = target.iface, "the other adapter didn't connect");
    }
    // Either way the connection left `ap`; give it back when the hotspot stops.
    Some(handoff)
}

/// The saved profile active on `dev` before we took it over, unless it is ours.
async fn previous_profile(conn: &Connection, dev: &DeviceInfo) -> Option<OwnedObjectPath> {
    if dev.active_connection.as_str() == "/" {
        return None;
    }
    let ac = ActiveConnectionProxy::builder(conn)
        .path(dev.active_connection.clone())
        .ok()?
        .build()
        .await
        .ok()?;
    if is_bridge_port_profile_id(&ac.id().await.ok()?) {
        return None;
    }
    ac.connection().await.ok()
}

async fn watch_hotplug(conn: &Connection) -> Option<Pin<Box<dyn Stream<Item = ()> + Send>>> {
    let nm = NetworkManagerProxy::new(conn).await.ok()?;
    let added = nm.receive_device_added().await.ok()?.map(|_| ());
    let removed = nm.receive_device_removed().await.ok()?.map(|_| ());
    Some(Box::pin(stream::select(added, removed)))
}

async fn watch_nm_owner(conn: &Connection) -> Option<EventStream> {
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let s = dbus
        .receive_name_owner_changed_with_args(&[(0, NM_BUS_NAME)])
        .await
        .ok()?;
    Some(Box::pin(s.filter_map(|sig| async move {
        let a = sig.args().ok()?;
        Some(if a.new_owner.is_some() {
            Event::NmAppeared
        } else {
            Event::NmVanished
        })
    })))
}
