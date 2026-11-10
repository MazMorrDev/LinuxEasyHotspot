//! System tray icon (StatusNotifierItem), shown when the desktop has a tray.

use ksni::TrayMethods as _;
use ksni::menu::{MenuItem, StandardItem};

use crate::app::Msg;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayState {
    pub running: bool,
    pub busy: bool,
    pub devices: usize,
    /// (MAC, name) of devices waiting for approval.
    pub pending: Vec<(String, String)>,
}

struct Tray {
    state: TrayState,
    tx: relm4::Sender<Msg>,
}

impl Tray {
    fn send(&self, m: Msg) {
        self.tx.emit(m);
    }
}

fn status_line(s: &TrayState) -> String {
    if s.busy {
        crate::i18n::tr("Working…").into()
    } else if s.running {
        let mut t = crate::i18n::tr_n("Running · {n} device", "Running · {n} devices", s.devices);
        if !s.pending.is_empty() {
            t.push_str(&crate::i18n::tr_n(
                " · {n} waiting",
                " · {n} waiting",
                s.pending.len(),
            ));
        }
        t
    } else {
        crate::i18n::tr("Hotspot is off").into()
    }
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "linuxeasyhotspot".into()
    }

    fn title(&self) -> String {
        "LinuxEasyHotspot".into()
    }

    fn icon_name(&self) -> String {
        crate::APP_ID.into()
    }

    fn icon_theme_path(&self) -> String {
        // Source builds: the icon isn't installed in the system theme.
        let dev = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons");
        if !crate::live::packaged() && std::path::Path::new(dev).exists() {
            dev.into()
        } else {
            String::new()
        }
    }

    fn status(&self) -> ksni::Status {
        if self.state.pending.is_empty() {
            ksni::Status::Active
        } else {
            ksni::Status::NeedsAttention
        }
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "LinuxEasyHotspot".into(),
            description: status_line(&self.state),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(Msg::ShowWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        use crate::i18n::tr;
        let s = &self.state;
        let mut items: Vec<MenuItem<Self>> = vec![
            StandardItem {
                label: status_line(s),
                enabled: false,
                ..Default::default()
            }
            .into(),
        ];
        for (mac, name) in &s.pending {
            let (m1, m2) = (mac.clone(), mac.clone());
            items.push(
                StandardItem {
                    label: tr("Allow “{name}”").replace("{name}", name),
                    activate: Box::new(move |t: &mut Self| t.send(Msg::Allow(m1.clone()))),
                    ..Default::default()
                }
                .into(),
            );
            items.push(
                StandardItem {
                    label: tr("Block “{name}”").replace("{name}", name),
                    activate: Box::new(move |t: &mut Self| t.send(Msg::Block(m2.clone()))),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: if s.running {
                    tr("Stop hotspot").into()
                } else {
                    tr("Start hotspot").into()
                },
                enabled: !s.busy,
                activate: Box::new(|t: &mut Self| t.send(Msg::Toggle)),
                ..Default::default()
            }
            .into(),
        );
        items.push(
            StandardItem {
                label: tr("Show window").into(),
                activate: Box::new(|t: &mut Self| t.send(Msg::ShowWindow)),
                ..Default::default()
            }
            .into(),
        );
        items.push(MenuItem::Separator);
        items.push(
            StandardItem {
                label: tr("Quit").into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|t: &mut Self| t.send(Msg::Quit)),
                ..Default::default()
            }
            .into(),
        );
        items
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.send(Msg::TrayGone);
        // Keep the service so the icon comes back if the panel restarts.
        true
    }

    fn watcher_online(&self) {
        self.send(Msg::TrayReady);
    }
}

impl std::fmt::Debug for TrayHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayHandle")
            .field("last", &self.last)
            .finish()
    }
}

/// A running tray icon. Dropping it removes the icon.
pub struct TrayHandle {
    handle: ksni::Handle<Tray>,
    last: TrayState,
}

impl TrayHandle {
    pub fn update(&mut self, state: TrayState) {
        if state == self.last {
            return;
        }
        self.last = state.clone();
        let h = self.handle.clone();
        crate::live::runtime().spawn(async move {
            h.update(|t| t.state = state).await;
        });
    }
}

impl Drop for TrayHandle {
    fn drop(&mut self) {
        // The signal is sent right away; no need to wait for the service to stop.
        drop(self.handle.shutdown());
    }
}

/// Starts the tray icon; reports `Msg::TrayStarted` or `Msg::TrayGone`.
pub fn spawn(tx: relm4::Sender<Msg>) {
    let t2 = tx.clone();
    crate::live::runtime().spawn(async move {
        let tray = Tray {
            state: TrayState::default(),
            tx: t2.clone(),
        };
        match tray.spawn().await {
            Ok(handle) => t2.emit(Msg::TrayStarted(Box::new(TrayHandle {
                handle,
                last: TrayState::default(),
            }))),
            Err(e) => {
                eprintln!("tray: {e}");
                tx.emit(Msg::TrayGone);
            }
        }
    });
}
