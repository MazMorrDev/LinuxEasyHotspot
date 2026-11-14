//! The app's state and logic, independent of any window: the hotspot keeps
//! being managed (tray, per-device rules) while the window is closed.
//!
//! Every [`Msg`] goes through [`Model::dispatch`], which updates the
//! view-model, records the session log, saves settings and tells the
//! backend. Things only a window can show (toasts, dialogs) are emitted as
//! [`ModelEvent`]s.

use std::time::Duration;

use gpui_kit::{App, ClipboardItem, Context, EventEmitter};
use leh_model::policy::{Access, DevicePolicy};
use leh_model::{Band, Mode, password};

use crate::backend::Live;
use crate::i18n::{tr, trf};
use crate::msg::{Msg, Page, Tx};
use crate::state::demo::{self, Scenario};
use crate::state::{HelperStatus, Problem, RunState, UiState};

/// A question or message that needs a dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// "Switch to Bridge mode?"
    Bridge,
    /// How to install or update the helper.
    Helper,
    ErrorDetails {
        title: String,
        details: String,
    },
    Restart,
    About,
    InstallFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelEvent {
    Toast(String),
    Ask(Ask),
    /// Bring the window up (open it again if it was closed).
    ShowWindow,
}

pub struct Args {
    pub demo: bool,
    pub page: Option<String>,
    pub running: bool,
    pub scenario: Option<String>,
    pub device: Option<String>,
    /// Started at login: stay in the background, maybe start the hotspot.
    pub hidden: bool,
}

pub struct Model {
    pub state: UiState,
    pub page: Page,
    next_start_fails: bool,
    /// `None` in `--demo` mode.
    live: Option<Live>,
    tray: Option<crate::tray::TrayHandle>,
    /// Started at login with "Start hotspot at login": start once ready.
    pub start_pending: bool,
    tx: Tx,
}

impl EventEmitter<ModelEvent> for Model {}

impl Model {
    pub fn new(args: &Args, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = async_channel::unbounded::<Msg>();
        let tx = Tx(tx);
        // Messages from other threads (backend, tray) are handled here, on
        // the UI thread.
        cx.spawn(async move |this, cx| {
            while let Ok(msg) = rx.recv().await {
                if this.update(cx, |m, cx| m.dispatch(msg, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let live = (!args.demo).then(|| Live::start(tx.clone()));
        if let Some(l) = &live {
            l.watch_helper();
        }
        let mut state = if live.is_some() {
            Live::initial_state(crate::config_store::load())
        } else {
            demo::initial()
        };
        if args.running && live.is_none() {
            state.run = RunState::On;
            demo::tick(&mut state);
            demo::seed_events(&mut state);
        }
        let start_pending = args.hidden && live.is_some() && state.cfg.start_at_login;
        if live.is_some() {
            // Keep the login entry pointing at this binary.
            crate::autostart::set(state.cfg.autostart);
        }
        if live.is_some() && state.cfg.show_tray {
            crate::tray::spawn(tx.clone());
        }
        if args.demo {
            // Fake traffic once per second.
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |m, cx| m.dispatch(Msg::Tick, cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        let mut me = Self {
            state,
            page: args
                .page
                .as_deref()
                .and_then(Page::from_id)
                .unwrap_or_default(),
            next_start_fails: false,
            live,
            tray: None,
            start_pending,
            tx,
        };
        if let Some(sc) = args.scenario.as_deref().and_then(Scenario::from_id) {
            me.apply_scenario(sc, cx);
        }
        if let Some(mac) = args.device.clone() {
            me.state.open_device = Some(mac);
        }
        me
    }

    pub fn is_demo(&self) -> bool {
        self.live.is_none()
    }

    /// A tray icon is showing, so closing the window keeps the app running.
    pub fn keeps_running_without_window(&self) -> bool {
        self.tray.is_some() && self.state.cfg.show_tray
    }

    /// Started hidden at login: a tray icon is (about to be) showing. If it
    /// fails to appear, `Msg::TrayGone` brings the window up.
    pub fn keeps_running_or_will(&self) -> bool {
        self.live.is_some() && self.state.cfg.show_tray
    }

    pub fn sender(&self) -> Tx {
        self.tx.clone()
    }

    fn toast(&self, text: impl Into<String>, cx: &mut Context<Self>) {
        cx.emit(ModelEvent::Toast(text.into()));
    }

    fn ask(&self, ask: Ask, cx: &mut Context<Self>) {
        cx.emit(ModelEvent::Ask(ask));
    }

    fn device_store(&self) -> crate::config_store::DeviceStore {
        crate::config_store::DeviceStore {
            new_devices: self.state.defaults,
            total_down_kbps: self.state.total_down_kbps,
            total_up_kbps: self.state.total_up_kbps,
            devices: self.state.policies.clone(),
            ..Default::default()
        }
    }

    fn policy_mut(&mut self, mac: &str) -> &mut DevicePolicy {
        let defaults = self.state.defaults;
        self.state
            .policies
            .entry(mac.to_owned())
            .or_insert_with(|| DevicePolicy::new_device(&defaults))
    }

    /// Runs `msg` after `delay` (demo mode timings).
    fn later(&self, delay: Duration, msg: Msg, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |m, cx| m.dispatch(msg, cx)).ok();
        })
        .detach();
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if let Err(errs) = self.state.cfg.validate() {
            let msg = crate::i18n::tr_dyn(&errs[0].to_string());
            let mut c = msg.chars();
            let msg = c
                .next()
                .map_or(String::new(), |f| f.to_uppercase().chain(c).collect());
            self.toast(msg, cx);
            return;
        }
        self.state.error = None;
        if let Some(live) = &mut self.live {
            crate::config_store::save(&self.state.cfg);
            live.start_hotspot(&self.state.cfg);
            return;
        }
        self.state.run = RunState::Starting;
        self.later(Duration::from_millis(1500), Msg::DemoStarted, cx);
    }

    fn apply_scenario(&mut self, sc: Scenario, cx: &mut Context<Self>) {
        let st = &mut self.state;
        match sc {
            Scenario::Normal => {
                let run = st.run;
                *st = demo::initial();
                st.run = run;
                self.next_start_fails = false;
            }
            Scenario::StartFails => {
                self.next_start_fails = true;
                self.toast(tr("The next start will fail"), cx);
            }
            Scenario::NoAdapter => st.problem = Some(Problem::NoAdapter),
            Scenario::NmMissing => st.problem = Some(Problem::NmMissing),
            Scenario::BridgeNoEthernet => {
                st.cfg.mode = Mode::Bridge;
                st.problem = Some(Problem::BridgeNoEthernet);
            }
            Scenario::AdapterGone => {
                st.run = RunState::Off;
                st.problem = Some(Problem::AdapterGone);
            }
            Scenario::HelperReady => st.helper = HelperStatus::Ready,
            Scenario::HelperMissing => st.helper = HelperStatus::Missing,
            Scenario::HelperUnauthorized => st.helper = HelperStatus::Unauthorized,
            Scenario::HelperTooOld => st.helper = HelperStatus::TooOld,
            Scenario::NoDevices => st.clients.clear(),
            Scenario::UsbAdapter => {
                use crate::state::{AdapterProblem, AdapterVm};
                let mut usb = AdapterVm::new("wlx00c0ca9a1b2c", true, true);
                usb.product = Some("802.11ac NIC".into());
                st.adapters = vec![AdapterVm::new("wlp2s0", true, false), usb];
                st.auto_adapter = Some("wlx00c0ca9a1b2c".into());
                st.cfg.ap_iface = None;
                st.adapter_problems = vec![AdapterProblem::UsbStorageMode {
                    name: "Realtek DISK".into(),
                    id: "0bda:1a2b".into(),
                }];
                self.toast(tr("New Wi-Fi adapter found"), cx);
            }
            Scenario::Windows => {
                // What Windows' Mobile Hotspot allows (see backend::windows).
                use crate::state::{AdapterVm, Features};
                st.features = Features {
                    bridge: false,
                    channel: false,
                    hidden: false,
                    band: true,
                    uplink_choice: true,
                };
                st.helper = HelperStatus::Unsupported;
                st.local_only_supported = false;
                st.cfg.mode = Mode::Router;
                st.cfg.hidden = false;
                st.cfg.channel = 0;
                let mut a = AdapterVm::new("5f0c3a1e-8d2b-4c11-9f6a-0b7e2d4c9a10", true, false);
                a.name = Some("Intel(R) Wi-Fi 6 AX201 160MHz".into());
                st.adapters = vec![a];
                st.auto_adapter = st.adapters.first().map(|a| a.iface.clone());
                st.cfg.ap_iface = None;
                st.adapter_problems.clear();
                st.nm_version.clear();
                for u in &mut st.uplinks {
                    u.connection = u.connection.take().or_else(|| Some("Cellular".into()));
                }
            }
            Scenario::NewDevice => {
                let c = demo::new_device();
                if st.client(&c.mac).is_none() {
                    let name = c.hostname.clone().unwrap_or_default();
                    st.policies.remove(&c.mac);
                    st.clients.push(c);
                    let msg = if st.defaults.require_approval {
                        trf("“{name}” wants to join — see Clients", &[("name", &name)])
                    } else {
                        trf(
                            "“{name}” joined with the default settings",
                            &[("name", &name)],
                        )
                    };
                    self.toast(msg, cx);
                }
            }
        }
    }

    /// Handles one message, then logs what changed, saves settings and
    /// updates the backend and the tray.
    pub fn dispatch(&mut self, msg: Msg, cx: &mut Context<Self>) {
        let before = self.state.cfg.clone();
        let store_before = self.device_store();
        let digest = crate::activity::Digest::of(&self.state);
        self.handle(msg, cx);
        crate::activity::record(&digest, &mut self.state);
        if self.live.is_some() {
            if self.state.cfg != before {
                crate::config_store::save(&self.state.cfg);
            }
            let store = self.device_store();
            if store != store_before {
                crate::config_store::save_devices(&store);
            }
        }
        if let Some(live) = &mut self.live {
            live.sync(&self.state);
        }
        if let Some(t) = &mut self.tray {
            t.update(crate::tray::TrayState::of(&self.state));
        }
        cx.notify();
    }

    fn handle(&mut self, msg: Msg, cx: &mut Context<Self>) {
        let st = &mut self.state;
        match msg {
            Msg::Toggle => match st.run {
                RunState::Off => self.start(cx),
                RunState::On if self.live.is_some() => {
                    if let Some(l) = &mut self.live {
                        l.stop_hotspot();
                    }
                }
                RunState::On => {
                    st.run = RunState::Stopping;
                    self.later(Duration::from_millis(600), Msg::DemoStopped, cx);
                }
                _ => {}
            },
            Msg::DemoStarted => {
                if self.next_start_fails {
                    self.next_start_fails = false;
                    st.run = RunState::Off;
                    let why = leh_model::reasons::device(9);
                    st.error = Some((
                        tr("Couldn't start the hotspot").into(),
                        crate::i18n::tr_dyn(&why.message),
                    ));
                } else if st.run == RunState::Starting {
                    st.run = RunState::On;
                }
            }
            Msg::DemoStopped => {
                st.run = RunState::Off;
                self.toast(tr("Hotspot stopped"), cx);
            }
            Msg::Ssid(s) => st.cfg.ssid = s,
            Msg::Password(p) => st.cfg.password = p,
            Msg::RegeneratePassword => {
                st.cfg.password = password::generate();
                self.toast(tr("New password generated"), cx);
            }
            Msg::CopyPassword => {
                cx.write_to_clipboard(ClipboardItem::new_string(st.cfg.password.clone()));
                self.toast(tr("Password copied"), cx);
            }
            Msg::Mode(Mode::Bridge) if !st.features.bridge => {
                self.toast(tr("Bridge mode isn't available on this system"), cx);
            }
            Msg::Mode(Mode::Bridge) if st.cfg.mode != Mode::Bridge => {
                self.ask(Ask::Bridge, cx);
            }
            Msg::Mode(Mode::LocalOnly) if !st.local_only_supported => {
                let text = if st.features.bridge {
                    trf(
                        "Local only needs NetworkManager 1.52 or newer (you have {version})",
                        &[("version", &st.nm_version)],
                    )
                } else {
                    tr("Local only isn't available on this system").into()
                };
                self.toast(text, cx);
            }
            Msg::Mode(m) => st.cfg.mode = m,
            Msg::BridgeConfirmed(true) => {
                st.cfg.mode = Mode::Bridge;
                // Only wired sources can be bridged.
                let wired = st
                    .cfg
                    .upstream_iface
                    .as_deref()
                    .and_then(|i| st.uplink(i))
                    .is_some_and(|u| u.can_bridge());
                if !wired {
                    st.cfg.upstream_iface = st
                        .uplinks
                        .iter()
                        .find(|u| u.can_bridge())
                        .map(|u| u.iface.clone());
                }
                if st.cfg.upstream_iface.is_none() {
                    st.problem = Some(Problem::BridgeNoEthernet);
                }
            }
            Msg::BridgeConfirmed(false) => {}
            Msg::Adapter(a) => st.cfg.ap_iface = a,
            Msg::Upstream(u) => {
                // Bridge always needs an explicit Ethernet device.
                if st.cfg.mode != Mode::Bridge || u.is_some() {
                    st.cfg.upstream_iface = u;
                }
            }
            Msg::Band(Band::Ghz5) if !st.ap_supports_5ghz() => {
                self.toast(tr("This adapter doesn't support 5 GHz"), cx);
            }
            Msg::Band(b) => {
                if st.cfg.band != b {
                    st.cfg.band = b;
                    st.cfg.channel = 0;
                }
            }
            Msg::Channel(c) => st.cfg.channel = c,
            Msg::Hidden(h) => st.cfg.hidden = h,
            Msg::TotalDown(v) => st.total_down_kbps = v,
            Msg::TotalUp(v) => st.total_up_kbps = v,
            Msg::Approval(a) => st.defaults.require_approval = a,
            Msg::Default(f) => f.apply(&mut st.defaults.limits),
            Msg::Autostart(a) => {
                st.cfg.autostart = a;
                if !a {
                    st.cfg.start_at_login = false;
                }
                if self.live.is_some() {
                    crate::autostart::set(a);
                }
            }
            Msg::StartAtLogin(a) => st.cfg.start_at_login = a,
            Msg::Tray(t) => {
                st.cfg.show_tray = t;
                if !t {
                    if let Some(tray) = self.tray.take() {
                        tray.close();
                    }
                } else if self.tray.is_none() && self.live.is_some() {
                    crate::tray::spawn(self.tx.clone());
                }
            }
            Msg::ExportDiagnostics if self.live.is_some() => {
                self.toast(tr("Diagnostics export is coming in a later version"), cx);
            }
            Msg::ExportDiagnostics => {
                self.toast(tr("Demo: diagnostics would be saved to a .txt file"), cx)
            }
            Msg::Allow(mac) => {
                self.policy_mut(&mac).access = Access::Allowed;
                self.toast(tr("Device allowed"), cx);
            }
            Msg::Block(mac) => {
                self.policy_mut(&mac).access = Access::Blocked;
                self.toast(tr("Device blocked"), cx);
            }
            Msg::OpenDevice(mac) => st.open_device = Some(mac),
            Msg::DeviceClosed => st.open_device = None,
            Msg::DeviceAlias(mac, name) => {
                let name = name.trim().to_owned();
                self.policy_mut(&mac).alias = (!name.is_empty()).then_some(name);
            }
            Msg::DeviceAccess(mac, a) => self.policy_mut(&mac).access = a,
            Msg::DeviceFollow(mac, f) => {
                let defaults = self.state.defaults.limits;
                let p = self.policy_mut(&mac);
                if p.follow_defaults && !f {
                    // Start the override from the current defaults.
                    p.limits = defaults;
                }
                p.follow_defaults = f;
            }
            Msg::DeviceLimit(mac, f) => f.apply(&mut self.policy_mut(&mac).limits),
            Msg::ForgetDevice(mac) => {
                st.policies.remove(&mac);
                st.clients.retain(|c| c.mac != mac);
                if st.open_device.as_deref() == Some(mac.as_str()) {
                    st.open_device = None;
                }
                self.toast(tr("Device forgotten"), cx);
            }
            Msg::HelperAction if self.live.is_some() && st.helper == HelperStatus::Unauthorized => {
                // Try again; polkit shows its dialog if needed.
                st.helper = HelperStatus::Ready;
                if let Some(live) = &mut self.live {
                    live.helper_authorize();
                }
            }
            Msg::HelperAction => match st.helper {
                HelperStatus::Unauthorized => {
                    st.helper = HelperStatus::Ready;
                    self.toast(tr("Demo: the system would ask for your password here"), cx);
                }
                HelperStatus::Unsupported | HelperStatus::Ready => {}
                _ => self.ask(Ask::Helper, cx),
            },
            Msg::HelperCheck if self.live.is_some() => {
                if let Some(live) = &self.live {
                    live.helper_check();
                }
            }
            Msg::HelperCheck => {
                st.helper = HelperStatus::Ready;
                self.toast(tr("Helper found — per-device control enabled"), cx);
            }
            Msg::ErrorDetails => {
                if let Some((title, details)) = st.error.take() {
                    self.ask(Ask::ErrorDetails { title, details }, cx);
                    if let Some(live) = &mut self.live {
                        live.dismiss_error();
                    }
                }
            }
            Msg::ProblemAction => self.problem_action(cx),
            Msg::Scenario(sc) => self.apply_scenario(sc, cx),
            Msg::Tick => {
                if st.run == RunState::On {
                    demo::tick(st);
                }
            }
            Msg::Backend(ev) => self.backend_event(ev, cx),
            Msg::ShowWindow => cx.emit(ModelEvent::ShowWindow),
            Msg::Quit => cx.quit(),
            Msg::TrayStarted(h) => {
                st.tray_available = true;
                if st.cfg.show_tray {
                    self.tray = Some(h);
                } else {
                    h.close();
                }
            }
            Msg::TrayReady => st.tray_available = true,
            Msg::TrayGone => {
                st.tray_available = false;
                if let Some(t) = self.tray.take() {
                    t.close();
                }
                // Without a tray there'd be no way back to a closed window.
                if cx.windows().is_empty() && !self.start_pending && st.run == RunState::Off {
                    cx.emit(ModelEvent::ShowWindow);
                }
            }
            Msg::Language(l) => {
                if l != st.cfg.language {
                    st.cfg.language = l;
                    self.ask(Ask::Restart, cx);
                }
            }
            Msg::Restart => {
                crate::RESTART.store(true, std::sync::atomic::Ordering::Relaxed);
                cx.quit();
            }
            Msg::InstallHelper => {
                if let Some(live) = &self.live {
                    live.install_helper();
                }
            }
            Msg::InstallProgress(text) => self.toast(text, cx),
            Msg::InstallDone(Ok(())) => {
                self.toast(tr("Helper installed"), cx);
                if let Some(live) = &self.live {
                    live.watch_helper();
                }
            }
            Msg::InstallDone(Err(e)) if e == "cancelled" => {
                self.toast(tr("Installation cancelled"), cx)
            }
            Msg::InstallDone(Err(e)) => self.ask(Ask::InstallFailed(e), cx),
            Msg::CopyText(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.toast(tr("Copied to the clipboard"), cx);
            }
            Msg::Helper(h) => {
                let was = st.helper;
                st.helper = h;
                if h == HelperStatus::Ready && was != HelperStatus::Ready && self.page != Page::Home
                {
                    self.toast(tr("Helper found: per-device control is on"), cx);
                }
            }
            Msg::Page(p) => {
                // Fresh connection states when looking at Settings.
                if p == Page::Settings
                    && let Some(live) = &self.live
                {
                    live.refresh();
                }
                self.page = p;
                st.open_device = None;
            }
            Msg::About => self.ask(Ask::About, cx),
        }
    }

    fn problem_action(&mut self, cx: &mut Context<Self>) {
        let st = &mut self.state;
        let Some(live) = self.live.as_mut() else {
            match st.problem.take() {
                Some(Problem::NmMissing) => {
                    st.problem = Some(Problem::NmMissing);
                    cx.open_url("https://networkmanager.dev/");
                }
                Some(Problem::BridgeNoEthernet) => st.cfg.mode = Mode::Router,
                Some(Problem::AdapterGone) => self.start(cx),
                _ => {}
            }
            return;
        };
        match st.problem {
            Some(Problem::NmMissing) => {
                cx.open_url("https://networkmanager.dev/");
                live.refresh();
            }
            Some(Problem::BridgeNoEthernet) => {
                st.cfg.mode = Mode::Router;
                st.problem = None;
                live.dismiss_error();
            }
            Some(Problem::AdapterGone) => {
                st.problem = None;
                live.refresh();
                live.start_hotspot(&st.cfg);
            }
            _ => live.refresh(),
        }
    }

    fn backend_event(&mut self, ev: crate::backend::Event, cx: &mut Context<Self>) {
        let Some(live) = self.live.as_mut() else {
            return;
        };
        let st = &mut self.state;
        let first = !st.backend_ready;
        let usb_before: Vec<String> = st
            .adapters
            .iter()
            .filter(|a| a.usb)
            .map(|a| a.iface.clone())
            .collect();
        let problems_before = st.adapter_problems.clone();
        let toasts = live.on_event(st, ev);
        for t in toasts {
            cx.emit(ModelEvent::Toast(t));
        }
        // Hot-plugged adapters: say what happened.
        if !first {
            let new_usb = st
                .adapters
                .iter()
                .find(|a| a.usb && !usb_before.contains(&a.iface));
            let new_problem = st
                .adapter_problems
                .iter()
                .any(|p| !problems_before.contains(p));
            if let Some(a) = new_usb {
                let name = a.label();
                // Only "Automatic" switches to it by itself.
                let msg = if st.cfg.ap_iface.is_some() {
                    trf(
                        "USB Wi-Fi adapter found ({iface}). Choose it in Settings → Hotspot adapter",
                        &[("iface", &name)],
                    )
                } else if st.run == RunState::On {
                    trf(
                        "USB Wi-Fi adapter found ({iface}). It will be used the next time the hotspot starts",
                        &[("iface", &name)],
                    )
                } else {
                    trf("USB Wi-Fi adapter found ({iface})", &[("iface", &name)])
                };
                cx.emit(ModelEvent::Toast(msg));
            } else if new_problem {
                cx.emit(ModelEvent::Toast(
                    tr("A Wi-Fi adapter can't be used — see Settings").into(),
                ));
            }
        }
        if self.start_pending && st.backend_ready && st.problem.is_none() && !st.adapters.is_empty()
        {
            self.start_pending = false;
            if st.run == RunState::Off {
                self.start(cx);
            }
        }
    }
}

/// Sends `msg` to the model from a click handler.
pub fn send(model: &gpui_kit::Entity<Model>, cx: &mut App, msg: Msg) {
    model.update(cx, |m, cx| m.dispatch(msg, cx));
}
