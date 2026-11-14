//! The window: a header with the page tabs and the main menu, banners, the
//! current page (or a full-page problem), and a bottom tab bar when narrow.
//!
//! The window only renders the [`Model`] and sends it messages; closing it
//! doesn't stop anything (with a tray icon, the app keeps running).

mod clients;
mod device;
mod dialogs;
mod home;
mod qr;
mod settings;
pub mod theme;
pub mod widgets;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, Entity, FocusHandle,
    Focusable as _, InteractiveElement as _, IntoElement, KeyBinding, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, Subscription, TitlebarOptions, Window,
    WindowBounds, WindowOptions, div, prelude::*, px, size,
};
use leh_model::policy::Access;
use leh_model::units::Kind;

use crate::i18n::tr;
use crate::model::{Model, ModelEvent, send};
use crate::msg::{LimitField, Msg, Page};
use crate::state::{HelperStatus, RunState, UiState};
use widgets::{LimitInput, LimitSlot, Lucide, icon};

gpui_kit::actions!(
    easyhotspot,
    [ToggleHotspot, CopyPassword, OpenSettings, Quit]
);

/// Layout breakpoints (logical pixels).
const NARROW: f32 = 500.;
const WIDE: f32 = 720.;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-space", ToggleHotspot, None),
        KeyBinding::new("ctrl-shift-c", CopyPassword, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("ctrl-q", Quit, None),
    ]);
}

/// Opens the main window (or brings it up if it's already open).
pub fn show_window(model: &Entity<Model>, size_px: Option<(f32, f32)>, cx: &mut App) {
    if let Some(w) = cx.windows().first() {
        let _ = w.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let (w, h) = size_px.unwrap_or((420., 680.));
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(w), px(h)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(leh_model::APP_NAME.into()),
            ..Default::default()
        }),
        window_min_size: Some(size(px(360.), px(300.))),
        app_id: Some(crate::APP_ID.into()),
        ..Default::default()
    };
    let model = model.clone();
    if let Err(e) = gpui_kit::open_window(opts, cx, |window, cx| {
        cx.new(|cx| MainView::new(model, window, cx))
    }) {
        eprintln!("window: {e}");
    }
}

/// The device page's fields.
pub struct DeviceForm {
    pub mac: String,
    pub name: Entity<InputState>,
    pub limits: [LimitInput; 3],
    _subs: Vec<Subscription>,
}

pub struct MainView {
    pub model: Entity<Model>,
    focus: FocusHandle,
    pub ssid: Entity<InputState>,
    pub password: Entity<InputState>,
    /// The password is shown in clear on Home while the hotspot runs.
    pub pw_revealed: bool,
    /// Total download / upload limits.
    pub totals: [LimitInput; 2],
    /// Defaults for new devices: download, upload, data cap.
    pub defaults: [LimitInput; 3],
    pub device: Option<DeviceForm>,
    pub qr: qr::Qr,
    last_run: RunState,
    _subs: Vec<Subscription>,
}

impl MainView {
    fn new(model: Entity<Model>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus = cx.focus_handle();
        let (ssid_text, pw_text) = {
            let s = &model.read(cx).state;
            (s.cfg.ssid.clone(), s.cfg.password.clone())
        };
        let ssid = cx.new(|cx| InputState::new(window, cx).default_value(ssid_text));
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .default_value(pw_text)
        });
        let mut subs = vec![
            cx.observe_in(&model, window, |this, _, window, cx| this.sync(window, cx)),
            cx.subscribe_in(&model, window, |this, _, ev: &ModelEvent, window, cx| {
                this.on_model_event(ev, window, cx);
            }),
        ];
        let m = model.clone();
        subs.push(
            cx.subscribe_in(&ssid, window, move |_, input, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    let v = input.read(cx).value().to_string();
                    send(&m, cx, Msg::Ssid(v));
                }
            }),
        );
        let m = model.clone();
        subs.push(cx.subscribe_in(
            &password,
            window,
            move |_, input, ev: &InputEvent, _, cx| {
                if matches!(ev, InputEvent::Change) {
                    let v = input.read(cx).value().to_string();
                    send(&m, cx, Msg::Password(v));
                }
            },
        ));

        let totals = [
            LimitInput::new(Kind::Rate, window, cx),
            LimitInput::new(Kind::Rate, window, cx),
        ];
        let defaults = [
            LimitInput::new(Kind::Rate, window, cx),
            LimitInput::new(Kind::Rate, window, cx),
            LimitInput::new(Kind::Data, window, cx),
        ];
        for (i, total) in totals.iter().enumerate() {
            subs.push(total.subscribe(
                window,
                cx,
                move |v: &mut Self| v.totals.get_mut(i),
                move |v, val, _, cx| {
                    let msg = if i == 0 {
                        Msg::TotalDown(val)
                    } else {
                        Msg::TotalUp(val)
                    };
                    send(&v.model, cx, msg);
                },
            ));
        }
        for (i, slot) in [LimitSlot::Down, LimitSlot::Up, LimitSlot::Cap]
            .into_iter()
            .enumerate()
        {
            subs.push(defaults[i].subscribe(
                window,
                cx,
                move |v: &mut Self| v.defaults.get_mut(i),
                move |v, val, _, cx| send(&v.model, cx, Msg::Default(limit_field(slot, val))),
            ));
        }

        // Closing the window keeps the app running only with a tray icon.
        let m = model.clone();
        window.on_window_should_close(cx, move |_, cx| {
            if !m.read(cx).keeps_running_without_window() {
                cx.quit();
            }
            true
        });
        if !crate::FORCED_THEME.with(|t| t.get()) {
            theme::follow_system(window, cx);
            subs.push(cx.observe_window_appearance(window, |_, window, cx| {
                theme::follow_system(window, cx);
            }));
        }
        focus.focus(window, cx);

        let mut me = Self {
            model,
            focus,
            ssid,
            password,
            pw_revealed: false,
            totals,
            defaults,
            device: None,
            qr: qr::Qr::default(),
            last_run: RunState::Off,
            _subs: subs,
        };
        me.sync(window, cx);
        me
    }

    /// Brings the text fields in line with the model (without touching the
    /// one being edited) and opens or closes the device page.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (ssid, pw, run, totals, defaults, open) = {
            let s = &self.model.read(cx).state;
            (
                s.cfg.ssid.clone(),
                s.cfg.password.clone(),
                s.run,
                [s.total_down_kbps, s.total_up_kbps],
                [
                    s.defaults.limits.down_kbps,
                    s.defaults.limits.up_kbps,
                    s.defaults.limits.cap_mb,
                ],
                s.open_device.clone(),
            )
        };
        sync_text(&self.ssid, &ssid, window, cx);
        sync_text(&self.password, &pw, window, cx);
        for (li, v) in self.totals.iter_mut().zip(totals) {
            li.sync(v, window, cx);
        }
        for (li, v) in self.defaults.iter_mut().zip(defaults) {
            li.sync(v, window, cx);
        }
        // Hidden again every time the hotspot starts.
        if run == RunState::On && self.last_run != RunState::On {
            self.pw_revealed = false;
        }
        self.last_run = run;

        match (&open, &self.device) {
            (Some(mac), Some(d)) if *mac == d.mac => {}
            (Some(mac), _) => self.device = Some(self.device_form(mac.clone(), window, cx)),
            (None, Some(_)) => self.device = None,
            (None, None) => {}
        }
        self.sync_device(window, cx);
        cx.notify();
    }

    fn on_model_event(&mut self, ev: &ModelEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev {
            ModelEvent::Toast(text) => {
                window.push_notification(Notification::new().message(text.clone()), cx);
            }
            ModelEvent::Ask(ask) => dialogs::open(self.model.clone(), ask, window, cx),
            ModelEvent::ShowWindow => window.activate_window(),
        }
    }

    /// A click handler that sends a message.
    pub fn on<F: Fn() -> Msg + 'static>(
        &self,
        msg: F,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static + use<F> {
        let m = self.model.clone();
        move |_, _, cx| send(&m, cx, msg())
    }

    fn header(&self, s: &UiState, page: Page, narrow: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let demo = self.model.read(cx).is_demo();
        let m = self.model.clone();
        let menu = Button::new("main-menu")
            .icon(icon(Lucide::Menu))
            .ghost()
            .small()
            .tooltip(tr("Main menu"))
            .dropdown_menu_with_anchor(gpui_kit::Anchor::TopRight, move |menu, window, cx| {
                main_menu(menu, &m, demo, window, cx)
            });
        h_flex()
            .w_full()
            .h(px(46.))
            .px_2()
            .border_b_1()
            .border_color(t.border)
            .bg(widgets::card_bg(cx))
            .child(div().w(px(32.)))
            .child(
                h_flex()
                    .flex_1()
                    .justify_center()
                    .when(!narrow, |d| d.child(self.tabs(s, page, false, cx))),
            )
            .child(menu)
            .into_any_element()
    }

    /// Home / Settings / Clients, with a badge for devices waiting.
    fn tabs(&self, s: &UiState, page: Page, bottom: bool, cx: &mut Context<Self>) -> AnyElement {
        let t = cx.theme();
        let pending = s
            .clients
            .iter()
            .filter(|c| s.effective(c).access == Access::Pending)
            .count();
        let badge = (s.helper_ready() && s.run == RunState::On && pending > 0).then_some(pending);
        let disabled = s.problem.is_some();
        let tab = |id: &'static str, p: Page, ico: Lucide, label: &str, badge: Option<usize>| {
            let selected = p == page;
            let m = self.model.clone();
            let content = if bottom {
                v_flex()
                    .items_center()
                    .gap_0p5()
                    .child(icon(ico).small())
                    .child(div().text_xs().font_semibold().child(label.to_owned()))
            } else {
                h_flex()
                    .gap_1p5()
                    .child(icon(ico).small())
                    .child(div().text_sm().font_semibold().child(label.to_owned()))
            };
            div()
                .id(id)
                .relative()
                .px_3()
                .py_1()
                .rounded_md()
                .when(bottom, |d| d.flex_1().flex().justify_center())
                .when(selected, |d| d.bg(t.accent))
                .when(!selected && !disabled, |d| {
                    d.hover(|d| d.bg(t.accent.opacity(0.5)))
                })
                .when(disabled, |d| d.opacity(0.5))
                .cursor_pointer()
                .child(content)
                .children(badge.map(|n| {
                    div()
                        .absolute()
                        .top(px(-4.))
                        .right(px(-6.))
                        .min_w(px(16.))
                        .h(px(16.))
                        .px_1()
                        .rounded_full()
                        .bg(t.primary)
                        .text_color(t.primary_foreground)
                        .text_xs()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(n.to_string())
                }))
                .when(!disabled, |d| {
                    d.on_click(move |_, _, cx| send(&m, cx, Msg::Page(p)))
                })
        };
        h_flex()
            .gap_1()
            .when(bottom, |d| d.w_full())
            .child(tab("tab-home", Page::Home, Lucide::Wifi, tr("Home"), None))
            .child(tab(
                "tab-settings",
                Page::Settings,
                Lucide::Settings,
                tr("Settings"),
                None,
            ))
            .child(tab(
                "tab-clients",
                Page::Clients,
                Lucide::Users,
                tr("Clients"),
                badge,
            ))
            .into_any_element()
    }

    fn banners(&self, s: &UiState, page: Page, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let t = cx.theme();
        let mut out = Vec::new();
        let banner = |text: String, button: &str, msg: fn() -> Msg, color| {
            h_flex()
                .w_full()
                .px_4()
                .py_2()
                .gap_3()
                .bg(color)
                .child(div().flex_1().text_sm().child(text))
                .child(
                    Button::new(SharedString::from(format!("banner-{button}")))
                        .label(button.to_owned())
                        .small()
                        .on_click(self.on(msg)),
                )
                .into_any_element()
        };
        let on_helper_page = matches!(page, Page::Settings | Page::Clients);
        let helper = match s.helper {
            HelperStatus::Ready | HelperStatus::Unsupported => None,
            HelperStatus::Missing => Some((
                tr("Per-device speed limits need the Easy Hotspot helper"),
                tr("Set up…"),
            )),
            HelperStatus::Unauthorized => Some((
                tr("Allow Easy Hotspot to manage speed limits"),
                tr("Authorize"),
            )),
            HelperStatus::TooOld => Some((
                tr("Update the helper to use per-device control"),
                tr("Set up…"),
            )),
        };
        if let Some((text, button)) = helper
            && s.problem.is_none()
            && on_helper_page
            && s.open_device.is_none()
        {
            out.push(banner(text.into(), button, || Msg::HelperAction, t.accent));
        }
        if let Some((title, _)) = &s.error
            && page == Page::Home
        {
            out.push(banner(
                title.clone(),
                tr("Details"),
                || Msg::ErrorDetails,
                t.danger.opacity(0.18),
            ));
        }
        out
    }

    fn problem_page(&self, s: &UiState, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = s.problem?;
        let t = cx.theme();
        let (ico, title, desc, action) = dialogs::problem_text(p);
        let mut parts: Vec<String> = vec![desc.to_owned()];
        if p == crate::state::Problem::NoAdapter {
            parts.extend(s.adapter_problems.iter().map(|prob| prob.explain()));
        }
        parts.extend(s.problem_detail.clone());
        parts.retain(|p| !p.is_empty());
        let desc = parts.join("\n\n");
        Some(
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_4()
                .p_8()
                .child(icon(ico).size(px(64.)).text_color(t.muted_foreground))
                .child(div().text_xl().font_semibold().text_center().child(title))
                .child(
                    div()
                        .max_w(px(420.))
                        .text_center()
                        .text_color(t.muted_foreground)
                        .child(desc),
                )
                .child(
                    Button::new("problem-action")
                        .label(action)
                        .primary()
                        .rounded(px(999.))
                        .on_click(self.on(|| Msg::ProblemAction)),
                )
                .into_any_element(),
        )
    }
}

fn limit_field(slot: LimitSlot, v: Option<u32>) -> LimitField {
    match slot {
        LimitSlot::Down => LimitField::Down(v),
        LimitSlot::Up => LimitField::Up(v),
        LimitSlot::Cap => LimitField::Cap(v),
    }
}

fn sync_text(input: &Entity<InputState>, text: &str, window: &mut Window, cx: &mut App) {
    let s = input.read(cx);
    if s.value() != text && !s.focus_handle(cx).is_focused(window) {
        input.update(cx, |s, cx| s.set_value(text.to_owned(), window, cx));
    }
}

fn main_menu(
    mut menu: PopupMenu,
    model: &Entity<Model>,
    demo: bool,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    use crate::state::demo::Scenario;
    if demo {
        let m = model.clone();
        menu = menu
            .submenu(tr("Demo scenarios"), window, cx, move |mut sub, _, _| {
                for (sc, _, label) in Scenario::ALL {
                    let m = m.clone();
                    sub = sub.item(
                        PopupMenuItem::new(tr(label))
                            .on_click(move |_, _, cx| send(&m, cx, Msg::Scenario(sc))),
                    );
                }
                sub
            })
            .separator();
    }
    let m = model.clone();
    let m2 = model.clone();
    menu.item(
        PopupMenuItem::new(tr("Keyboard Shortcuts"))
            .on_click(move |_, window, cx| dialogs::shortcuts(window, cx)),
    )
    .item(
        PopupMenuItem::new(tr("About Easy Hotspot"))
            .on_click(move |_, _, cx| send(&m, cx, Msg::About)),
    )
    .separator()
    .item(PopupMenuItem::new(tr("Quit")).on_click(move |_, _, cx| send(&m2, cx, Msg::Quit)))
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = window.viewport_size().width;
        let narrow = width < px(NARROW);
        let wide = width >= px(WIDE);
        let (s, page) = {
            let m = self.model.read(cx);
            (m.state.clone(), m.page)
        };
        let t = cx.theme();
        let (bg, fg) = (widgets::page_bg(cx), t.foreground);

        let content: AnyElement = if let Some(p) = self.problem_page(&s, cx) {
            p
        } else if s.open_device.is_some() {
            self.device_page(&s, window, cx)
        } else {
            match page {
                Page::Home => self.home(&s, wide, cx),
                Page::Settings => self.settings(&s, cx),
                Page::Clients => self.clients(&s, cx),
            }
        };

        let m1 = self.model.clone();
        let m2 = self.model.clone();
        let m3 = self.model.clone();
        let m4 = self.model.clone();
        v_flex()
            .id("root")
            .key_context("EasyHotspot")
            .track_focus(&self.focus)
            .on_action(move |_: &ToggleHotspot, _, cx| send(&m1, cx, Msg::Toggle))
            .on_action(move |_: &CopyPassword, _, cx| send(&m2, cx, Msg::CopyPassword))
            .on_action(move |_: &OpenSettings, _, cx| send(&m3, cx, Msg::Page(Page::Settings)))
            .on_action(move |_: &Quit, _, cx| send(&m4, cx, Msg::Quit))
            .size_full()
            .bg(bg)
            .text_color(fg)
            .text_sm()
            .child(self.header(&s, page, narrow, cx))
            .children(self.banners(&s, page, cx))
            .child(
                div()
                    .id("content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(content),
            )
            .when(narrow && s.problem.is_none(), |d| {
                d.child(
                    div()
                        .w_full()
                        .px_2()
                        .py_1()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .bg(widgets::card_bg(cx))
                        .child(self.tabs(&s, page, true, cx)),
                )
            })
    }
}

/// Centers page content at a readable width with side gutters.
pub fn clamp(max: f32, content: impl IntoElement) -> AnyElement {
    h_flex()
        .w_full()
        .justify_center()
        .px_4()
        .py_6()
        .child(div().w_full().max_w(px(max)).child(content))
        .into_any_element()
}

/// Set from `--dark` / `--light`: don't follow the system appearance.
pub fn force_theme(dark: bool, cx: &mut App) {
    crate::FORCED_THEME.with(|t| t.set(true));
    theme::force(dark, cx);
}
