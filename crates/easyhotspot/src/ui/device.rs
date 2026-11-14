//! A device's page: live speed and usage, name, access and limits. Known
//! devices can be edited while they're away.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::progress::Progress;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, IntoElement, ParentElement, Styled, Window, div,
    prelude::*, px,
};
use leh_model::policy::Access;
use leh_model::units::Kind;

use super::settings::limits_rows;
use super::widgets::{self, LimitInput, LimitSlot, Lucide, choice, icon, row};
use super::{DeviceForm, MainView, clamp, limit_field};
use crate::i18n::{tr, trf};
use crate::model::send;
use crate::msg::Msg;
use crate::state::{HelperStatus, UiState, fmt_bytes, fmt_last_seen, fmt_rate};

const ACCESS: [Access; 3] = [Access::Allowed, Access::Pending, Access::Blocked];
const SLOTS: [LimitSlot; 3] = [LimitSlot::Down, LimitSlot::Up, LimitSlot::Cap];

impl MainView {
    pub(super) fn device_form(
        &self,
        mac: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> DeviceForm {
        let name_text = self.model.read(cx).state.known_name(&mac);
        let name = cx.new(|cx| InputState::new(window, cx).default_value(name_text));
        let limits = [
            LimitInput::new(Kind::Rate, window, cx),
            LimitInput::new(Kind::Rate, window, cx),
            LimitInput::new(Kind::Data, window, cx),
        ];
        let mut subs = Vec::new();
        let (m, mc) = (self.model.clone(), mac.clone());
        subs.push(
            cx.subscribe_in(&name, window, move |_, input, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    let v = input.read(cx).value().to_string();
                    send(&m, cx, Msg::DeviceAlias(mc.clone(), v));
                }
            }),
        );
        for i in 0..3 {
            let mc = mac.clone();
            subs.push(limits[i].subscribe(
                window,
                cx,
                move |v: &mut Self| v.device.as_mut().map(|d| &mut d.limits[i]),
                move |v, val, _, cx| {
                    send(
                        &v.model,
                        cx,
                        Msg::DeviceLimit(mc.clone(), limit_field(SLOTS[i], val)),
                    );
                },
            ));
        }
        DeviceForm {
            mac,
            name,
            limits,
            _subs: subs,
        }
    }

    /// Shows the device's current values in its fields.
    pub(super) fn sync_device(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mac) = self.device.as_ref().map(|d| d.mac.clone()) else {
            return;
        };
        let (name, lim) = {
            let s = &self.model.read(cx).state;
            let pol = s.policy(&mac);
            (s.known_name(&mac), *pol.limits(&s.defaults))
        };
        let Some(d) = self.device.as_mut() else {
            return;
        };
        super::sync_text(&d.name, &name, window, cx);
        for (li, v) in d
            .limits
            .iter_mut()
            .zip([lim.down_kbps, lim.up_kbps, lim.cap_mb])
        {
            li.sync(v, window, cx);
        }
    }

    pub(super) fn device_page(
        &mut self,
        s: &UiState,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(d) = self.device.as_ref() else {
            return div().into_any_element();
        };
        let t = cx.theme();
        let mac = d.mac.clone();
        let client = s.client(&mac);
        let pol = s.policy(&mac);
        let lim = *pol.limits(&s.defaults);
        let per_device = s.helper != HelperStatus::Unsupported;

        // Speed and usage.
        let (rates, usage, frac) = match client {
            Some(c) => {
                let eff = s.effective(c);
                let rates = format!("↓ {}   ↑ {}", fmt_rate(c.down_kbps), fmt_rate(c.up_kbps));
                match lim.cap_mb.filter(|_| per_device) {
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
                        (rates, text, Some(frac.min(1.0)))
                    }
                    None => (
                        rates,
                        trf(
                            "This session: {used}",
                            &[("used", &fmt_bytes(c.session_bytes))],
                        ),
                        None,
                    ),
                }
            }
            None => (
                tr("Not connected").to_owned(),
                fmt_last_seen(pol.last_seen),
                None,
            ),
        };
        let show_rates = client.is_some() && (s.helper_ready() || !per_device);
        let header = v_flex()
            .items_center()
            .gap_1()
            .when(show_rates || client.is_none(), |d| {
                d.child(div().text_2xl().font_semibold().child(rates))
            })
            .child(div().text_color(t.muted_foreground).child(usage))
            .children(frac.map(|f| {
                div()
                    .w_full()
                    .mt_1()
                    .child(Progress::new("cap").value((f * 100.0) as f32))
            }));

        let vendor = leh_model::clients::Client {
            mac: mac.clone(),
            ..Default::default()
        }
        .vendor()
        .map_or(tr("Unknown vendor"), tr);
        let address = match client {
            Some(c) => format!(
                "{} · {} · {vendor}",
                c.ip.as_deref().unwrap_or(tr("no address")),
                mac
            ),
            None => format!("{mac} · {vendor}"),
        };

        let mut device_rows = vec![
            v_flex()
                .px_3()
                .py_2()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(t.muted_foreground)
                        .child(tr("Name")),
                )
                .child(Input::new(&d.name))
                .into_any_element(),
            row(tr("Address"), Some(address.into()), div(), cx).into_any_element(),
        ];
        if per_device {
            let ai = ACCESS.iter().position(|a| *a == pol.access).unwrap_or(0);
            let (m, mc) = (self.model.clone(), mac.clone());
            device_rows.push(
                row(
                    tr("Access"),
                    None,
                    choice(
                        "access",
                        vec![
                            tr("Allowed").into(),
                            tr("Waiting for approval").into(),
                            tr("Blocked").into(),
                        ],
                        ai,
                        false,
                        move |i, cx| send(&m, cx, Msg::DeviceAccess(mc.clone(), ACCESS[i])),
                    ),
                    cx,
                )
                .into_any_element(),
            );
        }

        let mut groups = vec![
            header.into_any_element(),
            widgets::group(Some(tr("Device").into()), None, None, device_rows, cx),
        ];

        if per_device {
            let mc = mac.clone();
            let model = self.model.clone();
            let mut rows = vec![
                row(
                    tr("Use default settings"),
                    Some(tr("Follow the settings for new devices").into()),
                    {
                        let m = model.clone();
                        let mc = mc.clone();
                        gpui_kit::component::switch::Switch::new("follow")
                            .checked(pol.follow_defaults)
                            .on_click(move |v, _, cx| {
                                send(&m, cx, Msg::DeviceFollow(mc.clone(), *v))
                            })
                            .into_any_element()
                    },
                    cx,
                )
                .into_any_element(),
            ];
            let view = cx.entity().downgrade();
            let mc2 = mc.clone();
            rows.extend(limits_rows(
                "dev",
                &d.limits,
                &lim,
                pol.follow_defaults,
                move |f, cx| send(&model, cx, Msg::DeviceLimit(mc.clone(), f)),
                move |i, u, window, cx| {
                    let mc = mc2.clone();
                    let _ = view.update(cx, |v, cx| {
                        let Some(d) = v.device.as_mut() else { return };
                        d.limits[i].unit = u;
                        if let Some(val) = d.limits[i].commit(window, cx) {
                            send(
                                &v.model,
                                cx,
                                Msg::DeviceLimit(mc, limit_field(SLOTS[i], val)),
                            );
                        }
                        cx.notify();
                    });
                },
                cx,
            ));
            groups.push(widgets::group(
                Some(tr("Limits").into()),
                None,
                None,
                rows,
                cx,
            ));
        }

        let (m, mc) = (self.model.clone(), mac.clone());
        groups.push(
            h_flex()
                .justify_center()
                .child(
                    Button::new("forget")
                        .label(tr("Forget this device"))
                        .danger()
                        .rounded(px(999.))
                        .px_6()
                        .on_click(move |_, window, cx| {
                            let (m, mc) = (m.clone(), mc.clone());
                            window.open_alert_dialog(cx, move |d, _, _| {
                                let (m, mc) = (m.clone(), mc.clone());
                                d.title(tr("Forget this device?"))
                                    .description(tr("Its name and limits are removed. If it connects again, it is treated as a new device."))
                                    .ok_text(tr("Forget"))
                                    .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                                    .cancel_text(tr("Cancel"))
                                    .show_cancel(true)
                                    .on_ok(move |_, _, cx| {
                                        send(&m, cx, Msg::ForgetDevice(mc.clone()));
                                        true
                                    })
                            });
                        }),
                )
                .into_any_element(),
        );

        let back = Button::new("back")
            .icon(icon(Lucide::ChevronLeft))
            .ghost()
            .small()
            .tooltip(tr("Back"))
            .on_click(self.on(|| Msg::DeviceClosed));
        v_flex()
            .child(
                h_flex().px_2().pt_3().gap_2().child(back).child(
                    div()
                        .flex_1()
                        .truncate()
                        .font_semibold()
                        .text_lg()
                        .child(s.known_name(&mac)),
                ),
            )
            .child(clamp(520., v_flex().gap_6().children(groups)))
            .into_any_element()
    }
}
