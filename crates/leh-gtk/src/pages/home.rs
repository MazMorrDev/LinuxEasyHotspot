//! Home: the one-button page.

use crate::i18n::{n_, tr, trf};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use leh_model::{Mode, password, ssid};
use relm4::adw::prelude::*;
use relm4::{Sender, adw, gtk};

use crate::app::Msg;
use crate::qr;
use crate::state::{RunState, UiState};
use crate::widgets::{Guard, combo, set_combo, set_text};

const MODES: [Mode; 3] = [Mode::Router, Mode::Bridge, Mode::LocalOnly];
/// Shown as hover tips instead of long item labels.
const MODE_HELP: [&str; 3] = [
    n_("Router: devices share this computer's internet connection"),
    n_("Bridge: devices join your wired network and get addresses from your router"),
    n_("Local only: devices can reach each other and this computer, but not the internet"),
];

pub struct HomePage {
    pub root: gtk::ScrolledWindow,
    stack: gtk::Stack,
    // Off / starting
    hero: gtk::Button,
    hero_icon: gtk::Image,
    hero_label: gtk::Label,
    off_status: gtk::Label,
    ssid: adw::EntryRow,
    password: adw::PasswordEntryRow,
    mode: adw::ComboRow,
    start: gtk::Button,
    start_label: gtk::Label,
    start_spinner: gtk::Spinner,
    // On
    on_grid: gtk::Grid,
    on_qr: gtk::Box,
    on_details: gtk::Box,
    on_title: gtk::Label,
    on_caption: gtk::Label,
    qr: gtk::Picture,
    scan_label: gtk::Label,
    on_password: adw::ActionRow,
    pw_reveal: gtk::ToggleButton,
    pw_text: Rc<RefCell<String>>,
    last_run: Cell<RunState>,
    stop: gtk::Button,
    qr_payload: RefCell<String>,
    guard: Guard,
}

fn pill(label: &str, class: &str) -> (gtk::Button, gtk::Label, gtk::Spinner) {
    let l = gtk::Label::new(Some(label));
    let s = gtk::Spinner::new();
    s.set_visible(false);
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    b.set_halign(gtk::Align::Center);
    b.append(&s);
    b.append(&l);
    let btn = gtk::Button::builder()
        .child(&b)
        .halign(gtk::Align::Center)
        .css_classes(["pill", class])
        .build();
    (btn, l, s)
}

impl HomePage {
    pub fn new(tx: Sender<Msg>) -> Self {
        let guard = Guard::default();

        // ---- Off layout
        let hero_icon = gtk::Image::from_icon_name(crate::ICON_SYMBOLIC);
        hero_icon.set_pixel_size(40);
        let hero_label = gtk::Label::new(Some(tr("OFF")));
        hero_label.add_css_class("caption-heading");
        let hero_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        hero_box.set_valign(gtk::Align::Center);
        hero_box.append(&hero_icon);
        hero_box.append(&hero_label);
        let hero = gtk::Button::builder()
            .child(&hero_box)
            .halign(gtk::Align::Center)
            .css_classes(["circular", "hero-toggle"])
            .tooltip_text(tr("Start hotspot (Ctrl+Space)"))
            .build();
        hero.update_property(&[gtk::accessible::Property::Label(tr(
            "Turn hotspot on or off",
        ))]);
        let t = tx.clone();
        hero.connect_clicked(move |_| t.emit(Msg::Toggle));

        let off_status = gtk::Label::new(Some(tr("Hotspot is off")));
        off_status.add_css_class("heading");
        off_status.add_css_class("dim-label");

        let ssid = adw::EntryRow::builder().title(tr("Name")).build();
        let password = adw::PasswordEntryRow::builder()
            .title(tr("Password"))
            .build();
        let copy = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text(tr("Copy password"))
            .css_classes(["flat"])
            .build();
        copy.update_property(&[gtk::accessible::Property::Label(tr("Copy password"))]);
        let regen = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text(tr("Generate a new password"))
            .css_classes(["flat"])
            .build();
        regen.update_property(&[gtk::accessible::Property::Label(tr(
            "Generate a new password",
        ))]);
        password.add_suffix(&copy);
        password.add_suffix(&regen);
        let mode = combo(tr("Mode"), &[tr("Router"), tr("Bridge"), tr("Local only")]);
        // Items in the popup get the explanation as a tooltip.
        let factory = gtk::SignalListItemFactory::new();
        factory.connect_setup(|_, item| {
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                item.set_child(Some(&gtk::Label::builder().xalign(0.0).build()));
            }
        });
        factory.connect_bind(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            if let (Some(label), Some(text)) = (
                item.child().and_downcast::<gtk::Label>(),
                item.item().and_downcast::<gtk::StringObject>(),
            ) {
                label.set_label(&text.string());
                label.set_tooltip_text(MODE_HELP.get(item.position() as usize).map(|s| tr(s)));
            }
        });
        mode.set_list_factory(Some(&factory));

        let (g, t) = (guard.clone(), tx.clone());
        ssid.connect_changed(move |e| {
            if !g.active() {
                t.emit(Msg::Ssid(e.text().into()));
            }
        });
        let (g, t) = (guard.clone(), tx.clone());
        password.connect_changed(move |e| {
            if !g.active() {
                t.emit(Msg::Password(e.text().into()));
            }
        });
        let t = tx.clone();
        copy.connect_clicked(move |_| t.emit(Msg::CopyPassword));
        let t = tx.clone();
        regen.connect_clicked(move |_| t.emit(Msg::RegeneratePassword));
        let (g, t) = (guard.clone(), tx.clone());
        mode.connect_selected_notify(move |r| {
            if !g.active()
                && let Some(m) = MODES.get(r.selected() as usize)
            {
                t.emit(Msg::Mode(*m));
            }
        });

        let net = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        net.append(&ssid);
        net.append(&password);
        net.append(&mode);
        let net_title = gtk::Label::builder()
            .label(tr("Network"))
            .halign(gtk::Align::Start)
            .css_classes(["heading"])
            .build();

        let (start, start_label, start_spinner) = pill(tr("Start hotspot"), "suggested-action");
        let t = tx.clone();
        start.connect_clicked(move |_| t.emit(Msg::Toggle));

        let off = gtk::Box::new(gtk::Orientation::Vertical, 18);
        off.append(&hero);
        off.append(&off_status);
        let net_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        net_box.append(&net_title);
        net_box.append(&net);
        off.append(&net_box);
        off.append(&start);

        // ---- On layout
        let on_title = gtk::Label::builder()
            .halign(gtk::Align::Start)
            .wrap(true)
            .build();
        on_title.add_css_class("heading");
        let dot = gtk::Label::new(Some("●"));
        dot.add_css_class("success");
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        title_row.append(&dot);
        title_row.append(&on_title);
        let on_caption = gtk::Label::builder().halign(gtk::Align::Start).build();
        on_caption.add_css_class("caption");
        on_caption.add_css_class("dim-label");
        let caption_btn = gtk::Button::builder()
            .child(&on_caption)
            .halign(gtk::Align::Start)
            .css_classes(["flat", "caption-link"])
            .tooltip_text(tr("Show connected devices"))
            .build();
        let t = tx.clone();
        caption_btn.connect_clicked(move |_| t.emit(Msg::GoTo("clients")));
        let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
        head.append(&title_row);
        head.append(&caption_btn);

        let qr = gtk::Picture::builder()
            .can_shrink(false)
            .halign(gtk::Align::Center)
            .build();
        qr.update_property(&[gtk::accessible::Property::Label(tr(
            "QR code to join the hotspot",
        ))]);
        let qr_frame = gtk::Box::builder()
            .halign(gtk::Align::Center)
            .css_classes(["qr-frame"])
            .build();
        qr_frame.append(&qr);
        let scan_label = gtk::Label::new(None);
        scan_label.add_css_class("dim-label");
        let qr_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        qr_box.set_valign(gtk::Align::Center);
        qr_box.append(&qr_frame);
        qr_box.append(&scan_label);

        let on_password = adw::ActionRow::builder().title(tr("Password")).build();
        on_password.add_css_class("property");
        on_password.set_subtitle_selectable(true);
        let pw_reveal = gtk::ToggleButton::builder()
            .icon_name("view-reveal-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text(tr("Show password"))
            .css_classes(["flat"])
            .build();
        pw_reveal.update_property(&[gtk::accessible::Property::Label(tr("Show password"))]);
        on_password.add_suffix(&pw_reveal);
        let pw_text: Rc<RefCell<String>> = Rc::default();
        let (row, text) = (on_password.clone(), pw_text.clone());
        pw_reveal.connect_toggled(move |b| render_password(&row, b, &text.borrow()));
        let copy2 = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text(tr("Copy password"))
            .css_classes(["flat"])
            .build();
        let t = tx.clone();
        copy2.connect_clicked(move |_| t.emit(Msg::CopyPassword));
        on_password.add_suffix(&copy2);
        let pw_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list", "password-row"])
            .build();
        pw_list.append(&on_password);

        let (stop, _, _) = pill(tr("Stop hotspot"), "destructive-action");
        let t = tx.clone();
        stop.connect_clicked(move |_| t.emit(Msg::Toggle));

        let details = gtk::Box::new(gtk::Orientation::Vertical, 18);
        details.set_valign(gtk::Align::Start);
        details.set_hexpand(true);
        details.append(&pw_list);
        details.append(&stop);

        // Narrow: status, QR, password, Stop (one column).
        // Wide (see `wide_setters`): status + password + Stop left, QR right.
        let on_grid = gtk::Grid::builder()
            .row_spacing(24)
            .column_spacing(32)
            .valign(gtk::Align::Center)
            .build();
        head.set_hexpand(true);
        on_grid.attach(&head, 0, 0, 1, 1);
        on_grid.attach(&qr_box, 0, 1, 1, 1);
        on_grid.attach(&details, 0, 2, 1, 1);
        let on = gtk::Box::new(gtk::Orientation::Vertical, 0);
        on.append(&on_grid);

        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .vhomogeneous(false)
            .build();
        stack.add_named(&off, Some("off"));
        stack.add_named(&on, Some("on"));

        let clamp = adw::Clamp::builder()
            .maximum_size(560)
            .child(&stack)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(16)
            .margin_end(16)
            .build();
        let root = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .child(&clamp)
            .build();

        Self {
            root,
            stack,
            hero,
            hero_icon,
            hero_label,
            off_status,
            ssid,
            password,
            mode,
            start,
            start_label,
            start_spinner,
            on_grid,
            on_qr: qr_box,
            on_details: details,
            on_title,
            on_caption,
            qr,
            scan_label,
            on_password,
            pw_reveal,
            pw_text,
            last_run: Cell::new(RunState::Off),
            stop,
            qr_payload: RefCell::new(String::new()),
            guard,
        }
    }

    /// Grid positions for the wide layout, applied by an `AdwBreakpoint`.
    pub fn wide_setters(&self, bp: &adw::Breakpoint) {
        let lm = self
            .on_grid
            .layout_manager()
            .expect("grid has a layout manager");
        let qr = lm.layout_child(&self.on_qr);
        let details = lm.layout_child(&self.on_details);
        bp.add_setter(&qr, "column", Some(&1.to_value()));
        bp.add_setter(&qr, "row", Some(&0.to_value()));
        bp.add_setter(&qr, "row-span", Some(&3.to_value()));
        bp.add_setter(&details, "row", Some(&1.to_value()));
    }

    pub fn update(&self, s: &UiState) {
        let showing_on = matches!(s.run, RunState::On | RunState::Stopping);
        self.stack
            .set_visible_child_name(if showing_on { "on" } else { "off" });

        self.guard.hold(|| {
            set_text(&self.ssid, &s.cfg.ssid);
            set_text(&self.password, &s.cfg.password);
            let idx = MODES.iter().position(|m| *m == s.cfg.mode).unwrap_or(0);
            set_combo(&self.mode, idx as u32);
            self.mode
                .set_tooltip_text(MODE_HELP.get(idx).map(|s| tr(s)));
        });
        toggle_class(&self.ssid, "error", ssid::validate(&s.cfg.ssid).is_err());
        toggle_class(
            &self.password,
            "error",
            password::validate(&s.cfg.password).is_err(),
        );
        let editable = s.run == RunState::Off;
        for w in [
            self.ssid.upcast_ref::<gtk::Widget>(),
            self.password.upcast_ref(),
            self.mode.upcast_ref(),
        ] {
            w.set_sensitive(editable);
        }

        let starting = s.run == RunState::Starting;
        self.start.set_sensitive(!starting);
        self.hero.set_sensitive(!starting);
        self.start_spinner.set_visible(starting);
        self.start_spinner.set_spinning(starting);
        self.start_label.set_label(if starting {
            tr("Starting…")
        } else {
            tr("Start hotspot")
        });
        self.hero_label
            .set_label(if starting { "…" } else { tr("OFF") });
        self.off_status.set_label(if starting {
            tr("Starting the hotspot…")
        } else {
            tr("Hotspot is off")
        });
        toggle_class(&self.hero, "suggested-action", starting);
        self.hero_icon.set_icon_name(Some(crate::ICON_SYMBOLIC));

        if showing_on {
            self.on_title.set_label(&s.status_title());
            self.on_caption.set_label(&s.status_caption());
            // Hidden again every time the hotspot starts.
            if self.last_run.get() != RunState::On && s.run == RunState::On {
                self.pw_reveal.set_active(false);
            }
            *self.pw_text.borrow_mut() = s.cfg.password.clone();
            render_password(&self.on_password, &self.pw_reveal, &s.cfg.password);
            self.scan_label
                .set_label(&trf("Scan to join “{name}”", &[("name", &s.cfg.ssid)]));
            self.stop.set_sensitive(s.run == RunState::On);
            let payload = leh_model::qr::payload(&s.cfg.ssid, &s.cfg.password, s.cfg.hidden);
            if *self.qr_payload.borrow() != payload {
                self.qr.set_paintable(qr::texture(&payload, 240).as_ref());
                *self.qr_payload.borrow_mut() = payload;
            }
        }
        self.last_run.set(s.run);
    }
}

pub fn toggle_class(w: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        w.add_css_class(class);
    } else {
        w.remove_css_class(class);
    }
}

fn render_password(row: &adw::ActionRow, reveal: &gtk::ToggleButton, password: &str) {
    let shown = reveal.is_active();
    reveal.set_icon_name(if shown {
        "view-conceal-symbolic"
    } else {
        "view-reveal-symbolic"
    });
    let label = if shown {
        tr("Hide password")
    } else {
        tr("Show password")
    };
    reveal.set_tooltip_text(Some(label));
    reveal.update_property(&[gtk::accessible::Property::Label(label)]);
    if shown {
        row.set_subtitle(password);
    } else {
        row.set_subtitle(&"•".repeat(password.chars().count()));
    }
    row.add_css_class("monospace");
}
