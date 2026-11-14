//! StatusNotifierItem tray icon (ksni), shown when the desktop has a tray.

use ksni::TrayMethods as _;
use ksni::menu::{MenuItem, StandardItem};

use super::{Item, TrayState};
use crate::msg::{Msg, Tx};

struct Tray {
    state: TrayState,
    tx: Tx,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "easyhotspot".into()
    }

    fn title(&self) -> String {
        leh_model::APP_NAME.into()
    }

    fn icon_name(&self) -> String {
        crate::APP_ID.into()
    }

    fn icon_theme_path(&self) -> String {
        // Source builds: the icon isn't installed in the system theme.
        let dev = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons");
        if !crate::packaged() && std::path::Path::new(dev).exists() {
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
            title: leh_model::APP_NAME.into(),
            description: self.state.status_line(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.tx.emit(Msg::ShowWindow);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        super::menu(&self.state)
            .into_iter()
            .map(|item| match item {
                Item::Label(label) => StandardItem {
                    label,
                    enabled: false,
                    ..Default::default()
                }
                .into(),
                Item::Separator => MenuItem::Separator,
                Item::Action {
                    label,
                    enabled,
                    msg,
                    arg,
                } => StandardItem {
                    label,
                    enabled,
                    activate: Box::new(move |t: &mut Self| t.tx.emit(msg(&arg))),
                    ..Default::default()
                }
                .into(),
            })
            .collect()
    }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        self.tx.emit(Msg::TrayGone);
        // Keep the service so the icon comes back if the panel restarts.
        true
    }

    fn watcher_online(&self) {
        self.tx.emit(Msg::TrayReady);
    }
}

/// A running tray icon; [`TrayHandle::close`] removes it.
pub struct TrayHandle {
    handle: ksni::Handle<Tray>,
    last: TrayState,
}

impl std::fmt::Debug for TrayHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayHandle")
            .field("last", &self.last)
            .finish()
    }
}

impl TrayHandle {
    pub fn update(&mut self, state: TrayState) {
        if state == self.last {
            return;
        }
        self.last = state.clone();
        let h = self.handle.clone();
        crate::backend::runtime().spawn(async move {
            h.update(|t| t.state = state).await;
        });
    }

    pub fn close(self) {
        // The signal is sent right away; no need to wait for the service to stop.
        drop(self.handle.shutdown());
    }
}

/// Starts the tray icon; reports `Msg::TrayStarted` or `Msg::TrayGone`.
pub fn spawn(tx: Tx) {
    crate::backend::runtime().spawn(async move {
        let tray = Tray {
            state: TrayState::default(),
            tx: tx.clone(),
        };
        match tray.spawn().await {
            Ok(handle) => tx.emit(Msg::TrayStarted(TrayHandle {
                handle,
                last: TrayState::default(),
            })),
            Err(e) => {
                eprintln!("tray: {e}");
                tx.emit(Msg::TrayGone);
            }
        }
    });
}
