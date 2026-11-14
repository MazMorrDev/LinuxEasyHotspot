//! Clients: hotspot usage, devices waiting for approval, and every known
//! device (connected or not) grouped into Allowed and Blocked. Each device
//! opens its page, where its name, access and limits are set.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, StyledExt as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, div, prelude::*, px,
};
use leh_model::policy::Access;

use super::widgets::{self, Lucide, icon};
use super::{MainView, clamp};
use crate::i18n::{tr, tr_n, trf};
use crate::msg::Msg;
use crate::state::{
    ClientVm, DeviceIcon, HelperStatus, RunState, UiState, fmt_bytes, fmt_last_seen, fmt_rate,
    policy_summary,
};

pub(super) fn device_icon(i: DeviceIcon) -> Lucide {
    match i {
        DeviceIcon::Phone => Lucide::Smartphone,
        DeviceIcon::Laptop => Lucide::Laptop,
        DeviceIcon::Tv => Lucide::Tv,
        DeviceIcon::Unknown => Lucide::CircleQuestionMark,
    }
}

fn subtitle(s: &UiState, c: &ClientVm, helper: bool) -> String {
    let ip = c.ip.clone().unwrap_or_else(|| tr("no address yet").into());
    let vendor = c.vendor.map_or(tr("Unknown vendor"), tr);
    if !helper {
        return format!("{ip} · {vendor}");
    }
    match s.effective(c).access {
        Access::Pending => format!("{ip} · {vendor}"),
        _ => policy_summary(s, c),
    }
}

/// A known device that isn't connected, as a view-model for its row.
fn offline_vm(s: &UiState, mac: &str) -> ClientVm {
    let hostname = s.policies.get(mac).and_then(|p| p.hostname.clone());
    crate::backend::common::client_vm(&leh_model::clients::Client {
        mac: mac.to_owned(),
        ip: None,
        hostname,
    })
}

/// What a row offers besides opening the device.
#[derive(Clone, Copy)]
enum Actions {
    None,
    /// Block and Allow (waiting for approval).
    BlockAllow,
    /// Allow (blocked).
    Allow,
}

impl MainView {
    pub(super) fn clients(&mut self, s: &UiState, cx: &mut Context<Self>) -> AnyElement {
        let warning = cx.theme().warning;
        let running = s.run == RunState::On;
        let has_known = !s.policies.is_empty();
        if !running && !has_known {
            return status_page(
                Lucide::Wifi,
                tr("Hotspot is off"),
                tr("Start the hotspot to see connected devices."),
                cx,
            );
        }
        if running && s.clients.is_empty() && !has_known {
            return status_page(
                Lucide::Smartphone,
                tr("No devices yet"),
                tr("Scan the QR code on Home to connect."),
                cx,
            );
        }
        let helper = s.helper_ready();
        let mut parts: Vec<AnyElement> = Vec::new();
        if !running {
            parts.push(widgets::note(
                tr("The hotspot is off. You can still change the settings of each device; they apply the next time it connects."),
                None,
                cx,
            ));
        } else {
            let tr_ = &s.traffic;
            parts.push(widgets::card(
                vec![
                    widgets::row(
                        tr("Speed right now"),
                        Some(tr("All devices together").into()),
                        div().text_sm().child(format!(
                            "↓ {}   ↑ {}",
                            fmt_rate(tr_.down_kbps),
                            fmt_rate(tr_.up_kbps)
                        )),
                        cx,
                    )
                    .into_any_element(),
                    widgets::row(
                        tr("Used since the hotspot started"),
                        Some(
                            trf(
                                "Downloaded {down} · uploaded {up}",
                                &[
                                    ("down", &fmt_bytes(tr_.down_bytes)),
                                    ("up", &fmt_bytes(tr_.up_bytes)),
                                ],
                            )
                            .into(),
                        ),
                        div()
                            .text_sm()
                            .child(fmt_bytes(tr_.down_bytes + tr_.up_bytes)),
                        cx,
                    )
                    .into_any_element(),
                ],
                cx,
            ));
        }

        let (mut pending, mut allowed, mut blocked) = (Vec::new(), Vec::new(), Vec::new());
        let mut n_online = 0;
        let per_device = s.helper != HelperStatus::Unsupported;
        // Connected devices first, in each group.
        for c in s.online_clients() {
            match s.policy(&c.mac).access {
                // Approval is only enforced with the helper.
                Access::Pending if helper => {
                    pending.push(self.client_row(s, c, false, Actions::BlockAllow, cx));
                }
                Access::Blocked if per_device => {
                    blocked.push(self.client_row(s, c, false, Actions::Allow, cx));
                }
                _ => {
                    n_online += 1;
                    allowed.push(self.client_row(s, c, helper, Actions::None, cx));
                }
            }
        }
        for (mac, p) in s.offline_devices() {
            let (list, actions) = match p.access {
                Access::Pending if per_device => (&mut pending, Actions::BlockAllow),
                Access::Blocked if per_device => (&mut blocked, Actions::Allow),
                _ => (&mut allowed, Actions::None),
            };
            list.push(self.offline_row(s, mac, actions, cx));
        }

        let heading = |text: String| div().mt_2().font_semibold().child(text).into_any_element();
        if !pending.is_empty() {
            parts.push(heading(tr("Waiting for approval").into()));
            parts.push(widgets::card(pending, cx));
        }
        parts.push(heading(trf(
            "Allowed · {n}",
            &[("n", &allowed.len().to_string())],
        )));
        if running {
            parts.push(widgets::note(
                if n_online == 0 {
                    tr("No device is connected right now. Scan the QR code on Home to connect.")
                        .to_owned()
                } else {
                    tr_n("{n} connected now", "{n} connected now", n_online)
                },
                None,
                cx,
            ));
        }
        if !allowed.is_empty() {
            parts.push(widgets::card(allowed, cx));
        }
        if !blocked.is_empty() {
            parts.push(heading(trf(
                "Blocked · {n}",
                &[("n", &blocked.len().to_string())],
            )));
            if !helper {
                parts.push(widgets::note(
                    tr("Blocking needs the Easy Hotspot helper. Until it's set up, these devices can still connect."),
                    Some(warning),
                    cx,
                ));
            }
            parts.push(widgets::card(blocked, cx));
        }
        clamp(600., v_flex().gap_2().children(parts))
    }

    /// Row for a connected device. `rates` shows its live speed (needs the
    /// helper).
    fn client_row(
        &self,
        s: &UiState,
        c: &ClientVm,
        rates: bool,
        actions: Actions,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let helper = s.helper_ready();
        let mut suffix = h_flex().gap_2();
        if helper && s.effective(c).cap_reached {
            suffix = suffix.child(
                div()
                    .px_2()
                    .rounded_full()
                    .bg(t.warning.opacity(0.2))
                    .text_color(t.warning)
                    .text_xs()
                    .child(tr("Cap reached")),
            );
        }
        if rates {
            suffix = suffix.child(
                div()
                    .text_xs()
                    .text_color(t.muted_foreground)
                    .child(format!(
                        "↓{} ↑{}",
                        fmt_rate(c.down_kbps),
                        fmt_rate(c.up_kbps)
                    )),
            );
        }
        self.device_row(
            &c.mac,
            device_icon(c.icon),
            false,
            s.display_name(c),
            subtitle(s, c, helper),
            suffix.into_any_element(),
            actions,
            cx,
        )
    }

    /// Row for a known device that isn't connected right now.
    fn offline_row(
        &self,
        s: &UiState,
        mac: &str,
        actions: Actions,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let vm = offline_vm(s, mac);
        let last_seen = s.policies.get(mac).and_then(|p| p.last_seen);
        let vendor = vm.vendor.map_or(tr("Unknown vendor"), tr);
        self.device_row(
            mac,
            device_icon(vm.icon),
            true,
            s.known_name(mac),
            format!("{} · {vendor}", fmt_last_seen(last_seen)),
            div().into_any_element(),
            actions,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn device_row(
        &self,
        mac: &str,
        ico: Lucide,
        dim: bool,
        title: String,
        subtitle: String,
        suffix: AnyElement,
        actions: Actions,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = cx.theme();
        let id = SharedString::from(format!("dev-{mac}"));
        let m1 = mac.to_owned();
        let m2 = mac.to_owned();
        let m3 = mac.to_owned();
        let buttons = match actions {
            Actions::None => None,
            Actions::BlockAllow => Some(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("block-{mac}")))
                            .label(tr("Block"))
                            .small()
                            .on_click(self.on(move || Msg::Block(m1.clone()))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("allow-{mac}")))
                            .label(tr("Allow"))
                            .primary()
                            .small()
                            .on_click(self.on(move || Msg::Allow(m2.clone()))),
                    ),
            ),
            Actions::Allow => Some(
                h_flex().child(
                    Button::new(SharedString::from(format!("allow-{mac}")))
                        .label(tr("Allow"))
                        .ghost()
                        .small()
                        .on_click(self.on(move || Msg::Allow(m2.clone()))),
                ),
            ),
        };
        h_flex()
            .id(id)
            .w_full()
            .min_h(px(56.))
            .px_3()
            .py_2()
            .gap_3()
            .cursor_pointer()
            .hover(|d| d.bg(t.accent.opacity(0.4)))
            .on_click(self.on(move || Msg::OpenDevice(m3.clone())))
            .child(
                icon(ico)
                    .size(px(20.))
                    .when(dim, |i| i.text_color(t.muted_foreground)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(title))
                    .child(
                        div()
                            .text_xs()
                            .text_color(t.muted_foreground)
                            .child(subtitle),
                    ),
            )
            .child(suffix)
            .children(buttons)
            .child(
                icon(Lucide::ChevronRight)
                    .small()
                    .text_color(t.muted_foreground),
            )
            .into_any_element()
    }
}

pub(super) fn status_page(
    ico: Lucide,
    title: &str,
    desc: &str,
    cx: &mut Context<MainView>,
) -> AnyElement {
    let t = cx.theme();
    v_flex()
        .w_full()
        .pt(px(72.))
        .px_6()
        .items_center()
        .gap_3()
        .child(icon(ico).size(px(64.)).text_color(t.muted_foreground))
        .child(div().text_xl().font_semibold().child(title.to_owned()))
        .child(
            div()
                .text_center()
                .text_color(t.muted_foreground)
                .child(desc.to_owned()),
        )
        .into_any_element()
}
