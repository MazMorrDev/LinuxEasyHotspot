//! Building blocks in the style of a preferences page: titled groups of
//! rows in a rounded card, with switches, drop-downs and text fields.

use std::rc::Rc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, ElementId, Entity, Focusable as _, Hsla,
    IntoElement, ParentElement, SharedString, Styled, Subscription, Window, div, prelude::*, px,
};
use leh_model::units::{self, Kind, UnitError};

use crate::i18n::tr;

/// Lucide icons (the full set; only the ones listed in `assets` are bundled).
pub use gpui_kit::assets::IconName as Lucide;

pub fn icon(name: Lucide) -> Icon {
    Icon::new(name)
}

/// Section heading with an optional description and something at the right.
pub fn group(
    title: Option<SharedString>,
    description: Option<SharedString>,
    suffix: Option<AnyElement>,
    rows: Vec<AnyElement>,
    cx: &App,
) -> AnyElement {
    let t = cx.theme();
    v_flex()
        .gap_2()
        .when(title.is_some() || suffix.is_some(), |d| {
            d.child(
                h_flex()
                    .justify_between()
                    .items_end()
                    .gap_2()
                    .children(title.map(|t| div().font_semibold().child(t)))
                    .children(suffix),
            )
        })
        .children(description.map(|d| div().text_sm().text_color(t.muted_foreground).child(d)))
        .when(!rows.is_empty(), |d| d.child(card(rows, cx)))
        .into_any_element()
}

/// Rows in a rounded box, separated by thin lines.
pub fn card(rows: Vec<AnyElement>, cx: &App) -> AnyElement {
    let t = cx.theme();
    let n = rows.len();
    v_flex()
        .rounded_lg()
        .border_1()
        .border_color(t.border)
        .bg(card_bg(cx))
        .overflow_hidden()
        .children(rows.into_iter().enumerate().map(|(i, r)| {
            div()
                .when(i + 1 < n, |d| d.border_b_1().border_color(t.border))
                .child(r)
        }))
        .into_any_element()
}

pub use super::theme::{card_bg, page_bg};

/// A row: title and subtitle at the left, `suffix` at the right.
pub fn row(
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    suffix: impl IntoElement,
    cx: &App,
) -> gpui_kit::Div {
    let t = cx.theme();
    h_flex()
        .w_full()
        .min_h(px(52.))
        .px_3()
        .py_2()
        .gap_3()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(div().child(title.into()))
                .children(
                    subtitle.map(|s| div().text_xs().text_color(t.muted_foreground).child(s)),
                ),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .gap_2()
                .items_center()
                .child(suffix),
        )
}

/// Text explaining something under or inside a group.
pub fn note(text: impl Into<SharedString>, color: Option<Hsla>, cx: &App) -> AnyElement {
    let t = cx.theme();
    div()
        .text_xs()
        .text_color(color.unwrap_or(t.muted_foreground))
        .child(text.into())
        .into_any_element()
}

/// A drop-down showing `labels[selected]`; picking an item calls `on`.
/// The closed button shows the short form of the label (before " · " or " (").
pub fn choice(
    id: impl Into<ElementId>,
    labels: Vec<SharedString>,
    selected: usize,
    disabled: bool,
    on: impl Fn(usize, &mut App) + 'static,
) -> AnyElement {
    let current = labels.get(selected).cloned().unwrap_or_default();
    let short: SharedString = current
        .split(" · ")
        .next()
        .and_then(|s| s.split(" (").next())
        .unwrap_or(&current)
        .to_owned()
        .into();
    let on = Rc::new(on);
    let labels = Rc::new(labels);
    Button::new(id)
        .label(short)
        .ghost()
        .small()
        .dropdown_caret(true)
        .disabled(disabled)
        .dropdown_menu(move |mut menu, _, _| {
            for (i, l) in labels.iter().enumerate() {
                let on = on.clone();
                menu = menu.item(
                    PopupMenuItem::new(l.clone())
                        .checked(i == selected)
                        .on_click(move |_, _, cx| on(i, cx)),
                );
            }
            menu
        })
        .into_any_element()
}

/// Where a typed limit goes when it's committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitSlot {
    Down,
    Up,
    Cap,
}

/// A typed limit: a number and a unit; empty means no limit. Values are in
/// base units (kbit/s for speeds, MB for data).
pub struct LimitInput {
    pub input: Entity<InputState>,
    pub kind: Kind,
    pub unit: usize,
    /// Last value shown or committed.
    pub value: Option<u32>,
    pub error: Option<&'static str>,
}

impl LimitInput {
    pub fn new<V: 'static>(kind: Kind, window: &mut Window, cx: &mut Context<V>) -> Self {
        // An empty field means no limit.
        let empty = match kind {
            Kind::Rate => tr("Unlimited"),
            Kind::Data => tr("No cap"),
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(empty));
        Self {
            input,
            kind,
            unit: match kind {
                Kind::Rate => units::RATE_DEFAULT,
                Kind::Data => units::DATA_DEFAULT,
            },
            value: None,
            error: None,
        }
    }

    /// Calls `on_commit` with the parsed value when Enter is pressed or the
    /// field loses focus.
    pub fn subscribe<V: 'static>(
        &self,
        window: &mut Window,
        cx: &mut Context<V>,
        pick: impl Fn(&mut V) -> Option<&mut LimitInput> + 'static,
        on_commit: impl Fn(&mut V, Option<u32>, &mut Window, &mut Context<V>) + 'static,
    ) -> Subscription {
        cx.subscribe_in(
            &self.input,
            window,
            move |this, _, ev: &InputEvent, window, cx| {
                if !matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    return;
                }
                let Some(li) = pick(this) else { return };
                if let Some(v) = li.commit(window, cx) {
                    on_commit(this, v, window, cx);
                }
                cx.notify();
            },
        )
    }

    /// Parses the text. On success normalizes it and returns the new value
    /// if it changed; on error remembers why and keeps the old value.
    pub fn commit<V: 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Option<Option<u32>> {
        let text = self.input.read(cx).value().to_string();
        match units::parse(self.kind, &text, self.unit) {
            Ok(v) => {
                self.error = None;
                let shown = match v {
                    Some((base, u)) => {
                        let (t, u) = units::display(self.kind, base, u);
                        self.unit = u;
                        t
                    }
                    None => String::new(),
                };
                if shown != text {
                    self.input
                        .update(cx, |s, cx| s.set_value(shown, window, cx));
                }
                let v = v.map(|(base, _)| base);
                let changed = self.value != v;
                self.value = v;
                changed.then_some(v)
            }
            Err(e) => {
                self.error = Some(match e {
                    UnitError::Invalid => tr("type a number"),
                    UnitError::TooSmall => tr("too small"),
                    UnitError::TooLarge => tr("too large"),
                });
                None
            }
        }
    }

    /// Shows `v` unless the user is typing in the field.
    pub fn sync<V: 'static>(&mut self, v: Option<u32>, window: &mut Window, cx: &mut Context<V>) {
        if self.value == v || self.input.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        self.value = v;
        self.error = None;
        let text = match v {
            Some(base) => {
                let (t, u) = units::display(self.kind, base, self.unit);
                self.unit = u;
                t
            }
            None => String::new(),
        };
        self.input.update(cx, |s, cx| s.set_value(text, window, cx));
    }

    /// The row: title (and the error, if any), field and unit.
    /// `on_unit` is called with the new unit index.
    pub fn render(
        &self,
        id: &'static str,
        title: &str,
        disabled: bool,
        on_unit: impl Fn(usize, &mut Window, &mut App) + 'static,
        cx: &App,
    ) -> AnyElement {
        let t = cx.theme();
        // The empty field's placeholder already says "Unlimited".
        let (title, color) = match self.error {
            Some(why) => (format!("{title} — {why}"), Some(t.danger)),
            None => (title.to_owned(), None),
        };
        let labels: Vec<SharedString> = self
            .kind
            .units()
            .iter()
            .map(|u| SharedString::from(u.label))
            .collect();
        let on_unit = Rc::new(on_unit);
        let unit_labels = Rc::new(labels);
        let unit = self.unit;
        let current = unit_labels.get(unit).cloned().unwrap_or_default();
        let unit_btn = Button::new(SharedString::from(format!("{id}-unit")))
            .label(current)
            .ghost()
            .small()
            .dropdown_caret(true)
            .disabled(disabled)
            .dropdown_menu(move |mut menu, _, _| {
                for (i, l) in unit_labels.iter().enumerate() {
                    let on = on_unit.clone();
                    menu = menu.item(
                        PopupMenuItem::new(l.clone())
                            .checked(i == unit)
                            .on_click(move |_, window, cx| on(i, window, cx)),
                    );
                }
                menu
            });
        h_flex()
            .w_full()
            .min_h(px(52.))
            .px_3()
            .py_2()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when_some(color, |d, c| d.text_color(c))
                    .when(disabled, |d| d.text_color(t.muted_foreground))
                    .child(title),
            )
            .child(
                div()
                    .w(px(104.))
                    .child(Input::new(&self.input).small().disabled(disabled)),
            )
            .child(unit_btn)
            .into_any_element()
    }
}
