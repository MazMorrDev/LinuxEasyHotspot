//! Small reusable widget groups.

use crate::i18n::{tr, trf};
use std::cell::Cell;
use std::rc::Rc;

use leh_model::policy::{CapAction, Limits, Priority};
use leh_model::units::{self, Kind, UnitError};
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

/// A typed limit: a number plus a unit drop-down; empty means no limit.
/// Values are reported in base units (kbit/s for speeds, MB for data).
#[derive(Clone)]
pub struct LimitRow {
    pub row: adw::EntryRow,
    unit: gtk::DropDown,
    kind: Kind,
    title: String,
    off_label: &'static str,
    value: Rc<Cell<Option<u32>>>,
    guard: Guard,
}

impl LimitRow {
    pub fn new(
        title: &str,
        kind: Kind,
        off_label: &'static str,
        on_change: impl Fn(Option<u32>) + 'static,
    ) -> Self {
        let row = adw::EntryRow::builder()
            .title(title)
            .show_apply_button(true)
            .input_purpose(gtk::InputPurpose::Number)
            .build();
        let labels: Vec<&str> = kind.units().iter().map(|u| u.label).collect();
        let unit = gtk::DropDown::from_strings(&labels);
        unit.set_valign(gtk::Align::Center);
        unit.add_css_class("flat");
        unit.set_selected(match kind {
            Kind::Rate => units::RATE_DEFAULT,
            Kind::Data => units::DATA_DEFAULT,
        } as u32);
        unit.update_property(&[gtk::accessible::Property::Label(&trf(
            "{name} unit",
            &[("name", title)],
        ))]);
        row.add_suffix(&unit);
        row.set_tooltip_text(Some(match kind {
            Kind::Rate => tr("Type a number and pick the unit, or type it: 5, 2.5 Mbps, 800 kbps, 1 MB/s. Leave empty for no limit."),
            Kind::Data => tr("Type a number and pick the unit, or type it: 500, 1.5 GB. Leave empty for no cap."),
        }));
        let me = Self {
            row,
            unit,
            kind,
            title: title.to_owned(),
            off_label,
            value: Rc::new(Cell::new(None)),
            guard: Guard::default(),
        };
        let on_change = Rc::new(on_change);
        let commit = {
            let me = me.clone();
            move || {
                if me.guard.active() {
                    return;
                }
                if let Some(v) = me.commit() {
                    on_change(v);
                }
            }
        };
        let c = commit.clone();
        me.row.connect_apply(move |_| c());
        let c = commit.clone();
        me.row.connect_entry_activated(move |_| c());
        let c = commit.clone();
        me.unit.connect_selected_notify(move |_| c());
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(move |_| commit());
        me.row.add_controller(focus);
        me.refresh_title();
        me
    }

    /// Parses the text; on success normalizes it and returns the new value
    /// if it changed. On error marks the row and keeps the old value.
    fn commit(&self) -> Option<Option<u32>> {
        let text = self.row.text();
        match units::parse(self.kind, &text, self.unit.selected() as usize) {
            Ok(v) => {
                self.row.remove_css_class("error");
                let v = v.map(|(base, u)| {
                    self.show(base, u);
                    base
                });
                if v.is_none() && !text.is_empty() {
                    self.guard.hold(|| self.row.set_text(""));
                }
                let changed = self.value.replace(v) != v;
                self.refresh_title();
                changed.then_some(v)
            }
            Err(e) => {
                self.row.add_css_class("error");
                let why = match e {
                    UnitError::Invalid => tr("type a number"),
                    UnitError::TooSmall => tr("too small"),
                    UnitError::TooLarge => tr("too large"),
                };
                self.row.set_title(&format!("{} — {why}", self.title));
                None
            }
        }
    }

    fn show(&self, base: u32, preferred: usize) {
        let (text, u) = units::display(self.kind, base, preferred);
        self.guard.hold(|| {
            if self.row.text() != text {
                self.row.set_text(&text);
            }
            self.unit.set_selected(u as u32);
        });
    }

    fn refresh_title(&self) {
        if self.value.get().is_some() {
            self.row.set_title(&self.title);
        } else {
            self.row
                .set_title(&format!("{} · {}", self.title, self.off_label));
        }
    }

    pub fn set(&self, v: Option<u32>) {
        // Don't overwrite what the user is typing.
        if self.value.get() == v || self.row.has_focus() || self.row.focus_child().is_some() {
            return;
        }
        self.value.set(v);
        self.row.remove_css_class("error");
        match v {
            Some(base) => self.show(base, self.unit.selected() as usize),
            None => self.guard.hold(|| self.row.set_text("")),
        }
        self.refresh_title();
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
    down: LimitRow,
    up: LimitRow,
    priority: adw::ComboRow,
    cap: LimitRow,
    cap_action: adw::ComboRow,
    guard: Guard,
    enabled: Cell<bool>,
}

impl LimitsEditor {
    pub fn new(on: impl Fn(LimitField) + Clone + 'static) -> Self {
        let o = on.clone();
        let down = LimitRow::new(
            tr("Download limit"),
            Kind::Rate,
            tr("Unlimited"),
            move |v| o(LimitField::Down(v)),
        );
        let o = on.clone();
        let up = LimitRow::new(tr("Upload limit"), Kind::Rate, tr("Unlimited"), move |v| {
            o(LimitField::Up(v))
        });
        let o = on.clone();
        let cap = LimitRow::new(
            tr("Data cap per session"),
            Kind::Data,
            tr("No cap"),
            move |v| o(LimitField::Cap(v)),
        );
        let priority = combo(tr("Priority"), &[tr("High"), tr("Normal"), tr("Low")]);
        priority.set_subtitle(tr("Who gets bandwidth first when the link is busy"));
        let cap_action = combo(
            tr("When the cap is reached"),
            &[
                tr("Block the device"),
                tr("Slow to 256 kbps"),
                tr("Slow to 1 Mbps"),
            ],
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
