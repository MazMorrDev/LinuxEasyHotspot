//! Small reusable widget groups.

use std::cell::Cell;
use std::rc::Rc;

use leh_model::policy::{CapAction, Limits, Priority};
use relm4::adw::prelude::*;
use relm4::{adw, gtk};

/// A guard to suppress change callbacks while the UI is being updated from state.
#[derive(Clone, Default)]
pub struct Guard(Rc<Cell<bool>>);

impl Guard {
    pub fn hold<R>(&self, f: impl FnOnce() -> R) -> R {
        let prev = self.0.replace(true);
        let r = f();
        self.0.set(prev);
        r
    }

    pub fn active(&self) -> bool {
        self.0.get()
    }
}

pub fn combo(title: &str, items: &[&str]) -> adw::ComboRow {
    let row = adw::ComboRow::builder().title(title).build();
    row.set_model(Some(&gtk::StringList::new(items)));
    row
}

/// Closed combo shows a short form of each item (text before " · " or " ("),
/// the popup list shows the full text. Keeps rows from truncating on narrow windows.
pub fn short_display(row: &adw::ComboRow) {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_child(Some(&gtk::Label::builder().xalign(1.0).build()));
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
            let full = text.string();
            let short = full
                .split(" · ")
                .next()
                .and_then(|s| s.split(" (").next())
                .unwrap_or(&full);
            label.set_label(short);
        }
    });
    row.set_factory(Some(&factory));
    let list = gtk::SignalListItemFactory::new();
    list.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            item.set_child(Some(&gtk::Label::builder().xalign(0.0).build()));
        }
    });
    list.connect_bind(|_, item| {
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        if let (Some(label), Some(text)) = (
            item.child().and_downcast::<gtk::Label>(),
            item.item().and_downcast::<gtk::StringObject>(),
        ) {
            label.set_label(&text.string());
        }
    });
    row.set_list_factory(Some(&list));
}

pub fn set_combo(row: &adw::ComboRow, idx: u32) {
    if row.selected() != idx {
        row.set_selected(idx);
    }
}

pub fn set_text(row: &impl IsA<gtk::Editable>, text: &str) {
    if row.text() != text {
        row.set_text(text);
    }
}

pub fn switch_row(title: &str, subtitle: &str) -> adw::SwitchRow {
    let row = adw::SwitchRow::builder().title(title).build();
    if !subtitle.is_empty() {
        row.set_subtitle(subtitle);
    }
    row
}

pub fn group(title: &str) -> adw::PreferencesGroup {
    adw::PreferencesGroup::builder().title(title).build()
}

/// A numeric value that can be switched off ("unlimited"/"no cap").
/// Stored values are `shown * scale` (e.g. Mbps shown, kbit/s stored).
#[derive(Clone)]
pub struct OptRow {
    pub row: adw::SpinRow,
    switch: gtk::Switch,
    scale: f64,
    off_label: &'static str,
    unit: &'static str,
    guard: Guard,
}

impl OptRow {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        title: &str,
        unit: &'static str,
        off_label: &'static str,
        (min, max, step): (f64, f64, f64),
        digits: u32,
        scale: f64,
        on_change: impl Fn(Option<u32>) + 'static,
    ) -> Self {
        let row = adw::SpinRow::with_range(min, max, step);
        row.set_title(title);
        row.set_digits(digits);
        let switch = gtk::Switch::builder().valign(gtk::Align::Center).build();
        switch.update_property(&[gtk::accessible::Property::Label(&format!("Enable {title}"))]);
        row.add_suffix(&switch);
        let me = Self {
            row,
            switch,
            scale,
            off_label,
            unit,
            guard: Guard::default(),
        };
        let on_change = Rc::new(on_change);
        let emit = {
            let me = me.clone();
            let on_change = on_change.clone();
            move || {
                me.refresh_subtitle();
                if !me.guard.active() {
                    on_change(me.value());
                }
            }
        };
        let e = emit.clone();
        me.row.connect_value_notify(move |_| e());
        me.switch.connect_active_notify(move |_| emit());
        me.refresh_subtitle();
        me
    }

    fn value(&self) -> Option<u32> {
        self.switch
            .is_active()
            .then(|| (self.row.value() * self.scale).round() as u32)
    }

    fn refresh_subtitle(&self) {
        if self.switch.is_active() {
            self.row.set_subtitle(self.unit);
        } else {
            self.row.set_subtitle(self.off_label);
        }
    }

    pub fn set(&self, v: Option<u32>) {
        if self.value() == v {
            return;
        }
        self.guard.hold(|| {
            self.switch.set_active(v.is_some());
            if let Some(v) = v {
                self.row.set_value(f64::from(v) / self.scale);
            }
        });
        self.refresh_subtitle();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitField {
    Down(Option<u32>),
    Up(Option<u32>),
    Priority(Priority),
    Cap(Option<u32>),
    CapAction(CapAction),
}

impl LimitField {
    pub fn apply(self, l: &mut Limits) {
        match self {
            Self::Down(v) => l.down_kbps = v,
            Self::Up(v) => l.up_kbps = v,
            Self::Priority(p) => l.priority = p,
            Self::Cap(v) => l.cap_mb = v,
            Self::CapAction(a) => l.cap_action = a,
        }
    }
}

const PRIORITIES: [Priority; 3] = [Priority::High, Priority::Normal, Priority::Low];
const CAP_ACTIONS: [CapAction; 3] = [
    CapAction::Block,
    CapAction::Throttle { kbps: 256 },
    CapAction::Throttle { kbps: 1_000 },
];

/// Rows for download, upload, priority, data cap and cap action.
pub struct LimitsEditor {
    down: OptRow,
    up: OptRow,
    priority: adw::ComboRow,
    cap: OptRow,
    cap_action: adw::ComboRow,
    guard: Guard,
    enabled: Cell<bool>,
}

impl LimitsEditor {
    pub fn new(on: impl Fn(LimitField) + Clone + 'static) -> Self {
        let mbps = (0.5, 1000.0, 0.5);
        let o = on.clone();
        let down = OptRow::new(
            "Download limit",
            "Mbps",
            "Unlimited",
            mbps,
            1,
            1000.0,
            move |v| o(LimitField::Down(v)),
        );
        let o = on.clone();
        let up = OptRow::new(
            "Upload limit",
            "Mbps",
            "Unlimited",
            mbps,
            1,
            1000.0,
            move |v| o(LimitField::Up(v)),
        );
        let o = on.clone();
        let cap = OptRow::new(
            "Data cap",
            "MB per session",
            "No cap",
            (10.0, 100_000.0, 50.0),
            0,
            1.0,
            move |v| o(LimitField::Cap(v)),
        );
        let priority = combo("Priority", &["High", "Normal", "Low"]);
        priority.set_subtitle("Who gets bandwidth first when the link is busy");
        let cap_action = combo(
            "When the cap is reached",
            &["Block the device", "Slow to 256 kbps", "Slow to 1 Mbps"],
        );
        let guard = Guard::default();
        let (g, o) = (guard.clone(), on.clone());
        priority.connect_selected_notify(move |r| {
            if !g.active()
                && let Some(p) = PRIORITIES.get(r.selected() as usize)
            {
                o(LimitField::Priority(*p));
            }
        });
        let g = guard.clone();
        cap_action.connect_selected_notify(move |r| {
            if !g.active()
                && let Some(a) = CAP_ACTIONS.get(r.selected() as usize)
            {
                on(LimitField::CapAction(*a));
            }
        });
        Self {
            down,
            up,
            priority,
            cap,
            cap_action,
            guard,
            enabled: Cell::new(true),
        }
    }

    pub fn add_to(&self, group: &adw::PreferencesGroup) {
        group.add(&self.down.row);
        group.add(&self.up.row);
        group.add(&self.priority);
        group.add(&self.cap.row);
        group.add(&self.cap_action);
    }

    pub fn set(&self, l: &Limits) {
        self.down.set(l.down_kbps);
        self.up.set(l.up_kbps);
        self.cap.set(l.cap_mb);
        self.guard.hold(|| {
            let p = PRIORITIES
                .iter()
                .position(|p| *p == l.priority)
                .unwrap_or(1);
            set_combo(&self.priority, p as u32);
            let a = CAP_ACTIONS
                .iter()
                .position(|a| *a == l.cap_action)
                .unwrap_or(0);
            set_combo(&self.cap_action, a as u32);
        });
        self.cap_action
            .set_sensitive(self.enabled.get() && l.cap_mb.is_some());
    }

    pub fn set_sensitive(&self, s: bool) {
        self.enabled.set(s);
        for w in [
            self.down.row.upcast_ref::<gtk::Widget>(),
            self.up.row.upcast_ref(),
            self.priority.upcast_ref(),
            self.cap.row.upcast_ref(),
        ] {
            w.set_sensitive(s);
        }
        if !s {
            self.cap_action.set_sensitive(false);
        }
    }
}
