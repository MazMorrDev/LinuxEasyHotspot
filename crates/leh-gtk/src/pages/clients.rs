//! Clients: hotspot usage, devices waiting for approval, and every known
//! device (connected or not) grouped into Allowed and Blocked. Each device
//! opens the device dialog, where its name, access and limits are set.

use crate::i18n::{tr, trf};
use std::cell::RefCell;
use std::collections::HashMap;

use leh_model::policy::Access;
use relm4::adw::prelude::*;
use relm4::{Sender, adw, gtk};

use crate::app::Msg;
use crate::state::{
    ClientVm, RunState, UiState, fmt_bytes, fmt_last_seen, fmt_rate, policy_summary,
};

pub struct ClientsPage {
    pub root: gtk::Stack,
    status: adw::StatusPage,
    off_note: gtk::Label,
    usage: gtk::ListBox,
    usage_now: gtk::Label,
    usage_total: gtk::Label,
    usage_total_row: adw::ActionRow,
    pending_title: gtk::Label,
    pending: gtk::ListBox,
    allowed_title: gtk::Label,
    allowed_note: gtk::Label,
    allowed: gtk::ListBox,
    blocked_title: gtk::Label,
    blocked_note: gtk::Label,
    blocked: gtk::ListBox,
    /// Structure currently rendered; rows are rebuilt only when it changes.
    shown_key: RefCell<String>,
    /// Live labels of connected devices, updated in place every tick.
    live: RefCell<HashMap<String, (adw::ActionRow, gtk::Label)>>,
    tx: Sender<Msg>,
}

fn list() -> gtk::ListBox {
    gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build()
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Start)
        .margin_top(12)
        .css_classes(["heading"])
        .build()
}

fn note() -> gtk::Label {
    gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .css_classes(["caption", "dim-label"])
        .visible(false)
        .build()
}

fn subtitle(s: &UiState, c: &ClientVm, helper: bool) -> String {
    let ip = c.ip.clone().unwrap_or_else(|| tr("no address yet").into());
    if !helper {
        let vendor = c.vendor.unwrap_or(tr("Unknown vendor"));
        return format!("{ip} · {vendor}");
    }
    match s.effective(c).access {
        Access::Pending => format!("{ip} · {}", c.vendor.unwrap_or(tr("Unknown vendor"))),
        _ => policy_summary(s, c),
    }
}

/// A known device that isn't connected, as a view-model for its row.
fn offline_vm(s: &UiState, mac: &str) -> ClientVm {
    let hostname = s.policies.get(mac).and_then(|p| p.hostname.clone());
    crate::live::client_vm(&leh_model::clients::Client {
        mac: mac.to_owned(),
        ip: None,
        hostname,
    })
}

impl ClientsPage {
    pub fn new(tx: Sender<Msg>) -> Self {
        let status = adw::StatusPage::builder()
            .icon_name("phone-symbolic")
            .build();

        let off_note = note();
        off_note.set_label(tr(
            "The hotspot is off. You can still change the settings of each device; they apply the next time it connects.",
        ));

        // Whole-hotspot usage.
        let usage = list();
        let usage_now = gtk::Label::builder()
            .css_classes(["numeric"])
            .valign(gtk::Align::Center)
            .build();
        let now_row = adw::ActionRow::builder()
            .title(tr("Speed right now"))
            .subtitle(tr("All devices together"))
            .build();
        now_row.add_prefix(&gtk::Image::from_icon_name(
            "network-transmit-receive-symbolic",
        ));
        now_row.add_suffix(&usage_now);
        let usage_total = gtk::Label::builder()
            .css_classes(["numeric"])
            .valign(gtk::Align::Center)
            .build();
        let usage_total_row = adw::ActionRow::builder()
            .title(tr("Used since the hotspot started"))
            .build();
        usage_total_row.add_prefix(&gtk::Image::from_icon_name("folder-download-symbolic"));
        usage_total_row.add_suffix(&usage_total);
        usage.append(&now_row);
        usage.append(&usage_total_row);

        let pending_title = heading(tr("Waiting for approval"));
        let pending = list();
        let allowed_title = heading("");
        let allowed_note = note();
        let allowed = list();
        let blocked_title = heading("");
        let blocked_note = note();
        let blocked = list();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&off_note);
        content.append(&usage);
        for w in [
            pending_title.upcast_ref::<gtk::Widget>(),
            pending.upcast_ref(),
            allowed_title.upcast_ref(),
            allowed_note.upcast_ref(),
            allowed.upcast_ref(),
            blocked_title.upcast_ref(),
            blocked_note.upcast_ref(),
            blocked.upcast_ref(),
        ] {
            content.append(w);
        }

        let clamp = adw::Clamp::builder()
            .maximum_size(600)
            .child(&content)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(16)
            .margin_end(16)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .build();

        let root = gtk::Stack::new();
        root.add_named(&status, Some("status"));
        root.add_named(&scroll, Some("list"));

        Self {
            root,
            status,
            off_note,
            usage,
            usage_now,
            usage_total,
            usage_total_row,
            pending_title,
            pending,
            allowed_title,
            allowed_note,
            allowed,
            blocked_title,
            blocked_note,
            blocked,
            shown_key: RefCell::default(),
            live: RefCell::default(),
            tx,
        }
    }

    /// Row for a connected device; opens the device dialog. `rates` shows
    /// its live speed (needs the helper); `actions` adds quick buttons
    /// (`Some(true)`: Block + Allow, `Some(false)`: Allow).
    fn row(&self, s: &UiState, c: &ClientVm, rates: bool, actions: Option<bool>) -> adw::ActionRow {
        let helper = s.helper_ready();
        let row = adw::ActionRow::builder()
            .title(glib_escape(&s.display_name(c)))
            .title_lines(1)
            .subtitle(glib_escape(&subtitle(s, c, helper)))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(c.icon.icon_name()));
        let rate_label = gtk::Label::builder()
            .css_classes(["caption", "numeric", "dim-label"])
            .visible(rates)
            .build();
        if helper && s.effective(c).cap_reached {
            let pill = gtk::Label::builder()
                .label(tr("Cap reached"))
                .valign(gtk::Align::Center)
                .css_classes(["caption", "warning", "pill-tag"])
                .build();
            row.add_suffix(&pill);
        }
        row.add_suffix(&rate_label);
        self.live
            .borrow_mut()
            .insert(c.mac.clone(), (row.clone(), rate_label));
        self.finish_row(&row, &c.mac, actions);
        row
    }

    /// Row for a known device that isn't connected right now.
    fn offline_row(&self, s: &UiState, mac: &str, actions: Option<bool>) -> adw::ActionRow {
        let vm = offline_vm(s, mac);
        let last_seen = s.policies.get(mac).and_then(|p| p.last_seen);
        let vendor = vm.vendor.unwrap_or(tr("Unknown vendor"));
        let row = adw::ActionRow::builder()
            .title(glib_escape(&s.known_name(mac)))
            .title_lines(1)
            .subtitle(glib_escape(&format!(
                "{} · {vendor}",
                fmt_last_seen(last_seen)
            )))
            .build();
        let icon = gtk::Image::from_icon_name(vm.icon.icon_name());
        icon.add_css_class("dim-label");
        row.add_prefix(&icon);
        self.finish_row(&row, mac, actions);
        row
    }

    /// Quick buttons (if any), then the arrow that opens the device dialog.
    fn finish_row(&self, row: &adw::ActionRow, mac: &str, actions: Option<bool>) {
        if let Some(block) = actions {
            self.add_actions(row, mac, block);
        }
        row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        row.set_activatable(true);
        let (tx, mac) = (self.tx.clone(), mac.to_owned());
        row.connect_activated(move |_| tx.emit(Msg::OpenDevice(mac.clone())));
    }

    /// Adds "Block"/"Allow" (or just "Allow") buttons to a row.
    fn add_actions(&self, row: &adw::ActionRow, mac: &str, block: bool) {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        b.set_valign(gtk::Align::Center);
        if block {
            let btn = gtk::Button::builder().label(tr("Block")).build();
            let (tx, mac) = (self.tx.clone(), mac.to_owned());
            btn.connect_clicked(move |_| tx.emit(Msg::Block(mac.clone())));
            b.append(&btn);
        }
        let allow = gtk::Button::builder()
            .label(tr("Allow"))
            .css_classes(if block {
                vec!["suggested-action"]
            } else {
                vec!["flat"]
            })
            .build();
        let (tx, mac) = (self.tx.clone(), mac.to_owned());
        allow.connect_clicked(move |_| tx.emit(Msg::Allow(mac.clone())));
        b.append(&allow);
        row.add_suffix(&b);
    }

    fn structure_key(s: &UiState) -> String {
        let mut k = format!("{:?}|{}|", s.run, s.helper_ready());
        for c in s.online_clients() {
            let e = s.effective(c);
            k.push_str(&format!(
                "{}:{}:{:?}:{}:{};",
                c.mac,
                s.display_name(c),
                e.access,
                e.cap_reached,
                c.ip.as_deref().unwrap_or_default(),
            ));
        }
        k.push('|');
        for (mac, p) in s.offline_devices() {
            k.push_str(&format!(
                "{mac}:{}:{:?}:{};",
                s.known_name(mac),
                p.access,
                fmt_last_seen(p.last_seen)
            ));
        }
        k
    }

    pub fn update(&self, s: &UiState) {
        let running = s.run == RunState::On;
        let has_known = !s.policies.is_empty();
        if !running && !has_known {
            self.status.set_title(tr("Hotspot is off"));
            self.status
                .set_description(Some(tr("Start the hotspot to see connected devices.")));
            self.status.set_icon_name(Some(crate::ICON_SYMBOLIC));
            self.root.set_visible_child_name("status");
            self.shown_key.borrow_mut().clear();
            return;
        }
        if running && s.clients.is_empty() && !has_known {
            self.status.set_title(tr("No devices yet"));
            self.status
                .set_description(Some(tr("Scan the QR code on Home to connect.")));
            self.status.set_icon_name(Some("phone-symbolic"));
            self.root.set_visible_child_name("status");
            self.shown_key.borrow_mut().clear();
            return;
        }
        self.root.set_visible_child_name("list");
        let helper = s.helper_ready();

        self.off_note.set_visible(!running);
        self.usage.set_visible(running);
        if running {
            let t = &s.traffic;
            self.usage_now.set_label(&format!(
                "↓ {}   ↑ {}",
                fmt_rate(t.down_kbps),
                fmt_rate(t.up_kbps)
            ));
            self.usage_total
                .set_label(&fmt_bytes(t.down_bytes + t.up_bytes));
            self.usage_total_row.set_subtitle(&trf(
                "Downloaded {down} · uploaded {up}",
                &[
                    ("down", &fmt_bytes(t.down_bytes)),
                    ("up", &fmt_bytes(t.up_bytes)),
                ],
            ));
        }

        let key = Self::structure_key(s);
        if *self.shown_key.borrow() != key {
            self.rebuild(s, helper);
            *self.shown_key.borrow_mut() = key;
        }

        for c in s.online_clients() {
            if let Some((row, rates)) = self.live.borrow().get(&c.mac) {
                rates.set_label(&format!(
                    "↓{} ↑{}",
                    fmt_rate(c.down_kbps),
                    fmt_rate(c.up_kbps)
                ));
                row.set_subtitle(&glib_escape(&subtitle(s, c, helper)));
            }
        }
    }

    fn rebuild(&self, s: &UiState, helper: bool) {
        self.live.borrow_mut().clear();
        self.pending.remove_all();
        self.allowed.remove_all();
        self.blocked.remove_all();
        let (mut n_pending, mut n_allowed, mut n_blocked, mut n_online) = (0, 0, 0, 0);

        // Connected devices first, in each group.
        for c in s.online_clients() {
            let access = s.policy(&c.mac).access;
            match access {
                // Approval is only enforced with the helper.
                Access::Pending if helper => {
                    n_pending += 1;
                    self.pending.append(&self.row(s, c, false, Some(true)));
                }
                Access::Blocked => {
                    n_blocked += 1;
                    self.blocked.append(&self.row(s, c, false, Some(false)));
                }
                _ => {
                    n_allowed += 1;
                    n_online += 1;
                    self.allowed.append(&self.row(s, c, helper, None));
                }
            }
        }
        for (mac, p) in s.offline_devices() {
            match p.access {
                Access::Pending => {
                    n_pending += 1;
                    self.pending.append(&self.offline_row(s, mac, Some(true)));
                }
                Access::Blocked => {
                    n_blocked += 1;
                    self.blocked.append(&self.offline_row(s, mac, Some(false)));
                }
                Access::Allowed => {
                    n_allowed += 1;
                    self.allowed.append(&self.offline_row(s, mac, None));
                }
            }
        }

        self.pending_title.set_visible(n_pending > 0);
        self.pending.set_visible(n_pending > 0);

        self.allowed_title
            .set_label(&trf("Allowed · {n}", &[("n", &n_allowed.to_string())]));
        self.allowed.set_visible(n_allowed > 0);
        let allowed_note = if !running(s) {
            None
        } else if n_online == 0 {
            Some(
                tr("No device is connected right now. Scan the QR code on Home to connect.")
                    .to_owned(),
            )
        } else {
            Some(crate::i18n::tr_n(
                "{n} connected now",
                "{n} connected now",
                n_online,
            ))
        };
        self.allowed_note.set_visible(allowed_note.is_some());
        self.allowed_note
            .set_label(allowed_note.as_deref().unwrap_or_default());

        self.blocked_title
            .set_label(&trf("Blocked · {n}", &[("n", &n_blocked.to_string())]));
        self.blocked_title.set_visible(n_blocked > 0);
        self.blocked.set_visible(n_blocked > 0);
        self.blocked_note.set_visible(n_blocked > 0 && !helper);
        self.blocked_note.set_label(tr(
            "Blocking needs the Linux Easy Hotspot helper. Until it's set up, these devices can still connect.",
        ));
    }
}

fn running(s: &UiState) -> bool {
    s.run == RunState::On
}

/// Row titles/subtitles are Pango markup; escape user-provided names.
pub fn glib_escape(s: &str) -> String {
    relm4::gtk::glib::markup_escape_text(s).to_string()
}
