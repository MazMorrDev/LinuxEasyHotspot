//! Settings: modes, adapters, radio, speed, new-device defaults, startup.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leh_model::{Band, Mode};
use relm4::adw::prelude::*;
use relm4::{Sender, adw, gtk};

use crate::app::Msg;
use crate::state::UiState;
use crate::widgets::{Guard, LimitsEditor, OptRow, combo, group, set_combo, switch_row};

/// (iface or `None` for automatic, label) entries shown in the source combo.
type SourceList = Rc<RefCell<Vec<(Option<String>, String)>>>;

const BANDS: [Band; 3] = [Band::Auto, Band::Ghz2_4, Band::Ghz5];

pub struct SettingsPage {
    pub root: adw::PreferencesPage,
    mode_rows: Vec<(Mode, adw::ActionRow, gtk::CheckButton)>,
    adapter: adw::ComboRow,
    upstream: adw::ComboRow,
    source_note: gtk::Label,
    band: adw::ComboRow,
    channel: adw::ComboRow,
    hidden: adw::SwitchRow,
    speed: adw::PreferencesGroup,
    startup: adw::PreferencesGroup,
    total_down: OptRow,
    total_up: OptRow,
    new_devices: adw::PreferencesGroup,
    needs_helper: gtk::Label,
    approval: adw::SwitchRow,
    defaults: LimitsEditor,
    autostart: adw::SwitchRow,
    start_at_login: adw::SwitchRow,
    tray: adw::SwitchRow,
    /// Rendered lists, to rebuild combo models only when they change.
    adapters_shown: RefCell<Vec<String>>,
    /// (iface or None for automatic, label) currently in the combo.
    upstreams_shown: SourceList,
    band_shown: Cell<Option<Band>>,
    guard: Guard,
}

fn channels(band: Band) -> Vec<String> {
    std::iter::once("Automatic".to_string())
        .chain(band.allowed_channels().iter().map(|c| c.to_string()))
        .collect()
}

impl SettingsPage {
    pub fn new(tx: Sender<Msg>) -> Self {
        let guard = Guard::default();
        let root = adw::PreferencesPage::new();

        // Connection mode
        let modes = group("Connection mode");
        let mut mode_rows = Vec::new();
        let mut first: Option<gtk::CheckButton> = None;
        for (mode, title, sub) in [
            (
                Mode::Router,
                "Router",
                "Devices share this computer's internet",
            ),
            (
                Mode::Bridge,
                "Bridge",
                "Devices join your wired network directly",
            ),
            (
                Mode::LocalOnly,
                "Local only",
                "Devices can reach each other, but not the internet",
            ),
        ] {
            let check = gtk::CheckButton::new();
            if let Some(f) = &first {
                check.set_group(Some(f));
            } else {
                first = Some(check.clone());
            }
            let row = adw::ActionRow::builder()
                .title(title)
                .subtitle(sub)
                .activatable_widget(&check)
                .build();
            row.add_prefix(&check);
            let (g, t) = (guard.clone(), tx.clone());
            check.connect_toggled(move |c| {
                if c.is_active() && !g.active() {
                    t.emit(Msg::Mode(mode));
                }
            });
            modes.add(&row);
            mode_rows.push((mode, row, check));
        }
        root.add(&modes);

        // Adapters
        let adapters = group("Adapters");
        let adapter = combo("Hotspot adapter", &[]);
        let upstream = combo("Internet source", &[]);
        crate::widgets::short_display(&upstream);
        crate::widgets::short_display(&adapter);
        adapters.add(&adapter);
        adapters.add(&upstream);
        let source_note = gtk::Label::builder()
            .wrap(true)
            .xalign(0.0)
            .margin_top(8)
            .css_classes(["caption"])
            .visible(false)
            .build();
        adapters.add(&source_note);
        let upstreams_shown: SourceList = Rc::default();
        root.add(&adapters);
        let (g, t) = (guard.clone(), tx.clone());
        adapter.connect_selected_notify(move |r| {
            if g.active() {
                return;
            }
            let name = r
                .selected_item()
                .and_downcast::<gtk::StringObject>()
                .map(|s| s.string().split(' ').next().unwrap_or_default().to_owned());
            t.emit(Msg::Adapter(name));
        });
        let (g, t, shown) = (guard.clone(), tx.clone(), upstreams_shown.clone());
        upstream.connect_selected_notify(move |r| {
            if g.active() {
                return;
            }
            if let Some((iface, _)) = shown.borrow().get(r.selected() as usize) {
                t.emit(Msg::Upstream(iface.clone()));
            }
        });

        // Radio
        let radio = group("Radio");
        let band = combo("Band", &["Automatic", "2.4 GHz", "5 GHz"]);
        let channel = combo("Channel", &["Automatic"]);
        let hidden = switch_row("Hidden network", "Devices must type the name to join");
        radio.add(&band);
        radio.add(&channel);
        radio.add(&hidden);
        root.add(&radio);
        let (g, t) = (guard.clone(), tx.clone());
        band.connect_selected_notify(move |r| {
            if !g.active()
                && let Some(b) = BANDS.get(r.selected() as usize)
            {
                t.emit(Msg::Band(*b));
            }
        });
        let (g, t) = (guard.clone(), tx.clone());
        channel.connect_selected_notify(move |r| {
            if g.active() {
                return;
            }
            let ch = r
                .selected_item()
                .and_downcast::<gtk::StringObject>()
                .and_then(|s| s.string().parse().ok())
                .unwrap_or(0);
            t.emit(Msg::Channel(ch));
        });
        let (g, t) = (guard.clone(), tx.clone());
        hidden.connect_active_notify(move |r| {
            if !g.active() {
                t.emit(Msg::Hidden(r.is_active()));
            }
        });

        // Connection speed
        let speed = group("Connection speed");
        speed.set_description(Some("Limits for the whole hotspot. Set them to your real connection speed for priorities to work well."));
        let t = tx.clone();
        let total_down = OptRow::new(
            "Total download speed",
            "Mbps, shared by all devices",
            "Unlimited",
            (1.0, 1000.0, 1.0),
            0,
            1.0,
            move |v| t.emit(Msg::TotalDown(v)),
        );
        let t = tx.clone();
        let total_up = OptRow::new(
            "Total upload speed",
            "Mbps, shared by all devices",
            "Unlimited",
            (1.0, 1000.0, 1.0),
            0,
            1.0,
            move |v| t.emit(Msg::TotalUp(v)),
        );
        speed.add(&total_down.row);
        speed.add(&total_up.row);
        root.add(&speed);

        // New devices
        let new_devices = group("New devices");
        new_devices.set_description(Some(
            "Applied automatically the first time a device connects.",
        ));
        let needs_helper = gtk::Label::builder()
            .label("Needs helper")
            .valign(gtk::Align::Center)
            .css_classes(["caption", "dim-label"])
            .build();
        new_devices.set_header_suffix(Some(&needs_helper));
        let approval = switch_row(
            "Ask before letting devices join",
            "New devices wait until you allow them",
        );
        new_devices.add(&approval);
        let (g, t) = (guard.clone(), tx.clone());
        approval.connect_active_notify(move |r| {
            if !g.active() {
                t.emit(Msg::Approval(r.is_active()));
            }
        });
        let t = tx.clone();
        let defaults = LimitsEditor::new(move |f| t.emit(Msg::Default(f)));
        defaults.add_to(&new_devices);
        root.add(&new_devices);

        // Startup
        let startup = group("Startup");
        let autostart = switch_row("Launch at login", "Start in the background when you log in");
        let start_at_login = switch_row("Start hotspot at login", "");
        let tray = switch_row("Show in system tray", "");
        startup.add(&autostart);
        startup.add(&start_at_login);
        startup.add(&tray);
        root.add(&startup);
        for (row, mk) in [
            (&autostart, Msg::Autostart as fn(bool) -> Msg),
            (&start_at_login, Msg::StartAtLogin),
            (&tray, Msg::Tray),
        ] {
            let (g, t) = (guard.clone(), tx.clone());
            row.connect_active_notify(move |r| {
                if !g.active() {
                    t.emit(mk(r.is_active()));
                }
            });
        }

        // Troubleshooting
        let trouble = group("Troubleshooting");
        let diag = adw::ActionRow::builder()
            .title("Export diagnostics…")
            .subtitle("Save a report to attach to bug reports (passwords removed)")
            .activatable(true)
            .build();
        diag.add_suffix(&gtk::Image::from_icon_name("go-next-symbolic"));
        let t = tx.clone();
        diag.connect_activated(move |_| t.emit(Msg::ExportDiagnostics));
        trouble.add(&diag);
        root.add(&trouble);

        Self {
            root,
            mode_rows,
            adapter,
            upstream,
            source_note,
            band,
            channel,
            hidden,
            speed,
            startup,
            total_down,
            total_up,
            new_devices,
            needs_helper,
            approval,
            defaults,
            autostart,
            start_at_login,
            tray,
            adapters_shown: RefCell::default(),
            upstreams_shown,
            band_shown: Cell::new(None),
            guard,
        }
    }

    pub fn update(&self, s: &UiState) {
        let editable = s.run == crate::state::RunState::Off;
        self.guard.hold(|| {
            for (mode, row, check) in &self.mode_rows {
                if *mode == Mode::LocalOnly {
                    row.set_sensitive(s.local_only_supported && editable);
                    if !s.local_only_supported {
                        row.set_subtitle(&format!(
                            "Needs NetworkManager 1.52 or newer (you have {})",
                            s.nm_version
                        ));
                    }
                } else {
                    row.set_sensitive(editable);
                }
                if check.is_active() != (s.cfg.mode == *mode) {
                    check.set_active(s.cfg.mode == *mode);
                }
            }

            // Adapters
            let adapters: Vec<String> = s
                .adapters
                .iter()
                .map(|(i, five)| {
                    format!("{i} {}", if *five { "(2.4 + 5 GHz)" } else { "(2.4 GHz)" })
                })
                .collect();
            if *self.adapters_shown.borrow() != adapters {
                let refs: Vec<&str> = adapters.iter().map(String::as_str).collect();
                self.adapter.set_model(Some(&gtk::StringList::new(&refs)));
                *self.adapters_shown.borrow_mut() = adapters;
            }
            let ai = s
                .adapters
                .iter()
                .position(|(i, _)| Some(i.as_str()) == s.cfg.ap_iface.as_deref())
                .unwrap_or(0);
            set_combo(&self.adapter, ai as u32);
            self.adapter.set_subtitle(match s.adapters.get(ai) {
                Some((_, true)) => "Supports 2.4 and 5 GHz",
                Some((_, false)) => "Supports 2.4 GHz",
                None => "No adapter can create hotspots",
            });
            self.adapter.set_sensitive(editable);

            let bridge = s.cfg.mode == Mode::Bridge;
            let mut entries: Vec<(Option<String>, String)> = Vec::new();
            if !bridge {
                entries.push((None, "Automatic (same as this computer)".into()));
            }
            entries.extend(
                s.uplinks
                    .iter()
                    .filter(|u| !bridge || u.can_bridge())
                    .map(|u| (Some(u.iface.clone()), u.label())),
            );
            if *self.upstreams_shown.borrow() != entries {
                let refs: Vec<&str> = entries.iter().map(|(_, l)| l.as_str()).collect();
                self.upstream.set_model(Some(&gtk::StringList::new(&refs)));
                *self.upstreams_shown.borrow_mut() = entries.clone();
            }
            let ui = entries
                .iter()
                .position(|(i, _)| i.as_deref() == s.cfg.upstream_iface.as_deref())
                .unwrap_or(0);
            set_combo(&self.upstream, ui as u32);
            self.upstream.set_title(if bridge {
                "Wired network to bridge"
            } else {
                "Internet source"
            });
            let selected = s.cfg.upstream_iface.as_deref().and_then(|i| s.uplink(i));
            let detail = match selected {
                Some(u) => u.detail(),
                None if bridge => "No wired connection selected".into(),
                None => match s.default_uplink() {
                    Some(d) => format!("Same as this computer: {}", d.label()),
                    None => "Same as this computer".into(),
                },
            };
            self.upstream.set_subtitle(&detail);
            self.upstream.set_sensitive(editable && !entries.is_empty());

            let note = if bridge {
                if entries.is_empty() {
                    Some("No wired connection found. Plug in an Ethernet cable or USB tethering, or use Router mode.".to_string())
                } else {
                    Some("Only wired connections can be bridged. To share Wi-Fi or mobile internet, use Router mode.".to_string())
                }
            } else {
                match (selected, s.default_uplink()) {
                    (Some(sel), Some(def)) if sel.connected && !sel.is_default => Some(format!(
                        "Hotspot traffic currently goes through {}. Sending only the hotspot through {} needs the upcoming helper.",
                        def.label(),
                        sel.label()
                    )),
                    _ => None,
                }
            };
            self.source_note.set_visible(note.is_some());
            self.source_note.set_label(note.as_deref().unwrap_or_default());
            toggle(&self.source_note, "warning", !bridge);
            toggle(&self.source_note, "dim-label", bridge);

            // Radio
            let bi = BANDS.iter().position(|b| *b == s.cfg.band).unwrap_or(0);
            set_combo(&self.band, bi as u32);
            self.band.set_subtitle(if s.ap_supports_5ghz() {
                ""
            } else {
                "This adapter only supports 2.4 GHz"
            });
            if self.band_shown.get() != Some(s.cfg.band) {
                let ch = channels(s.cfg.band);
                let refs: Vec<&str> = ch.iter().map(String::as_str).collect();
                self.channel.set_model(Some(&gtk::StringList::new(&refs)));
                self.band_shown.set(Some(s.cfg.band));
            }
            let ci = s
                .cfg
                .band
                .allowed_channels()
                .iter()
                .position(|c| *c == s.cfg.channel)
                .map_or(0, |i| i + 1);
            set_combo(&self.channel, ci as u32);
            self.channel
                .set_sensitive(editable && s.cfg.band != Band::Auto);
            self.band.set_sensitive(editable);
            if self.hidden.is_active() != s.cfg.hidden {
                self.hidden.set_active(s.cfg.hidden);
            }
            self.hidden.set_sensitive(editable);

            // Speed + defaults
            self.total_down.set(s.total_down_mbps);
            self.total_up.set(s.total_up_mbps);
            let ready = s.helper_ready();
            self.new_devices.set_sensitive(ready);
            self.needs_helper.set_visible(!ready);
            if self.approval.is_active() != s.defaults.require_approval {
                self.approval.set_active(s.defaults.require_approval);
            }
            self.defaults.set_sensitive(ready);
            self.defaults.set(&s.defaults.limits);

            // Not wired to the real system yet.
            let later = "Coming in a later version.";
            let speed_ok = s.helper_ready();
            self.total_down.row.set_sensitive(speed_ok);
            self.total_up.row.set_sensitive(speed_ok);
            self.speed.set_description(Some(if !speed_ok {
                "Needs the LinuxEasyHotspot helper."
            } else {
                "Limits for the whole hotspot. Set them to your real connection speed for priorities to work well."
            }));
            self.startup.set_sensitive(!s.live);
            self.startup.set_description(s.live.then_some(later));

            // Startup
            if self.autostart.is_active() != s.cfg.autostart {
                self.autostart.set_active(s.cfg.autostart);
            }
            if self.start_at_login.is_active() != s.cfg.start_at_login {
                self.start_at_login.set_active(s.cfg.start_at_login);
            }
            self.start_at_login.set_sensitive(s.cfg.autostart);
            if self.tray.is_active() != s.cfg.show_tray {
                self.tray.set_active(s.cfg.show_tray);
            }
            self.tray.set_visible(s.tray_available);
        });
    }
}

fn toggle(w: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}
