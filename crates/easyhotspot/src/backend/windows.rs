//! Windows backend: the built-in Mobile Hotspot through `leh-win`.
//!
//! Windows doesn't let apps pick a channel, hide the network, bridge, or
//! control devices one by one, so those parts of the UI are turned off
//! (see [`Features`]).

use leh_model::{ErrorKind, HotspotConfig, HotspotState};
use leh_win::{Controller, Snapshot, UplinkKind as WinUplinkKind};

use super::common::{self, HelperInstall, TrafficMeter};
use crate::i18n::{tr, trf};
use crate::msg::{Msg, Tx};
use crate::state::{
    AdapterVm, Features, HelperStatus, Problem, RunState, UiState, UplinkKind, UplinkVm,
};

#[derive(Debug)]
pub enum Event {
    Snapshot(Box<Snapshot>),
}

pub struct Live {
    controller: Controller,
    last: Snapshot,
    /// Error the user already dismissed; not shown again until it changes.
    dismissed: Option<HotspotState>,
    traffic: TrafficMeter,
}

/// What Windows' hotspot can do; the band only on Windows 11.
fn features(band: bool) -> Features {
    Features {
        bridge: false,
        channel: false,
        hidden: false,
        band,
        uplink_choice: true,
    }
}

impl Live {
    pub fn start(tx: Tx) -> Self {
        let controller =
            Controller::spawn(move |snap| tx.emit(Msg::Backend(Event::Snapshot(Box::new(snap)))));
        Self {
            controller,
            last: Snapshot::default(),
            dismissed: None,
            traffic: TrafficMeter::default(),
        }
    }

    /// View-model before the first snapshot arrives.
    pub fn initial_state(cfg: HotspotConfig) -> UiState {
        let mut st = crate::state::demo::initial();
        st.cfg = cfg;
        st.live = true;
        st.backend_ready = false;
        st.features = features(false);
        st.local_only_supported = false;
        st.nm_version.clear();
        st.adapters.clear();
        st.auto_adapter = None;
        st.adapter_problems.clear();
        st.wifi_moved_to = None;
        st.traffic = Default::default();
        st.ethernet.clear();
        st.uplinks.clear();
        st.clients.clear();
        st.helper = HelperStatus::Unsupported;
        st.tray_available = true;
        st.running_iface = None;
        let store = crate::config_store::load_devices();
        st.defaults = store.new_devices;
        st.total_down_kbps = store.total_down_kbps;
        st.total_up_kbps = store.total_up_kbps;
        st.policies = store.devices;
        // Settings Windows can't apply.
        st.cfg.mode = leh_model::Mode::Router;
        st.cfg.hidden = false;
        st.cfg.channel = 0;
        st
    }

    pub fn start_hotspot(&mut self, cfg: &HotspotConfig) {
        self.dismissed = None;
        self.controller.start(cfg.clone());
    }

    pub fn stop_hotspot(&mut self) {
        self.controller.stop();
    }

    pub fn refresh(&self) {
        self.controller.refresh();
    }

    pub fn dismiss_error(&mut self) {
        self.dismissed = Some(self.last.state.clone());
    }

    pub fn on_event(&mut self, st: &mut UiState, ev: Event) -> Vec<String> {
        let Event::Snapshot(snap) = ev;
        self.apply(st, *snap)
    }

    fn apply(&mut self, st: &mut UiState, snap: Snapshot) -> Vec<String> {
        st.backend_ready = true;
        st.features = features(snap.band_selectable);
        st.adapters = snap
            .adapters
            .iter()
            .map(|a| AdapterVm {
                name: Some(a.name.clone()),
                ..AdapterVm::new(&a.id, a.five_ghz.unwrap_or(false), a.usb)
            })
            .collect();
        // Windows picks the adapter itself; prefer showing a USB one.
        st.auto_adapter = snap
            .adapters
            .iter()
            .find(|a| a.usb)
            .or(snap.adapters.first())
            .map(|a| a.id.clone());
        let mut uplinks: Vec<UplinkVm> = snap
            .uplinks
            .iter()
            .map(|u| UplinkVm {
                iface: u.id.clone(),
                kind: match u.kind {
                    WinUplinkKind::Ethernet | WinUplinkKind::Other => UplinkKind::Ethernet,
                    WinUplinkKind::Wifi => UplinkKind::Wifi,
                    WinUplinkKind::Cellular => UplinkKind::Modem,
                },
                connection: Some(u.name.clone()),
                connected: true,
                available: true,
                is_default: u.is_default,
            })
            .collect();
        uplinks.sort_by_key(|u| (!u.is_default, u.kind as u8, u.connection.clone()));
        st.uplinks = uplinks;

        let dismissed = self.dismissed.as_ref() == Some(&snap.state);
        st.problem = None;
        st.problem_detail = None;
        if let Some(why) = &snap.unavailable {
            st.problem = Some(Problem::Unavailable);
            st.problem_detail = Some(crate::i18n::tr_dyn(why));
        }
        st.run = match &snap.state {
            HotspotState::Idle => RunState::Off,
            HotspotState::Activating => RunState::Starting,
            HotspotState::Active => RunState::On,
            HotspotState::Deactivating => RunState::Stopping,
            HotspotState::Error(kind) => {
                if !dismissed {
                    match kind {
                        ErrorKind::NoApAdapter => st.problem = Some(Problem::NoAdapter),
                        ErrorKind::DeviceUnavailable => st.problem = Some(Problem::AdapterGone),
                        ErrorKind::Activation(e) => {
                            st.error = Some((
                                tr("Couldn't start the hotspot").into(),
                                crate::i18n::tr_dyn(&e.message),
                            ));
                        }
                        ErrorKind::Call(msg) => {
                            st.error = Some((tr("Windows reported an error").into(), msg.clone()));
                        }
                        // NetworkManager-only problems.
                        ErrorKind::NmUnavailable
                        | ErrorKind::NmTooOld(_)
                        | ErrorKind::BridgeNeedsEthernet => {}
                    }
                }
                RunState::Off
            }
        };
        if st.run != RunState::Off {
            st.error = None;
        }
        st.running_iface = snap.running.as_ref().and_then(|r| r.adapter_id.clone());
        if let Some(r) = &snap.running
            && st.run == RunState::On
            && r.ssid != st.cfg.ssid
            && !r.ssid.is_empty()
        {
            // Started from Windows' settings with another name: show what runs.
            st.cfg.ssid = r.ssid.clone();
        }

        let mut toasts = Vec::new();
        if st.run == RunState::On {
            toasts.extend(
                common::merge_clients(st, &snap.clients)
                    .into_iter()
                    .map(|name| trf("“{name}” wants to join — see Clients", &[("name", &name)])),
            );
            match snap.traffic {
                Some((rx, tx)) => self.traffic.fold(st, rx, tx),
                None => self.traffic.reset(),
            }
        } else if !st.clients.is_empty() || self.last.state == HotspotState::Active {
            self.traffic.reset();
            common::forget_session(st);
        }
        self.last = snap;
        toasts
    }

    /// No per-device control on Windows yet.
    pub fn sync(&mut self, _st: &UiState) {}

    pub fn helper_check(&self) {}

    pub fn helper_authorize(&mut self) {}

    pub fn watch_helper(&self) {}

    pub fn install_helper(&self) {}
}

pub fn helper_install() -> HelperInstall {
    HelperInstall {
        can_install: false,
        command: String::new(),
        no_new_privs: false,
    }
}
