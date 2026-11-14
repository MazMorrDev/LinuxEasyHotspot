//! Everything that can happen: user actions, backend events, tray clicks.

use leh_model::policy::{Access, CapAction, Limits, Priority};
use leh_model::{Band, Language, Mode};

use crate::state::HelperStatus;
use crate::state::demo::Scenario;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Page {
    #[default]
    Home,
    Settings,
    Clients,
}

impl Page {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "home" => Some(Self::Home),
            "settings" => Some(Self::Settings),
            "clients" => Some(Self::Clients),
            _ => None,
        }
    }
}

/// One editable part of a device's (or the defaults') limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitField {
    Down(Option<u32>),
    Up(Option<u32>),
    Priority(Priority),
    Cap(Option<u32>),
    CapAction(CapAction),
}

impl LimitField {
    pub fn apply(self, l: &mut Limits) {
        match self {
            Self::Down(v) => l.down_kbps = v,
            Self::Up(v) => l.up_kbps = v,
            Self::Priority(p) => l.priority = p,
            Self::Cap(v) => l.cap_mb = v,
            Self::CapAction(a) => l.cap_action = a,
        }
    }
}

// Some messages only come from the Linux backend (helper, tray watcher).
#[cfg_attr(windows, allow(dead_code))]
#[derive(Debug)]
pub enum Msg {
    Toggle,
    DemoStarted,
    DemoStopped,
    Ssid(String),
    Password(String),
    RegeneratePassword,
    CopyPassword,
    Mode(Mode),
    BridgeConfirmed(bool),
    Adapter(Option<String>),
    Upstream(Option<String>),
    Band(Band),
    Channel(u32),
    Hidden(bool),
    TotalDown(Option<u32>),
    TotalUp(Option<u32>),
    Approval(bool),
    Default(LimitField),
    Autostart(bool),
    StartAtLogin(bool),
    Tray(bool),
    ExportDiagnostics,
    Allow(String),
    Block(String),
    OpenDevice(String),
    DeviceClosed,
    DeviceAlias(String, String),
    DeviceAccess(String, Access),
    DeviceFollow(String, bool),
    DeviceLimit(String, LimitField),
    ForgetDevice(String),
    HelperAction,
    HelperCheck,
    ErrorDetails,
    ProblemAction,
    Scenario(Scenario),
    Tick,
    Page(Page),
    About,
    /// From the platform backend (NetworkManager or Windows).
    Backend(crate::backend::Event),
    /// Helper probe result.
    Helper(HelperStatus),
    /// Install the helper (Linux, from a source checkout).
    InstallHelper,
    InstallProgress(String),
    InstallDone(Result<(), String>),
    CopyText(String),
    ShowWindow,
    Quit,
    TrayStarted(crate::tray::TrayHandle),
    /// The desktop has no tray (or it went away).
    TrayGone,
    TrayReady,
    Language(Language),
    Restart,
}

/// Sends messages to the model from other threads (backend, tray).
#[derive(Clone)]
pub struct Tx(pub async_channel::Sender<Msg>);

impl Tx {
    pub fn emit(&self, msg: Msg) {
        // Unbounded: only fails once the app is shutting down.
        let _ = self.0.try_send(msg);
    }
}
