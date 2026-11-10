//! Clients: pending approvals, connected devices, blocked devices.

use std::cell::RefCell;
use std::collections::HashMap;

use leh_model::policy::Access;
use relm4::adw::prelude::*;
use relm4::{Sender, adw, gtk};

use crate::app::Msg;
use crate::state::{ClientVm, RunState, UiState, fmt_rate, policy_summary};

pub struct ClientsPage {
    pub root: gtk::Stack,
    status: adw::StatusPage,
    pending_title: gtk::Label,
    pending: gtk::ListBox,
    connected_title: gtk::Label,
    connected: gtk::ListBox,
    blocked: gtk::ListBox,
    /// Structure currently rendered; rows are rebuilt only when it changes.
    shown_key: RefCell<String>,
    /// Live labels updated in place every tick.
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
        .css_classes(["heading"])
        .build()
}

fn subtitle(s: &UiState, c: &ClientVm, helper: bool) -> String {
    let ip = c.ip.clone().unwrap_or_else(|| "no address yet".into());
    if !helper {
        let vendor = c.vendor.unwrap_or("Unknown vendor");
        let short_mac: String = c.mac.chars().take(8).collect();
        return format!("{ip} · {vendor} · {short_mac}…");
    }
    match s.effective(c).access {
        Access::Pending => format!("{ip} · {}", c.vendor.unwrap_or("Unknown vendor")),
        _ => policy_summary(s, c),
    }
}

impl ClientsPage {
    pub fn new(tx: Sender<Msg>) -> Self {
        let status = adw::StatusPage::builder()
            .icon_name("phone-symbolic")
            .build();

        let pending_title = heading("Waiting for approval");
        let pending = list();
        let connected_title = heading("Connected");
        let connected = list();
        let blocked = list();

        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&pending_title);
        content.append(&pending);
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_height_request(12);
        content.append(&spacer);
        content.append(&connected_title);
        content.append(&connected);
        let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        spacer.set_height_request(12);
        content.append(&spacer);
        content.append(&blocked);

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
            pending_title,
            pending,
            connected_title,
            connected,
            blocked,
            shown_key: RefCell::default(),
            live: RefCell::default(),
            tx,
        }
    }

    fn row(&self, s: &UiState, c: &ClientVm, helper: bool) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(glib_escape(&s.display_name(c)))
            .subtitle(glib_escape(&subtitle(s, c, helper)))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(c.icon.icon_name()));
        if helper {
            let rates = gtk::Label::builder()
                .css_classes(["caption", "numeric", "dim-label"])
                .build();
            let eff = s.effective(c);
            if eff.cap_reached {
                let pill = gtk::Label::builder()
                    .label("Cap reached")
                    .valign(gtk::Align::Center)
                    .css_classes(["caption", "warning", "pill-tag"])
                    .build();
                row.add_suffix(&pill);
            }
            row.add_suffix(&rates);
            row.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
            row.set_activatable(true);
            let (tx, mac) = (self.tx.clone(), c.mac.clone());
            row.connect_activated(move |_| tx.emit(Msg::OpenDevice(mac.clone())));
            self.live
                .borrow_mut()
                .insert(c.mac.clone(), (row.clone(), rates));
        }
        row
    }

    fn structure_key(s: &UiState) -> String {
        let mut k = format!("{:?}|{}|", s.run, s.helper_ready());
        for c in &s.clients {
            let e = s.effective(c);
            k.push_str(&format!(
                "{}:{}:{:?}:{};",
                c.mac,
                s.display_name(c),
                e.access,
                e.cap_reached
            ));
        }
        k
    }

    pub fn update(&self, s: &UiState) {
        if s.run != RunState::On {
            self.status.set_title("Hotspot is off");
            self.status
                .set_description(Some("Start the hotspot to see connected devices."));
            self.status.set_icon_name(Some(crate::ICON_SYMBOLIC));
            self.root.set_visible_child_name("status");
            self.shown_key.borrow_mut().clear();
            return;
        }
        if s.clients.is_empty() {
            self.status.set_title("No devices yet");
            self.status
                .set_description(Some("Scan the QR code on Home to connect."));
            self.status.set_icon_name(Some("phone-symbolic"));
            self.root.set_visible_child_name("status");
            self.shown_key.borrow_mut().clear();
            return;
        }
        self.root.set_visible_child_name("list");
        let helper = s.helper_ready();

        let key = Self::structure_key(s);
        if *self.shown_key.borrow() != key {
            self.live.borrow_mut().clear();
            self.pending.remove_all();
            self.connected.remove_all();
            self.blocked.remove_all();
            let mut n_pending = 0;
            let mut n_conn = 0;
            let blocked_exp = adw::ExpanderRow::new();
            let mut n_blocked = 0;
            for c in &s.clients {
                match s.effective(c).access {
                    Access::Pending if helper => {
                        n_pending += 1;
                        let row = self.row(s, c, false);
                        row.set_subtitle(&glib_escape(&subtitle(s, c, true)));
                        let block = gtk::Button::builder()
                            .label("Block")
                            .valign(gtk::Align::Center)
                            .build();
                        let allow = gtk::Button::builder()
                            .label("Allow")
                            .valign(gtk::Align::Center)
                            .css_classes(["suggested-action"])
                            .build();
                        let (tx, mac) = (self.tx.clone(), c.mac.clone());
                        block.connect_clicked(move |_| tx.emit(Msg::Block(mac.clone())));
                        let (tx, mac) = (self.tx.clone(), c.mac.clone());
                        allow.connect_clicked(move |_| tx.emit(Msg::Allow(mac.clone())));
                        let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                        b.append(&block);
                        b.append(&allow);
                        row.add_suffix(&b);
                        self.pending.append(&row);
                    }
                    Access::Blocked if helper && !s.effective(c).cap_reached => {
                        n_blocked += 1;
                        blocked_exp.add_row(&self.row(s, c, true));
                    }
                    _ => {
                        n_conn += 1;
                        self.connected.append(&self.row(s, c, helper));
                    }
                }
            }
            self.pending_title.set_visible(n_pending > 0);
            self.pending.set_visible(n_pending > 0);
            self.connected_title
                .set_label(&format!("Connected · {n_conn}"));
            self.connected.set_visible(n_conn > 0);
            blocked_exp.set_title(&format!("Blocked · {n_blocked}"));
            self.blocked.append(&blocked_exp);
            self.blocked.set_visible(n_blocked > 0);
            *self.shown_key.borrow_mut() = key;
        }

        for c in &s.clients {
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
}

/// Row titles/subtitles are Pango markup; escape user-provided names.
pub fn glib_escape(s: &str) -> String {
    relm4::gtk::glib::markup_escape_text(s).to_string()
}
