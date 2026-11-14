//! Dialogs asked for by the model, the problem pages' texts, and the
//! keyboard shortcuts list.

use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::{App, Entity, IntoElement, ParentElement, Styled, Window, div, prelude::*};

use super::widgets::{Lucide, icon};
use crate::i18n::tr;
use crate::model::{Ask, Model, send};
use crate::msg::Msg;
use crate::state::{HelperStatus, Problem};

pub fn problem_text(p: Problem) -> (Lucide, &'static str, &'static str, &'static str) {
    match p {
        Problem::NmMissing => (
            Lucide::WifiOff,
            tr("NetworkManager is required"),
            tr(
                "Easy Hotspot creates hotspots through NetworkManager 1.30 or newer, which isn't running on this system.",
            ),
            tr("Learn more"),
        ),
        Problem::NoAdapter => (
            Lucide::WifiOff,
            tr("Your Wi-Fi card can't create hotspots"),
            tr(
                "None of your Wi-Fi adapters support access-point mode. A USB Wi-Fi adapter that supports AP mode will work.",
            ),
            tr("Refresh"),
        ),
        Problem::BridgeNoEthernet => (
            Lucide::Cable,
            tr("Plug in an Ethernet cable"),
            tr(
                "Bridge mode connects your devices to a wired network. Connect a cable, or share your internet in Router mode instead.",
            ),
            tr("Use Router mode"),
        ),
        Problem::AdapterGone => (
            Lucide::WifiOff,
            tr("Wi-Fi adapter unavailable"),
            tr("The Wi-Fi adapter was turned off or unplugged."),
            tr("Retry"),
        ),
        // The reason comes from Windows (`UiState::problem_detail`).
        Problem::Unavailable => (
            Lucide::WifiOff,
            tr("Windows can't share a hotspot"),
            "",
            tr("Refresh"),
        ),
    }
}

/// A monospace box with a command and a copy button.
fn command_box(model: &Entity<Model>, cmd: String, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let m = model.clone();
    let c = cmd.clone();
    h_flex()
        .gap_2()
        .p_2()
        .rounded_md()
        .bg(t.secondary)
        .child(div().flex_1().font_family("monospace").text_sm().child(cmd))
        .child(
            Button::new("copy-cmd")
                .icon(icon(Lucide::Copy))
                .ghost()
                .small()
                .tooltip(tr("Copy command"))
                .on_click(move |_, _, cx| send(&m, cx, Msg::CopyText(c.clone()))),
        )
}

pub fn open(model: Entity<Model>, ask: &Ask, window: &mut Window, cx: &mut App) {
    let ask = ask.clone();
    match ask {
        Ask::Bridge => window.open_alert_dialog(cx, move |d, _, _| {
            let (m1, m2) = (model.clone(), model.clone());
            d.title(tr("Switch to Bridge mode?"))
                .description(tr(
                    "Devices will join your wired network and get addresses from your router. While the hotspot is on, this computer's wired connection moves into a bridge: it keeps the same address on your network, but drops for a few seconds when the hotspot starts and stops.\n\nTo share files with devices on the hotspot (for example with Warpinator), set those apps to use any network interface, or use Router mode, which never touches the wired connection.",
                ))
                .ok_text(tr("Use Bridge mode"))
                .cancel_text(tr("Cancel"))
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    send(&m1, cx, Msg::BridgeConfirmed(true));
                    true
                })
                .on_cancel(move |_, _, cx| {
                    send(&m2, cx, Msg::BridgeConfirmed(false));
                    true
                })
        }),
        Ask::Helper => {
            let (helper, demo) = {
                let m = model.read(cx);
                (m.state.helper, m.is_demo())
            };
            let update = helper == HelperStatus::TooOld;
            let info = crate::backend::helper_install();
            window.open_alert_dialog(cx, move |d, _, cx| {
                let title = if update {
                    tr("Update the helper")
                } else {
                    tr("Install the helper")
                };
                let body = if update {
                    tr("The installed helper is older than this app and needs an update.")
                } else {
                    tr(
                        "Speed limits, approval of new devices and data caps need a small helper that runs with administrator rights. Everything else works without it.",
                    )
                };
                let hint = if info.can_install {
                    tr("Or run this in a terminal:")
                } else if info.no_new_privs {
                    tr("This app was started from a terminal that can't ask for administrator rights. Run this in a normal terminal:")
                } else {
                    tr("Run this in a terminal:")
                };
                let cmd = if demo {
                    "sudo apt install easyhotspot".to_owned()
                } else {
                    info.command.clone()
                };
                let t = cx.theme();
                let content = v_flex()
                    .gap_3()
                    .child(body)
                    .child(div().text_sm().text_color(t.muted_foreground).child(hint))
                    .child(command_box(&model, cmd, cx));
                let m = model.clone();
                let d = d.title(title).description(content).cancel_text(tr("Close")).show_cancel(true);
                if info.can_install && !demo {
                    d.ok_text(if update {
                        tr("Update now")
                    } else {
                        tr("Install now")
                    })
                    .on_ok(move |_, _, cx| {
                        send(&m, cx, Msg::InstallHelper);
                        true
                    })
                } else {
                    d.ok_text(tr("Check again")).on_ok(move |_, _, cx| {
                        send(&m, cx, Msg::HelperCheck);
                        true
                    })
                }
            });
        }
        Ask::ErrorDetails { title, details } => window.open_alert_dialog(cx, move |d, _, _| {
            let m = model.clone();
            d.title(title.clone())
                .description(details.clone())
                .ok_text(tr("Try again"))
                .cancel_text(tr("Close"))
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    send(&m, cx, Msg::Toggle);
                    true
                })
        }),
        Ask::Restart => window.open_alert_dialog(cx, move |d, _, _| {
            let m = model.clone();
            d.title(tr("Restart to change the language"))
                .description(tr(
                    "Easy Hotspot needs to restart to show the new language. A running hotspot keeps working.",
                ))
                .ok_text(tr("Restart now"))
                .cancel_text(tr("Later"))
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    send(&m, cx, Msg::Restart);
                    true
                })
        }),
        Ask::About => {
            let helper = match model.read(cx).state.helper {
                HelperStatus::Ready => tr("Helper: installed"),
                HelperStatus::Missing => tr("Helper: not installed"),
                HelperStatus::Unauthorized => tr("Helper: installed, not authorized"),
                HelperStatus::TooOld => tr("Helper: needs an update"),
                HelperStatus::Unsupported => "",
            };
            window.open_alert_dialog(cx, move |d, _, cx| {
                let t = cx.theme();
                d.title(leh_model::APP_NAME)
                    .description(
                        v_flex()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(t.muted_foreground)
                                    .child(format!("{} {}", tr("Version"), env!("CARGO_PKG_VERSION"))),
                            )
                            .child(tr("The simplest way to share a Wi-Fi hotspot."))
                            .when(!helper.is_empty(), |d| d.child(helper))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(t.muted_foreground)
                                    .child(tr("Easy Hotspot contributors · GNU GPL v3 or later")),
                            ),
                    )
                    .ok_text(tr("Close"))
            });
        }
        Ask::InstallFailed(e) => window.open_alert_dialog(cx, move |d, _, _| {
            d.title(tr("Couldn't install the helper"))
                .description(e.clone())
                .ok_text(tr("Close"))
                .ok_variant(ButtonVariant::Secondary)
        }),
    }
}

pub fn shortcuts(window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, |d, _, cx| {
        let t = cx.theme();
        let row = |keys: &str, what: &str| {
            h_flex()
                .justify_between()
                .gap_4()
                .child(what.to_owned())
                .child(
                    div()
                        .px_2()
                        .rounded_sm()
                        .bg(t.secondary)
                        .font_family("monospace")
                        .text_sm()
                        .child(keys.to_owned()),
                )
        };
        d.title(tr("Keyboard Shortcuts"))
            .description(
                v_flex()
                    .gap_2()
                    .child(row("Ctrl+Space", tr("Turn hotspot on or off")))
                    .child(row("Ctrl+Shift+C", tr("Copy password")))
                    .child(row("Ctrl+,", tr("Settings")))
                    .child(row("Ctrl+Q", tr("Quit"))),
            )
            .ok_text(tr("Close"))
    });
}
