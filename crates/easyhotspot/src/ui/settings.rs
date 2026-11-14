//! Settings: mode, adapters, radio, speed, new-device defaults, startup,
//! language. Parts the system can't do are hidden or explained.

use gpui_kit::component::radio::Radio;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Context, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::*,
};
use leh_model::policy::{CapAction, Limits, Priority};
use leh_model::{Band, Language, Mode};

use super::widgets::{self, LimitInput, Lucide, choice, icon, row};
use super::{MainView, clamp};
use crate::i18n::{tr, trf};
use crate::model::{Model, send};
use crate::msg::{LimitField, Msg};
use crate::state::{HelperStatus, RunState, UiState};

const BANDS: [Band; 3] = [Band::Auto, Band::Ghz2_4, Band::Ghz5];
const PRIORITIES: [Priority; 3] = [Priority::High, Priority::Normal, Priority::Low];
const CAP_ACTIONS: [CapAction; 3] = [
    CapAction::Block,
    CapAction::Throttle { kbps: 256 },
    CapAction::Throttle { kbps: 1_000 },
];

pub(super) fn switch(
    id: &'static str,
    on: bool,
    disabled: bool,
    model: &gpui_kit::Entity<Model>,
    msg: fn(bool) -> Msg,
) -> AnyElement {
    let m = model.clone();
    Switch::new(id)
        .checked(on)
        .disabled(disabled)
        .on_click(move |v, _, cx| send(&m, cx, msg(*v)))
        .into_any_element()
}

impl MainView {
    pub(super) fn settings(&mut self, s: &UiState, cx: &mut Context<Self>) -> AnyElement {
        let mut groups = vec![self.modes(s, cx), self.adapters(s, cx)];
        groups.extend(self.radio(s, cx));
        if s.helper == HelperStatus::Unsupported {
            groups.push(widgets::group(
                Some(tr("Per-device control").into()),
                Some(
                    tr("Speed limits, approval of new devices and data caps aren't available on Windows yet. Devices can still be renamed, and every device that connects is listed in Clients.")
                        .into(),
                ),
                None,
                Vec::new(),
                cx,
            ));
        } else {
            groups.push(self.speed(s, cx));
            groups.push(self.new_devices(s, cx));
        }
        groups.push(self.startup(s, cx));
        groups.push(self.language(s, cx));
        groups.push(self.troubleshooting(cx));
        clamp(600., v_flex().gap_6().children(groups))
    }

    fn modes(&self, s: &UiState, cx: &mut Context<Self>) -> AnyElement {
        let editable = s.run == RunState::Off;
        let t = cx.theme();
        let rows = [
            (
                Mode::Router,
                tr("Router"),
                tr("Devices share this computer's internet").to_owned(),
                true,
            ),
            (
                Mode::Bridge,
                tr("Bridge"),
                if s.features.bridge {
                    tr("Devices join your wired network directly").into()
                } else {
                    tr("Not available with Windows' Mobile Hotspot").into()
                },
                s.features.bridge,
            ),
            (
                Mode::LocalOnly,
                tr("Local only"),
                if s.local_only_supported {
                    tr("Devices can reach each other, but not the internet").into()
                } else if s.features.bridge {
                    trf(
                        "Needs NetworkManager 1.52 or newer (you have {version})",
                        &[("version", &s.nm_version)],
                    )
                } else {
                    tr("Not available with Windows' Mobile Hotspot").into()
                },
                s.local_only_supported,
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (mode, title, sub, available))| {
            let enabled = available && editable;
            let m = self.model.clone();
            h_flex()
                .id(("mode-row", i))
                .px_3()
                .py_2()
                .gap_3()
                .min_h(gpui_kit::px(52.))
                .when(enabled, |d| {
                    d.cursor_pointer()
                        .on_click(move |_, _, cx| send(&m, cx, Msg::Mode(mode)))
                })
                .child(
                    Radio::new(("mode-radio", i))
                        .checked(s.cfg.mode == mode)
                        .disabled(!enabled),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .when(!available, |d| d.text_color(t.muted_foreground))
                        .child(title.to_owned())
                        .child(div().text_xs().text_color(t.muted_foreground).child(sub)),
                )
                .into_any_element()
        })
        .collect();
        widgets::group(Some(tr("Connection mode").into()), None, None, rows, cx)
    }

    fn adapters(&self, s: &UiState, cx: &mut Context<Self>) -> AnyElement {
        let editable = s.run == RunState::Off;
        let t = cx.theme();
        let mut rows = Vec::new();

        // Adapters: "Automatic" first, then every AP-capable adapter.
        let mut adapters: Vec<(Option<String>, String)> = Vec::new();
        if s.adapters.len() > 1 {
            adapters.push((None, tr("Automatic (prefers a USB adapter)").into()));
        }
        adapters.extend(
            s.adapters
                .iter()
                .map(|a| (Some(a.iface.clone()), a.label())),
        );
        // A configured adapter that's unplugged stays visible.
        if let Some(i) = s.cfg.ap_iface.as_deref()
            && !s.adapters.iter().any(|a| a.iface == i)
        {
            let name = if s.features.bridge {
                i
            } else {
                tr("Wi-Fi adapter")
            };
            adapters.push((
                Some(i.to_owned()),
                trf("{iface} (not connected)", &[("iface", name)]),
            ));
        }
        let ai = adapters
            .iter()
            .position(|(i, _)| i.as_deref() == s.cfg.ap_iface.as_deref())
            .unwrap_or(0);
        let detail = match s.current_adapter() {
            Some(a) => {
                let mut parts = Vec::new();
                if s.cfg.ap_iface.is_none() && s.adapters.len() > 1 {
                    parts.push(trf("Uses {adapter}", &[("adapter", &a.label())]));
                }
                if s.features.band {
                    parts.push(a.bands().to_owned());
                }
                if let Some(p) = &a.product {
                    parts.push(p.clone());
                }
                if s.run == RunState::On && s.running_iface.as_deref() == Some(a.iface.as_str()) {
                    parts.push(tr("in use").to_owned());
                }
                parts.join(" · ")
            }
            None => tr("No adapter can create hotspots").to_owned(),
        };
        let ids: Vec<Option<String>> = adapters.iter().map(|(i, _)| i.clone()).collect();
        let m = self.model.clone();
        rows.push(
            row(
                tr("Hotspot adapter"),
                Some(detail.into()),
                choice(
                    "adapter",
                    adapters.iter().map(|(_, l)| l.clone().into()).collect(),
                    ai,
                    !editable || adapters.is_empty(),
                    move |i, cx| send(&m, cx, Msg::Adapter(ids[i].clone())),
                ),
                cx,
            )
            .into_any_element(),
        );
        let problems: Vec<String> = s.adapter_problems.iter().map(|p| p.explain()).collect();
        if !problems.is_empty() {
            rows.push(
                div()
                    .px_3()
                    .py_2()
                    .child(widgets::note(
                        problems.join("\n\n"),
                        s.adapters.is_empty().then_some(t.warning),
                        cx,
                    ))
                    .into_any_element(),
            );
        }

        // Internet source.
        let bridge = s.cfg.mode == Mode::Bridge;
        let mut entries: Vec<(Option<String>, String)> = Vec::new();
        if !bridge {
            entries.push((None, tr("Automatic (same as this computer)").into()));
        }
        entries.extend(
            s.uplinks
                .iter()
                .filter(|u| !bridge || u.can_bridge())
                .map(|u| (Some(u.iface.clone()), u.label())),
        );
        let ui = entries
            .iter()
            .position(|(i, _)| i.as_deref() == s.cfg.upstream_iface.as_deref())
            .unwrap_or(0);
        let selected = s.cfg.upstream_iface.as_deref().and_then(|i| s.uplink(i));
        let detail = match selected {
            Some(u) => u.detail(),
            None if bridge => tr("No wired connection selected").into(),
            None => match s.default_uplink() {
                Some(d) => trf("Same as this computer: {source}", &[("source", &d.label())]),
                None => tr("Same as this computer").into(),
            },
        };
        let ids: Vec<Option<String>> = entries.iter().map(|(i, _)| i.clone()).collect();
        let m = self.model.clone();
        rows.push(
            row(
                if bridge {
                    tr("Wired network to bridge")
                } else {
                    tr("Internet source")
                },
                Some(detail.into()),
                choice(
                    "upstream",
                    entries.iter().map(|(_, l)| l.clone().into()).collect(),
                    ui,
                    !editable || entries.is_empty() || !s.features.uplink_choice,
                    move |i, cx| send(&m, cx, Msg::Upstream(ids[i].clone())),
                ),
                cx,
            )
            .into_any_element(),
        );
        let note = if bridge {
            Some((
                if entries.is_empty() {
                    tr("No wired connection found. Plug in an Ethernet cable or USB tethering, or use Router mode.")
                } else {
                    tr("Only wired connections can be bridged. To share Wi-Fi or mobile internet, use Router mode.")
                }
                .to_string(),
                None,
            ))
        } else if s.features.bridge {
            match (selected, s.default_uplink()) {
                (Some(sel), Some(def)) if sel.connected && !sel.is_default => Some((
                    trf(
                        "Hotspot traffic currently goes through {source}. Sending only the hotspot through {name} needs the upcoming helper.",
                        &[("source", &def.label()), ("name", &sel.label())],
                    ),
                    Some(t.warning),
                )),
                _ => None,
            }
        } else {
            None
        };
        if let Some((text, color)) = note {
            rows.push(
                div()
                    .px_3()
                    .py_2()
                    .child(widgets::note(text, color, cx))
                    .into_any_element(),
            );
        }
        widgets::group(Some(tr("Adapters").into()), None, None, rows, cx)
    }

    fn radio(&self, s: &UiState, cx: &mut Context<Self>) -> Option<AnyElement> {
        let editable = s.run == RunState::Off;
        let mut rows = Vec::new();
        if s.features.band {
            let bi = BANDS.iter().position(|b| *b == s.cfg.band).unwrap_or(0);
            let m = self.model.clone();
            rows.push(
                row(
                    tr("Band"),
                    (!s.ap_supports_5ghz())
                        .then(|| tr("This adapter only supports 2.4 GHz").into()),
                    choice(
                        "band",
                        vec![
                            tr("Automatic").into(),
                            tr("2.4 GHz").into(),
                            tr("5 GHz").into(),
                        ],
                        bi,
                        !editable,
                        move |i, cx| send(&m, cx, Msg::Band(BANDS[i])),
                    ),
                    cx,
                )
                .into_any_element(),
            );
        }
        if s.features.channel {
            let channels: Vec<SharedString> = std::iter::once(tr("Automatic").into())
                .chain(
                    s.cfg
                        .band
                        .allowed_channels()
                        .iter()
                        .map(|c| c.to_string().into()),
                )
                .collect();
            let allowed = s.cfg.band.allowed_channels();
            let ci = allowed
                .iter()
                .position(|c| *c == s.cfg.channel)
                .map_or(0, |i| i + 1);
            let m = self.model.clone();
            rows.push(
                row(
                    tr("Channel"),
                    None,
                    choice(
                        "channel",
                        channels,
                        ci,
                        !editable || s.cfg.band == Band::Auto,
                        move |i, cx| {
                            let ch = if i == 0 { 0 } else { allowed[i - 1] };
                            send(&m, cx, Msg::Channel(ch));
                        },
                    ),
                    cx,
                )
                .into_any_element(),
            );
        }
        if s.features.hidden {
            rows.push(
                row(
                    tr("Hidden network"),
                    Some(tr("Devices must type the name to join").into()),
                    switch("hidden", s.cfg.hidden, !editable, &self.model, Msg::Hidden),
                    cx,
                )
                .into_any_element(),
            );
        }
        (!rows.is_empty()).then(|| widgets::group(Some(tr("Radio").into()), None, None, rows, cx))
    }

    fn speed(&self, s: &UiState, cx: &mut Context<Self>) -> AnyElement {
        let ready = s.helper_ready();
        let view = cx.entity().downgrade();
        let rows = (0..2)
            .map(|i| {
                let view = view.clone();
                self.totals[i].render(
                    if i == 0 { "total-down" } else { "total-up" },
                    if i == 0 {
                        tr("Total download speed")
                    } else {
                        tr("Total upload speed")
                    },
                    !ready,
                    move |u, window, cx| {
                        let _ = view.update(cx, |v, cx| {
                            v.totals[i].unit = u;
                            if let Some(val) = v.totals[i].commit(window, cx) {
                                let msg = if i == 0 {
                                    Msg::TotalDown(val)
                                } else {
                                    Msg::TotalUp(val)
                                };
                                send(&v.model, cx, msg);
                            }
                            cx.notify();
                        });
                    },
                    cx,
                )
            })
            .collect();
        widgets::group(
            Some(tr("Connection speed").into()),
            Some(
                if ready {
                    tr("Limits for the whole hotspot. Set them to your real connection speed for priorities to work well.")
                } else {
                    tr("Needs the Easy Hotspot helper.")
                }
                .into(),
            ),
            None,
            rows,
            cx,
        )
    }

    fn new_devices(&self, s: &UiState, cx: &mut Context<Self>) -> AnyElement {
        let ready = s.helper_ready();
        let t = cx.theme();
        let mut rows = vec![
            row(
                tr("Ask before letting devices join"),
                Some(tr("New devices wait until you allow them").into()),
                switch(
                    "approval",
                    s.defaults.require_approval,
                    !ready,
                    &self.model,
                    Msg::Approval,
                ),
                cx,
            )
            .into_any_element(),
        ];
        let view = cx.entity().downgrade();
        rows.extend(limits_rows(
            "def",
            &self.defaults,
            &s.defaults.limits,
            !ready,
            {
                let m = self.model.clone();
                move |f, cx| send(&m, cx, Msg::Default(f))
            },
            move |i, u, window, cx| {
                let _ = view.update(cx, |v, cx| {
                    v.defaults[i].unit = u;
                    if let Some(val) = v.defaults[i].commit(window, cx) {
                        send(
                            &v.model,
                            cx,
                            Msg::Default(super::limit_field(SLOTS[i], val)),
                        );
                    }
                    cx.notify();
                });
            },
            cx,
        ));
        widgets::group(
            Some(tr("New devices").into()),
            Some(tr("Applied automatically the first time a device connects.").into()),
            (!ready).then(|| {
                div()
                    .text_xs()
                    .text_color(t.muted_foreground)
                    .child(tr("Needs helper"))
                    .into_any_element()
            }),
            rows,
            cx,
        )
    }

    fn startup(&self, s: &UiState, cx: &mut Context<Self>) -> AnyElement {
        let rows = vec![
            row(
                tr("Launch at login"),
                Some(tr("Open in the background when you log in").into()),
                switch(
                    "autostart",
                    s.cfg.autostart,
                    false,
                    &self.model,
                    Msg::Autostart,
                ),
                cx,
            )
            .into_any_element(),
            row(
                tr("Start hotspot at login"),
                Some(tr("Start sharing as soon as you log in").into()),
                switch(
                    "start-at-login",
                    s.cfg.start_at_login,
                    !s.cfg.autostart,
                    &self.model,
                    Msg::StartAtLogin,
                ),
                cx,
            )
            .into_any_element(),
            row(
                tr("Show in system tray"),
                Some(
                    if s.tray_available {
                        tr("Closing the window keeps the app running in the tray")
                    } else {
                        tr("Your desktop doesn't show a system tray")
                    }
                    .into(),
                ),
                switch("tray", s.cfg.show_tray, false, &self.model, Msg::Tray),
                cx,
            )
            .into_any_element(),
        ];
        widgets::group(Some(tr("Startup").into()), None, None, rows, cx)
    }

    fn language(&self, s: &UiState, cx: &mut Context<Self>) -> AnyElement {
        let li = Language::ALL
            .iter()
            .position(|l| *l == s.cfg.language)
            .unwrap_or(0);
        let m = self.model.clone();
        widgets::card(
            vec![
                row(
                    tr("Language"),
                    Some(tr("Takes effect after restarting the app").into()),
                    choice(
                        "language",
                        vec![tr("System").into(), "English".into(), "Español".into()],
                        li,
                        false,
                        move |i, cx| send(&m, cx, Msg::Language(Language::ALL[i])),
                    ),
                    cx,
                )
                .into_any_element(),
            ],
            cx,
        )
    }

    fn troubleshooting(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows = vec![
            row(
                tr("Export diagnostics…"),
                Some(tr("Save a report to attach to bug reports (passwords removed)").into()),
                icon(Lucide::ChevronRight),
                cx,
            )
            .id("diagnostics")
            .cursor_pointer()
            .on_click(self.on(|| Msg::ExportDiagnostics))
            .into_any_element(),
        ];
        widgets::group(Some(tr("Troubleshooting").into()), None, None, rows, cx)
    }
}

const SLOTS: [super::LimitSlot; 3] = [
    super::LimitSlot::Down,
    super::LimitSlot::Up,
    super::LimitSlot::Cap,
];

/// Rows for download, upload, priority, data cap and cap action.
/// `on_unit(i, unit)` is called when the unit of typed field `i` changes.
pub(super) fn limits_rows(
    id: &'static str,
    inputs: &[LimitInput; 3],
    l: &Limits,
    disabled: bool,
    on: impl Fn(LimitField, &mut App) + Clone + 'static,
    on_unit: impl Fn(usize, usize, &mut Window, &mut App) + Clone + 'static,
    cx: &App,
) -> Vec<AnyElement> {
    let ids: [&'static str; 3] = match id {
        "dev" => ["dev-down", "dev-up", "dev-cap"],
        _ => ["def-down", "def-up", "def-cap"],
    };
    let typed = |i: usize, title: &str| {
        let on_unit = on_unit.clone();
        inputs[i].render(
            ids[i],
            title,
            disabled,
            move |u, window, cx| on_unit(i, u, window, cx),
            cx,
        )
    };
    let pi = PRIORITIES
        .iter()
        .position(|p| *p == l.priority)
        .unwrap_or(1);
    let ai = CAP_ACTIONS
        .iter()
        .position(|a| *a == l.cap_action)
        .unwrap_or(0);
    let (on1, on2) = (on.clone(), on);
    vec![
        typed(0, tr("Download limit")),
        typed(1, tr("Upload limit")),
        row(
            tr("Priority"),
            Some(tr("Who gets bandwidth first when the link is busy").into()),
            choice(
                SharedString::from(format!("{id}-priority")),
                vec![tr("High").into(), tr("Normal").into(), tr("Low").into()],
                pi,
                disabled,
                move |i, cx| on1(LimitField::Priority(PRIORITIES[i]), cx),
            ),
            cx,
        )
        .into_any_element(),
        typed(2, tr("Data cap per session")),
        row(
            tr("When the cap is reached"),
            None,
            choice(
                SharedString::from(format!("{id}-cap-action")),
                vec![
                    tr("Block the device").into(),
                    tr("Slow to 256 kbps").into(),
                    tr("Slow to 1 Mbps").into(),
                ],
                ai,
                disabled || l.cap_mb.is_none(),
                move |i, cx| on2(LimitField::CapAction(CAP_ACTIONS[i]), cx),
            ),
            cx,
        )
        .into_any_element(),
    ]
}
