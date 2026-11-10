//! Per-device detail dialog (adaptive: bottom sheet on narrow windows).

use crate::i18n::{tr, trf};
use leh_model::policy::Access;
use relm4::adw::prelude::*;
use relm4::{Sender, adw, gtk};

use crate::app::Msg;
use crate::state::{UiState, fmt_bytes, fmt_rate};
use crate::widgets::{Guard, LimitsEditor, combo, group, set_combo, set_text, switch_row};

const ACCESS: [Access; 3] = [Access::Allowed, Access::Pending, Access::Blocked];

pub struct DeviceDialog {
    pub dialog: adw::Dialog,
    pub mac: String,
    /// Takes initial focus so the dialog doesn't open editing the name.
    pub scroll: gtk::ScrolledWindow,
    rates: gtk::Label,
    usage: gtk::Label,
    level: gtk::LevelBar,
    name: adw::EntryRow,
    address: adw::ActionRow,
    access: adw::ComboRow,
    follow: adw::SwitchRow,
    limits: LimitsEditor,
    guard: Guard,
}

impl DeviceDialog {
    pub fn new(tx: Sender<Msg>, mac: &str) -> Self {
        let guard = Guard::default();
        let rates = gtk::Label::builder()
            .css_classes(["title-2", "numeric"])
            .build();
        let usage = gtk::Label::builder().css_classes(["dim-label"]).build();
        let level = gtk::LevelBar::builder()
            .min_value(0.0)
            .max_value(1.0)
            .build();
        let header = gtk::Box::new(gtk::Orientation::Vertical, 6);
        header.append(&rates);
        header.append(&usage);
        header.append(&level);

        let device = group(tr("Device"));
        let name = adw::EntryRow::builder()
            .title(tr("Name"))
            .show_apply_button(true)
            .build();
        let address = adw::ActionRow::builder().title(tr("Address")).build();
        address.add_css_class("property");
        address.set_subtitle_selectable(true);
        let access = combo(
            tr("Access"),
            &[tr("Allowed"), tr("Waiting for approval"), tr("Blocked")],
        );
        device.add(&name);
        device.add(&address);
        device.add(&access);

        let limits_group = group(tr("Limits"));
        let follow = switch_row(
            tr("Use default settings"),
            tr("Follow the settings for new devices"),
        );
        limits_group.add(&follow);
        let (t, m) = (tx.clone(), mac.to_owned());
        let limits = LimitsEditor::new(move |f| t.emit(Msg::DeviceLimit(m.clone(), f)));
        limits.add_to(&limits_group);

        let forget = gtk::Button::builder()
            .label(tr("Forget this device"))
            .halign(gtk::Align::Center)
            .css_classes(["pill", "destructive-action"])
            .build();

        let (t, m) = (tx.clone(), mac.to_owned());
        name.connect_apply(move |e| t.emit(Msg::DeviceAlias(m.clone(), e.text().into())));
        let (g, t, m) = (guard.clone(), tx.clone(), mac.to_owned());
        access.connect_selected_notify(move |r| {
            if !g.active()
                && let Some(a) = ACCESS.get(r.selected() as usize)
            {
                t.emit(Msg::DeviceAccess(m.clone(), *a));
            }
        });
        let (g, t, m) = (guard.clone(), tx.clone(), mac.to_owned());
        follow.connect_active_notify(move |r| {
            if !g.active() {
                t.emit(Msg::DeviceFollow(m.clone(), r.is_active()));
            }
        });

        let content = gtk::Box::new(gtk::Orientation::Vertical, 24);
        content.set_margin_top(12);
        content.set_margin_bottom(24);
        content.set_margin_start(16);
        content.set_margin_end(16);
        content.append(&header);
        content.append(&device);
        content.append(&limits_group);
        content.append(&forget);
        let clamp = adw::Clamp::builder()
            .maximum_size(520)
            .child(&content)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .focusable(true)
            .child(&clamp)
            .build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&adw::HeaderBar::new());
        toolbar.set_content(Some(&scroll));

        let dialog = adw::Dialog::builder()
            .child(&toolbar)
            .content_width(460)
            .content_height(640)
            .follows_content_size(false)
            .build();
        let t = tx.clone();
        dialog.connect_closed(move |_| t.emit(Msg::DeviceClosed));

        let (t, m, d) = (tx.clone(), mac.to_owned(), dialog.clone());
        forget.connect_clicked(move |_| {
            let confirm = adw::AlertDialog::new(
                Some(tr("Forget this device?")),
                Some(tr("Its name and limits are removed. If it connects again, it is treated as a new device.")),
            );
            confirm.add_response("cancel", tr("Cancel"));
            confirm.add_response("forget", tr("Forget"));
            confirm.set_response_appearance("forget", adw::ResponseAppearance::Destructive);
            let (t, m) = (t.clone(), m.clone());
            confirm.connect_response(Some("forget"), move |_, _| t.emit(Msg::ForgetDevice(m.clone())));
            confirm.present(Some(&d));
        });

        Self {
            dialog,
            mac: mac.to_owned(),
            scroll,
            rates,
            usage,
            level,
            name,
            address,
            access,
            follow,
            limits,
            guard,
        }
    }

    pub fn update(&self, s: &UiState) {
        let Some(c) = s.client(&self.mac) else {
            self.dialog.close();
            return;
        };
        let pol = s.policy(&c.mac);
        let lim = pol.limits(&s.defaults);
        let eff = s.effective(c);
        self.dialog.set_title(&s.display_name(c));
        self.rates.set_label(&format!(
            "↓ {}   ↑ {}",
            fmt_rate(c.down_kbps),
            fmt_rate(c.up_kbps)
        ));
        match lim.cap_mb {
            Some(cap) => {
                let frac = c.session_bytes as f64 / (f64::from(cap) * 1e6);
                let mut text = trf(
                    "This session: {used} of {cap}",
                    &[
                        ("used", &fmt_bytes(c.session_bytes)),
                        ("cap", &fmt_bytes(u64::from(cap) * 1_000_000)),
                    ],
                );
                if eff.cap_reached {
                    text.push_str(tr(" · cap reached"));
                }
                self.usage.set_label(&text);
                self.level.set_value(frac.min(1.0));
                self.level.set_visible(true);
            }
            None => {
                self.usage.set_label(&trf(
                    "This session: {used}",
                    &[("used", &fmt_bytes(c.session_bytes))],
                ));
                self.level.set_visible(false);
            }
        }
        self.guard.hold(|| {
            // The row itself never has focus, its inner text field does:
            // don't overwrite what the user is typing.
            if !self
                .name
                .state_flags()
                .contains(gtk::StateFlags::FOCUS_WITHIN)
            {
                set_text(&self.name, &s.display_name(c));
            }
            let idx = ACCESS.iter().position(|a| *a == pol.access).unwrap_or(0);
            set_combo(&self.access, idx as u32);
            if self.follow.is_active() != pol.follow_defaults {
                self.follow.set_active(pol.follow_defaults);
            }
        });
        let vendor = c.vendor.unwrap_or(tr("Unknown vendor"));
        self.address.set_subtitle(&format!(
            "{} · {} · {vendor}",
            c.ip.as_deref().unwrap_or(tr("no address")),
            c.mac
        ));
        self.limits.set_sensitive(!pol.follow_defaults);
        self.limits.set(lim);
    }
}
