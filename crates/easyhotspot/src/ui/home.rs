//! Home: the one-button page. Off: name, password, mode and Start. On: the
//! status, the QR code, the password, Stop and the session's activity.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, StyledExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement,
    StatefulInteractiveElement as _, Styled, div, prelude::*, px,
};
use leh_model::{Mode, password, ssid};

use super::widgets::{self, Lucide, icon};
use super::{MainView, clamp};
use crate::activity::EventKind;
use crate::i18n::{n_, tr, trf};
use crate::model::send;
use crate::msg::{Msg, Page};
use crate::state::{RunState, UiState};

const MODES: [Mode; 3] = [Mode::Router, Mode::Bridge, Mode::LocalOnly];
const MODE_NAMES: [&str; 3] = [n_("Router"), n_("Bridge"), n_("Local only")];

/// The session log never gets shorter than this (pixels).
const ACTIVITY_MIN_HEIGHT: f32 = 180.;

impl MainView {
    pub(super) fn home(&mut self, s: &UiState, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let on = matches!(s.run, RunState::On | RunState::Stopping);
        if on {
            self.home_on(s, wide, cx)
        } else {
            clamp(560., self.home_off(s, cx))
        }
    }

    fn home_off(&self, s: &UiState, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let starting = s.run == RunState::Starting;
        let editable = s.run == RunState::Off;

        let hero = div()
            .id("hero")
            .size(px(112.))
            .rounded_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .bg(if starting {
                t.primary
            } else {
                widgets::card_bg(cx)
            })
            .shadow_sm()
            .text_color(if starting {
                t.primary_foreground
            } else {
                t.foreground
            })
            .border_1()
            .border_color(t.border)
            .cursor_pointer()
            .hover(|d| d.opacity(0.85))
            .child(icon(Lucide::Wifi).size(px(40.)))
            .child(
                div()
                    .text_xs()
                    .font_semibold()
                    .child(if starting { "…" } else { tr("OFF") }),
            )
            .when(!starting, |d| d.on_click(self.on(|| Msg::Toggle)));

        let ssid_err = ssid::validate(&s.cfg.ssid)
            .err()
            .map(|e| crate::i18n::tr_dyn(&e.to_string()));
        let pw_err = password::validate(&s.cfg.password)
            .err()
            .map(|e| crate::i18n::tr_dyn(&e.to_string()));
        let field = |label: &str, input: AnyElement, err: Option<String>| {
            v_flex()
                .px_3()
                .py_2()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child(label.to_owned()),
                )
                .child(input)
                .children(err.map(|e| div().text_xs().text_color(t.danger).child(e)))
                .into_any_element()
        };
        let pw_buttons = h_flex()
            .gap_1()
            .child(
                Button::new("copy-pw")
                    .icon(icon(Lucide::Copy))
                    .ghost()
                    .small()
                    .tooltip(tr("Copy password"))
                    .on_click(self.on(|| Msg::CopyPassword)),
            )
            .child(
                Button::new("regen-pw")
                    .icon(icon(Lucide::RefreshCw))
                    .ghost()
                    .small()
                    .tooltip(tr("Generate a new password"))
                    .disabled(!editable)
                    .on_click(self.on(|| Msg::RegeneratePassword)),
            );
        let mode_idx = MODES.iter().position(|m| *m == s.cfg.mode).unwrap_or(0);
        let modes_available: Vec<Mode> = MODES
            .into_iter()
            .filter(|m| match m {
                Mode::Bridge => s.features.bridge,
                Mode::LocalOnly => s.local_only_supported,
                Mode::Router => true,
            })
            .collect();
        let rows = vec![
            field(
                tr("Name"),
                Input::new(&self.ssid)
                    .disabled(!editable)
                    .into_any_element(),
                ssid_err,
            ),
            field(
                tr("Password"),
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&self.password).mask_toggle().disabled(!editable)),
                    )
                    .child(pw_buttons)
                    .into_any_element(),
                pw_err,
            ),
        ];
        let mut rows = rows;
        // Only worth a choice when there's more than one mode.
        if modes_available.len() > 1 || s.cfg.mode != Mode::Router {
            let m = self.model.clone();
            rows.push(
                widgets::row(
                    tr("Mode"),
                    Some(mode_help(s.cfg.mode).into()),
                    widgets::choice(
                        "mode",
                        MODE_NAMES.iter().map(|n| tr(n).into()).collect(),
                        mode_idx,
                        !editable,
                        move |i, cx| send(&m, cx, Msg::Mode(MODES[i])),
                    ),
                    cx,
                )
                .into_any_element(),
            );
        }

        let start = Button::new("start")
            .label(if starting {
                tr("Starting…")
            } else {
                tr("Start hotspot")
            })
            .primary()
            .rounded(px(999.))
            .px_8()
            .loading(starting)
            .disabled(starting)
            .on_click(self.on(|| Msg::Toggle));

        v_flex()
            .gap_5()
            .items_center()
            .child(hero)
            .child(
                div()
                    .font_semibold()
                    .text_color(t.muted_foreground)
                    .child(if starting {
                        tr("Starting the hotspot…")
                    } else {
                        tr("Hotspot is off")
                    }),
            )
            .child(div().w_full().child(widgets::group(
                Some(tr("Network").into()),
                None,
                None,
                rows,
                cx,
            )))
            .child(start)
            .into_any_element()
    }

    fn home_on(&mut self, s: &UiState, wide: bool, cx: &mut Context<Self>) -> AnyElement {
        let payload = leh_model::qr::payload(&s.cfg.ssid, &s.cfg.password, s.cfg.hidden);
        let qr = self.qr.render(&payload, 232.);
        let t = cx.theme();

        let head = v_flex()
            .gap_0p5()
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        div()
                            .mt(px(7.))
                            .flex_shrink_0()
                            .size(px(10.))
                            .rounded_full()
                            .bg(t.success),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_semibold()
                            .text_lg()
                            .child(s.status_title()),
                    ),
            )
            .child(
                div()
                    .id("caption")
                    .text_sm()
                    .text_color(t.muted_foreground)
                    .cursor_pointer()
                    .hover(|d| d.underline())
                    .child(s.status_caption())
                    .on_click(self.on(|| Msg::Page(Page::Clients))),
            );

        let qr_box = v_flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .p_3()
                    .rounded_xl()
                    .bg(gpui_kit::white())
                    .border_1()
                    .border_color(t.border)
                    .child(qr),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(t.muted_foreground)
                    .child(trf("Scan to join “{name}”", &[("name", &s.cfg.ssid)])),
            );

        let shown = if self.pw_revealed {
            s.cfg.password.clone()
        } else {
            "•".repeat(s.cfg.password.chars().count())
        };
        let view = cx.entity().downgrade();
        let password = widgets::card(
            vec![
                widgets::row(
                    tr("Password"),
                    Some(shown.into()),
                    h_flex()
                        .gap_1()
                        .child(
                            Button::new("reveal")
                                .icon(icon(if self.pw_revealed {
                                    Lucide::EyeOff
                                } else {
                                    Lucide::Eye
                                }))
                                .ghost()
                                .small()
                                .tooltip(if self.pw_revealed {
                                    tr("Hide password")
                                } else {
                                    tr("Show password")
                                })
                                .on_click(move |_, _, cx| {
                                    let _ = view.update(cx, |v, cx| {
                                        v.pw_revealed = !v.pw_revealed;
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            Button::new("copy-pw-on")
                                .icon(icon(Lucide::Copy))
                                .ghost()
                                .small()
                                .tooltip(tr("Copy password"))
                                .on_click(self.on(|| Msg::CopyPassword)),
                        ),
                    cx,
                )
                .into_any_element(),
            ],
            cx,
        );
        let stop = Button::new("stop")
            .label(tr("Stop hotspot"))
            .danger()
            .rounded(px(999.))
            .px_8()
            .loading(s.run == RunState::Stopping)
            .disabled(s.run != RunState::On)
            .on_click(self.on(|| Msg::Toggle));
        let details = v_flex()
            .gap_5()
            .w_full()
            .child(password)
            .child(h_flex().justify_center().child(stop));

        let top = if wide {
            h_flex()
                .gap_8()
                .items_start()
                .child(v_flex().flex_1().gap_6().child(head).child(details))
                .child(qr_box)
                .into_any_element()
        } else {
            v_flex()
                .gap_6()
                .child(head)
                .child(qr_box)
                .child(details)
                .into_any_element()
        };

        clamp(
            if wide { 760. } else { 560. },
            v_flex().gap_6().child(top).child(self.activity(s, cx)),
        )
    }

    /// Session activity, newest first.
    fn activity(&self, s: &UiState, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let rows: Vec<AnyElement> = if s.events.is_empty() {
            vec![
                div()
                    .px_3()
                    .py_3()
                    .text_color(t.muted_foreground)
                    .child(tr("Nothing has happened yet"))
                    .into_any_element(),
            ]
        } else {
            s.events
                .iter()
                .rev()
                .map(|e| {
                    let (ico, color) = match e.kind {
                        EventKind::Started => (Lucide::Play, Some(t.success)),
                        EventKind::Stopped => (Lucide::Square, None),
                        EventKind::Joined => (Lucide::UserPlus, None),
                        EventKind::Left => (Lucide::UserMinus, None),
                        EventKind::Waiting => (Lucide::AlarmClock, Some(t.warning)),
                        EventKind::Allowed => (Lucide::Check, Some(t.success)),
                        EventKind::Blocked => (Lucide::Ban, Some(t.danger)),
                        EventKind::Warning => (Lucide::TriangleAlert, Some(t.warning)),
                        EventKind::Error => (Lucide::CircleX, Some(t.danger)),
                        EventKind::Info => (Lucide::Info, None),
                    };
                    h_flex()
                        .px_3()
                        .py_2()
                        .gap_3()
                        .child(
                            icon(ico)
                                .small()
                                .text_color(color.unwrap_or(t.muted_foreground)),
                        )
                        .child(div().flex_1().min_w_0().text_sm().child(e.text.clone()))
                        .child(
                            div()
                                .text_xs()
                                .text_color(t.muted_foreground)
                                .child(e.time.clone()),
                        )
                        .into_any_element()
                })
                .collect()
        };
        v_flex()
            .gap_2()
            .child(div().font_semibold().child(tr("Session activity")))
            .child(
                div()
                    .id("activity")
                    .min_h(px(ACTIVITY_MIN_HEIGHT))
                    .max_h(px(320.))
                    .overflow_y_scroll()
                    .child(widgets::card(rows, cx)),
            )
            .into_any_element()
    }
}

fn mode_help(m: Mode) -> &'static str {
    match m {
        Mode::Router => tr("Devices share this computer's internet"),
        Mode::Bridge => tr("Devices join your wired network directly"),
        Mode::LocalOnly => tr("Devices can reach each other, but not the internet"),
    }
}
