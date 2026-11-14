//! Notification-area icon (`tray-icon`). It must live on the UI thread,
//! whose message loop GPUI runs; the icon itself is kept in a thread-local
//! and [`TrayHandle`] is only a token for it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use super::{Item, TrayState};
use crate::msg::{Msg, Tx};

/// What each menu entry does: its message constructor and argument.
type Actions = Arc<Mutex<HashMap<MenuId, (fn(&str) -> Msg, String)>>>;

struct Tray {
    icon: TrayIcon,
    actions: Actions,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

#[derive(Debug)]
pub struct TrayHandle {
    last: TrayState,
}

impl TrayHandle {
    pub fn update(&mut self, state: TrayState) {
        if state == self.last {
            return;
        }
        TRAY.with_borrow(|t| {
            if let Some(t) = t {
                let _ = t.icon.set_tooltip(Some(tooltip(&state)));
                t.icon
                    .set_menu(Some(Box::new(build_menu(&state, &t.actions))));
            }
        });
        self.last = state;
    }

    pub fn close(self) {
        TRAY.with_borrow_mut(|t| t.take());
    }
}

fn tooltip(s: &TrayState) -> String {
    format!("{} · {}", leh_model::APP_NAME, s.status_line())
}

fn build_menu(s: &TrayState, actions: &Actions) -> Menu {
    let menu = Menu::new();
    let mut map = HashMap::new();
    for item in super::menu(s) {
        let res = match item {
            Item::Label(label) => menu.append(&MenuItem::new(label, false, None)),
            Item::Separator => menu.append(&PredefinedMenuItem::separator()),
            Item::Action {
                label,
                enabled,
                msg,
                arg,
            } => {
                let entry = MenuItem::new(label, enabled, None);
                map.insert(entry.id().clone(), (msg, arg));
                menu.append(&entry)
            }
        };
        if let Err(e) = res {
            eprintln!("tray: {e}");
        }
    }
    if let Ok(mut a) = actions.lock() {
        *a = map;
    }
    menu
}

/// A 32×32 icon drawn here, so no image files or decoders are needed: white
/// Wi-Fi waves on a blue rounded square.
fn icon() -> Option<Icon> {
    const N: usize = 32;
    let mut rgba = vec![0u8; N * N * 4];
    let (cx, cy) = (15.5_f32, 24.0_f32);
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = (x as f32, y as f32);
            // Rounded square background.
            let dx = (fx - 15.5).abs() - 10.0;
            let dy = (fy - 15.5).abs() - 10.0;
            let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - 5.5;
            if outside > 0.5 {
                continue;
            }
            let mut px = [0x35, 0x84, 0xe4, 0xff];
            // Waves: arcs around (cx, cy) within ±45°, plus a dot.
            let (vx, vy) = (fx - cx, cy - fy);
            let r = (vx * vx + vy * vy).sqrt();
            let in_cone = vy > 0.0 && vx.abs() <= vy;
            let wave = [6.5_f32, 11.5, 16.5].iter().any(|w| (r - w).abs() < 1.6);
            if r < 2.4 || (in_cone && wave) {
                px = [0xff, 0xff, 0xff, 0xff];
            }
            rgba[(y * N + x) * 4..][..4].copy_from_slice(&px);
        }
    }
    Icon::from_rgba(rgba, N as u32, N as u32).ok()
}

/// Creates the icon (on the UI thread) and reports `Msg::TrayStarted`.
pub fn spawn(tx: Tx) {
    let actions: Actions = Arc::default();
    let state = TrayState::default();
    let mut builder = TrayIconBuilder::new()
        .with_tooltip(tooltip(&state))
        .with_menu(Box::new(build_menu(&state, &actions)))
        .with_menu_on_left_click(false);
    if let Some(i) = icon() {
        builder = builder.with_icon(i);
    }
    let icon = match builder.build() {
        Ok(i) => i,
        Err(e) => {
            eprintln!("tray: {e}");
            tx.emit(Msg::TrayGone);
            return;
        }
    };
    let t = tx.clone();
    TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = e
        {
            t.emit(Msg::ShowWindow);
        }
    }));
    let (t, a) = (tx.clone(), actions.clone());
    MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
        let found = a.lock().ok().and_then(|m| m.get(&e.id).cloned());
        if let Some((msg, arg)) = found {
            t.emit(msg(&arg));
        }
    }));
    TRAY.with_borrow_mut(|slot| *slot = Some(Tray { icon, actions }));
    tx.emit(Msg::TrayStarted(TrayHandle { last: state }));
}
