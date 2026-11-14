//! Session activity log shown on Home: what happened since the hotspot
//! started (devices joining and leaving, approvals, caps, adapters, errors).
//!
//! Instead of sprinkling log calls over every handler, the app takes a small
//! [`Digest`] of the view-model before each update and [`record`]s the
//! differences afterwards, so live and demo mode log the same way.

use std::collections::BTreeMap;

use leh_model::policy::Access;
use relm4::gtk::glib;

use crate::i18n::{tr, trf};
use crate::state::{HelperStatus, RunState, UiState, fmt_bytes};

/// Older entries are dropped beyond this.
const MAX_EVENTS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Started,
    Stopped,
    Joined,
    Left,
    Waiting,
    Allowed,
    Blocked,
    Warning,
    Error,
    Info,
}

impl EventKind {
    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Started => "media-playback-start-symbolic",
            Self::Stopped => "media-playback-stop-symbolic",
            Self::Joined => "list-add-symbolic",
            Self::Left => "list-remove-symbolic",
            Self::Waiting => "alarm-symbolic",
            Self::Allowed => "object-select-symbolic",
            Self::Blocked => "action-unavailable-symbolic",
            Self::Warning => "dialog-warning-symbolic",
            Self::Error => "dialog-error-symbolic",
            Self::Info => "dialog-information-symbolic",
        }
    }

    /// Style class for the icon, if any.
    pub fn css_class(self) -> Option<&'static str> {
        match self {
            Self::Started | Self::Allowed => Some("success"),
            Self::Warning | Self::Waiting => Some("warning"),
            Self::Error | Self::Blocked => Some("error"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEvent {
    /// Local time, "14:05:32".
    pub time: String,
    pub kind: EventKind,
    pub text: String,
}

fn clock() -> String {
    glib::DateTime::now_local()
        .and_then(|t| t.format("%H:%M:%S"))
        .map(|s| s.to_string())
        .unwrap_or_default()
}

impl UiState {
    pub fn log(&mut self, kind: EventKind, text: impl Into<String>) {
        self.events.push(SessionEvent {
            time: clock(),
            kind,
            text: text.into(),
        });
        if self.events.len() > MAX_EVENTS {
            let extra = self.events.len() - MAX_EVENTS;
            self.events.drain(..extra);
        }
        self.events_seq += 1;
    }
}

/// What the log compares between two moments.
pub struct Digest {
    run: RunState,
    /// Connected devices: MAC → (name, effective access, cap reached).
    online: BTreeMap<String, (String, Access, bool)>,
    /// Saved access of every known device.
    saved: BTreeMap<String, (String, Access)>,
    adapters: Vec<(String, String)>,
    problems: Vec<String>,
    wifi_moved_to: Option<String>,
    error: Option<(String, String)>,
    helper: HelperStatus,
    used_bytes: u64,
}

impl Digest {
    pub fn of(s: &UiState) -> Self {
        Self {
            run: s.run,
            online: s
                .online_clients()
                .iter()
                .map(|c| {
                    let e = s.effective(c);
                    (c.mac.clone(), (s.display_name(c), e.access, e.cap_reached))
                })
                .collect(),
            saved: s
                .policies
                .iter()
                .map(|(mac, p)| (mac.clone(), (s.known_name(mac), p.access)))
                .collect(),
            adapters: s
                .adapters
                .iter()
                .map(|a| (a.iface.clone(), a.label()))
                .collect(),
            problems: s.adapter_problems.iter().map(|p| p.explain()).collect(),
            wifi_moved_to: s.wifi_moved_to.as_ref().map(|a| a.label()),
            error: s.error.clone(),
            helper: s.helper,
            used_bytes: s.traffic.down_bytes + s.traffic.up_bytes,
        }
    }
}

fn quoted(name: &str) -> String {
    format!("“{name}”")
}

/// Adds an entry for everything that changed since `before`.
pub fn record(before: &Digest, s: &mut UiState) {
    let now = Digest::of(s);
    let helper = s.helper_ready();

    // The hotspot itself. A new session starts with a clean log.
    if before.run == RunState::Off && now.run == RunState::Starting {
        s.events.clear();
        s.events_seq += 1;
    }
    if before.run != RunState::On && now.run == RunState::On {
        let mut parts = vec![quoted(&s.cfg.ssid)];
        parts.push(
            match s.cfg.mode {
                leh_model::Mode::Router => tr("Router"),
                leh_model::Mode::Bridge => tr("Bridge"),
                leh_model::Mode::LocalOnly => tr("Local only"),
            }
            .to_owned(),
        );
        if let Some(a) = s.current_adapter() {
            parts.push(a.label());
        }
        s.log(
            EventKind::Started,
            trf("Hotspot on: {details}", &[("details", &parts.join(" · "))]),
        );
    }
    if matches!(before.run, RunState::On | RunState::Stopping) && now.run == RunState::Off {
        let text = if before.used_bytes > 0 {
            trf(
                "Hotspot off · {used} used in this session",
                &[("used", &fmt_bytes(before.used_bytes))],
            )
        } else {
            tr("Hotspot off").to_owned()
        };
        s.log(EventKind::Stopped, text);
    }

    // Devices joining and leaving (not the burst when the hotspot stops).
    if before.run == RunState::On && now.run == RunState::On {
        for (mac, (name, access, _)) in &now.online {
            if before.online.contains_key(mac) {
                continue;
            }
            if *access == Access::Pending && helper {
                s.log(
                    EventKind::Waiting,
                    trf("{name} wants to join", &[("name", &quoted(name))]),
                );
            } else if *access == Access::Blocked && helper {
                s.log(
                    EventKind::Blocked,
                    trf(
                        "{name} tried to connect (blocked)",
                        &[("name", &quoted(name))],
                    ),
                );
            } else {
                s.log(
                    EventKind::Joined,
                    trf("{name} connected", &[("name", &quoted(name))]),
                );
            }
        }
        for (mac, (name, _, _)) in &before.online {
            if !now.online.contains_key(mac) && now.saved.contains_key(mac) {
                s.log(
                    EventKind::Left,
                    trf("{name} disconnected", &[("name", &quoted(name))]),
                );
            }
        }
        for (mac, (name, _, cap)) in &now.online {
            if *cap && before.online.get(mac).is_some_and(|(_, _, was)| !was) {
                s.log(
                    EventKind::Warning,
                    trf("{name} reached its data cap", &[("name", &quoted(name))]),
                );
            }
        }
    }

    // Decisions about known devices.
    for (mac, (name, access)) in &now.saved {
        let Some((_, was)) = before.saved.get(mac) else {
            continue;
        };
        if was == access {
            continue;
        }
        let name = quoted(name);
        match access {
            Access::Allowed => s.log(
                EventKind::Allowed,
                trf("{name} allowed", &[("name", &name)]),
            ),
            Access::Blocked => s.log(
                EventKind::Blocked,
                trf("{name} blocked", &[("name", &name)]),
            ),
            Access::Pending => s.log(
                EventKind::Waiting,
                trf("{name} must be approved again", &[("name", &name)]),
            ),
        }
    }
    for (mac, (name, _)) in &before.saved {
        if !now.saved.contains_key(mac) {
            s.log(
                EventKind::Info,
                trf("{name} forgotten", &[("name", &quoted(name))]),
            );
        }
    }

    // Adapters plugged in or out (the first list after startup isn't news).
    if !before.adapters.is_empty() || !before.problems.is_empty() {
        for (iface, label) in &now.adapters {
            if !before.adapters.iter().any(|(i, _)| i == iface) {
                s.log(
                    EventKind::Info,
                    trf("Wi-Fi adapter connected: {adapter}", &[("adapter", label)]),
                );
            }
        }
        for (iface, label) in &before.adapters {
            if !now.adapters.iter().any(|(i, _)| i == iface) {
                s.log(
                    EventKind::Warning,
                    trf("Wi-Fi adapter removed: {adapter}", &[("adapter", label)]),
                );
            }
        }
    }
    for p in &now.problems {
        if !before.problems.contains(p) {
            s.log(EventKind::Warning, p.clone());
        }
    }

    if let Some(adapter) = &now.wifi_moved_to
        && before.wifi_moved_to.as_ref() != Some(adapter)
    {
        s.log(
            EventKind::Info,
            trf(
                "This computer's Wi-Fi connection moved to {adapter}, so it stays online",
                &[("adapter", adapter)],
            ),
        );
    }

    if let Some((title, details)) = &now.error
        && before.error.as_ref() != Some(&(title.clone(), details.clone()))
    {
        s.log(EventKind::Error, format!("{title}: {details}"));
    }

    match (before.helper, now.helper) {
        (a, HelperStatus::Unauthorized) if a != HelperStatus::Unauthorized => s.log(
            EventKind::Warning,
            tr("Speed limits paused: Linux Easy Hotspot needs your authorization"),
        ),
        (HelperStatus::Unauthorized, HelperStatus::Ready) => {
            s.log(EventKind::Info, tr("Speed limits authorized"))
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::demo;

    fn texts(s: &UiState) -> Vec<String> {
        s.events.iter().map(|e| e.text.clone()).collect()
    }

    #[test]
    fn records_a_session() {
        let mut s = demo::initial();
        s.run = RunState::Off;

        // Start: the log is cleared, then "on" is recorded.
        s.events.push(SessionEvent {
            time: String::new(),
            kind: EventKind::Info,
            text: "old session".into(),
        });
        let d = Digest::of(&s);
        s.run = RunState::Starting;
        record(&d, &mut s);
        assert!(s.events.is_empty());
        let d = Digest::of(&s);
        s.run = RunState::On;
        record(&d, &mut s);
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].kind, EventKind::Started);

        // A new device asks to join (approval is on in the demo defaults).
        let d = Digest::of(&s);
        let c = demo::new_device();
        s.policies.insert(
            c.mac.clone(),
            leh_model::policy::DevicePolicy::new_device(&s.defaults),
        );
        s.clients.push(c.clone());
        record(&d, &mut s);
        assert!(
            texts(&s)
                .iter()
                .any(|t| t.contains("iPad-de-Ana") && t.contains("wants to join"))
        );

        // It is allowed, then leaves.
        let d = Digest::of(&s);
        s.policies.get_mut(&c.mac).unwrap().access = Access::Allowed;
        record(&d, &mut s);
        assert_eq!(s.events.last().unwrap().kind, EventKind::Allowed);
        let d = Digest::of(&s);
        s.clients.retain(|x| x.mac != c.mac);
        record(&d, &mut s);
        assert_eq!(s.events.last().unwrap().kind, EventKind::Left);

        // Stop: one entry, devices aren't listed as leaving one by one.
        let n = s.events.len();
        let d = Digest::of(&s);
        s.run = RunState::Off;
        record(&d, &mut s);
        assert_eq!(s.events.len(), n + 1);
        assert_eq!(s.events.last().unwrap().kind, EventKind::Stopped);
    }

    #[test]
    fn log_is_capped() {
        let mut s = demo::initial();
        for i in 0..(MAX_EVENTS + 10) {
            s.log(EventKind::Info, i.to_string());
        }
        assert_eq!(s.events.len(), MAX_EVENTS);
        assert_eq!(s.events.last().unwrap().text, (MAX_EVENTS + 9).to_string());
    }
}
