//! Tray icon: a StatusNotifierItem on Linux, a notification-area icon on
//! Windows. Clicking it shows the window; its menu starts/stops the hotspot
//! and approves waiting devices.

use leh_model::policy::Access;

use crate::i18n::{tr, tr_n};
use crate::state::{RunState, UiState};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{TrayHandle, spawn};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{TrayHandle, spawn};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayState {
    pub running: bool,
    pub busy: bool,
    pub devices: usize,
    /// (MAC, name) of devices waiting for approval.
    pub pending: Vec<(String, String)>,
}

impl TrayState {
    pub fn of(s: &UiState) -> Self {
        let on = s.run == RunState::On;
        Self {
            running: on,
            busy: matches!(s.run, RunState::Starting | RunState::Stopping),
            devices: if on { s.connected_count() } else { 0 },
            pending: if on && s.helper_ready() {
                s.clients
                    .iter()
                    .filter(|c| s.effective(c).access == Access::Pending)
                    .map(|c| (c.mac.clone(), s.display_name(c)))
                    .collect()
            } else {
                Vec::new()
            },
        }
    }

    /// "Running · 3 devices · 1 waiting".
    pub fn status_line(&self) -> String {
        if self.busy {
            tr("Working…").into()
        } else if self.running {
            let mut t = tr_n(
                "Running · {n} device",
                "Running · {n} devices",
                self.devices,
            );
            if !self.pending.is_empty() {
                t.push_str(&tr_n(
                    " · {n} waiting",
                    " · {n} waiting",
                    self.pending.len(),
                ));
            }
            t
        } else {
            tr("Hotspot is off").into()
        }
    }
}

/// One entry of the tray menu, shared by both platforms.
pub enum Item {
    /// Greyed-out text.
    Label(String),
    Action {
        label: String,
        enabled: bool,
        msg: fn(&str) -> crate::msg::Msg,
        /// Argument passed to `msg` (a MAC address, or empty).
        arg: String,
    },
    Separator,
}

/// The tray menu for `s`.
pub fn menu(s: &TrayState) -> Vec<Item> {
    use crate::msg::Msg;
    let mut items = vec![Item::Label(s.status_line())];
    for (mac, name) in &s.pending {
        items.push(Item::Action {
            label: tr("Allow “{name}”").replace("{name}", name),
            enabled: true,
            msg: |m| Msg::Allow(m.to_owned()),
            arg: mac.clone(),
        });
        items.push(Item::Action {
            label: tr("Block “{name}”").replace("{name}", name),
            enabled: true,
            msg: |m| Msg::Block(m.to_owned()),
            arg: mac.clone(),
        });
    }
    items.push(Item::Separator);
    items.push(Item::Action {
        label: if s.running {
            tr("Stop hotspot").into()
        } else {
            tr("Start hotspot").into()
        },
        enabled: !s.busy,
        msg: |_| Msg::Toggle,
        arg: String::new(),
    });
    items.push(Item::Action {
        label: tr("Show window").into(),
        enabled: true,
        msg: |_| Msg::ShowWindow,
        arg: String::new(),
    });
    items.push(Item::Separator);
    items.push(Item::Action {
        label: tr("Quit").into(),
        enabled: true,
        msg: |_| Msg::Quit,
        arg: String::new(),
    });
    items
}
