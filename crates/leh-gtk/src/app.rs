//! Root component: window, header, banners, status pages, menus, shortcuts.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::i18n::{tr, trf};
use std::time::Duration;

use leh_model::policy::{Access, DevicePolicy};
use leh_model::{Band, Mode, password};
use relm4::adw::prelude::*;
use relm4::gtk::{gio, glib};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, adw, gtk};

use crate::live::{self, Live};
use crate::pages::{
    clients::ClientsPage, device::DeviceDialog, home::HomePage, settings::SettingsPage,
};
use crate::state::demo::{self, Scenario};
use crate::state::{HelperStatus, Problem, RunState, UiState};
use crate::widgets::LimitField;

pub struct Args {
    pub demo: bool,
    /// Demo/screenshot helpers.
    pub page: Option<String>,
    pub running: bool,
    pub dark: bool,
    pub light: bool,
    pub scenario: Option<String>,
    pub device: Option<String>,
    /// Render the window to this PNG after startup, then quit.
    pub screenshot: Option<String>,
    pub size: Option<(i32, i32)>,
    /// Started at login: stay in the background (tray), maybe start the hotspot.
    pub hidden: bool,
}

#[derive(Debug)]
pub enum Msg {
    Toggle,
    DemoStarted,
    DemoStopped,
    Ssid(String),
    Password(String),
    RegeneratePassword,
    CopyPassword,
    Mode(Mode),
    BridgeConfirmed(bool),
    Adapter(Option<String>),
    Upstream(Option<String>),
    Band(Band),
    Channel(u32),
    Hidden(bool),
    TotalDown(Option<u32>),
    TotalUp(Option<u32>),
    Approval(bool),
    Default(LimitField),
    Autostart(bool),
    StartAtLogin(bool),
    Tray(bool),
    ExportDiagnostics,
    Allow(String),
    Block(String),
    OpenDevice(String),
    DeviceClosed,
    DeviceAlias(String, String),
    DeviceAccess(String, Access),
    DeviceFollow(String, bool),
    DeviceLimit(String, LimitField),
    ForgetDevice(String),
    HelperAction,
    HelperCheck,
    ErrorDetails,
    ProblemAction,
    Scenario(Scenario),
    Tick,
    Page(String),
    GoTo(&'static str),
    About,
    /// Live mode: new controller snapshot.
    Snapshot(Box<leh_nm::Snapshot>),
    /// Live mode: connected clients changed.
    Clients(Vec<leh_model::clients::Client>),
    /// Live mode: helper probe result.
    Helper(HelperStatus),
    /// Live mode: install the helper via pkexec.
    InstallHelper,
    InstallProgress(String),
    InstallDone(Result<(), String>),
    CopyText(String),
    /// Live mode: debounced shaping update is due.
    ShapeFlush(u64),
    /// Live mode: helper answered an apply (true) or clear (false).
    ShapeResult(bool, Result<(), leh_nm::shaper_client::ShaperError>),
    /// Live mode: per-device byte counters.
    Counters(std::collections::HashMap<String, (u64, u64)>),
    /// Live mode: the hotspot interface's (rx, tx) byte counters.
    IfaceBytes(u64, u64),
    ShowWindow,
    Quit,
    TrayStarted(Box<crate::tray::TrayHandle>),
    /// The desktop has no tray (or it went away).
    TrayGone,
    TrayReady,
    Language(leh_model::Language),
    Restart,
}

pub struct App {
    state: UiState,
    page: String,
    next_start_fails: bool,
    window: adw::ApplicationWindow,
    toasts: adw::ToastOverlay,
    device: RefCell<Option<DeviceDialog>>,
    view_stack: adw::ViewStack,
    /// `None` in `--demo` mode.
    live: Option<Live>,
    tray: Option<Box<crate::tray::TrayHandle>>,
    /// Closing the window only hides it (a tray icon is showing).
    hide_on_close: Rc<Cell<bool>>,
    /// Started at login with "Start hotspot at login": start once NM is ready.
    start_pending: bool,
}

pub struct Widgets {
    home: HomePage,
    settings: SettingsPage,
    clients: ClientsPage,
    content: gtk::Stack,
    problem: adw::StatusPage,
    switcher: adw::ViewSwitcher,
    switcher_bar: adw::ViewSwitcherBar,
    helper_banner: adw::Banner,
    error_banner: adw::Banner,
}

fn toast(overlay: &adw::ToastOverlay, text: &str) {
    overlay.add_toast(adw::Toast::builder().title(text).timeout(3).build());
}

impl App {
    fn toast(&self, text: &str) {
        toast(&self.toasts, text);
    }

    fn device_store(&self) -> crate::config_store::DeviceStore {
        crate::config_store::DeviceStore {
            new_devices: self.state.defaults,
            total_down_kbps: self.state.total_down_kbps,
            total_up_kbps: self.state.total_up_kbps,
            devices: self.state.policies.clone(),
            ..Default::default()
        }
    }

    fn policy_mut(&mut self, mac: &str) -> &mut DevicePolicy {
        let defaults = self.state.defaults;
        self.state
            .policies
            .entry(mac.to_owned())
            .or_insert_with(|| DevicePolicy::new_device(&defaults))
    }

    fn start(&mut self, sender: &ComponentSender<Self>) {
        if let Err(errs) = self.state.cfg.validate() {
            let msg = crate::i18n::tr_dyn(&errs[0].to_string());
            let mut c = msg.chars();
            let msg = c
                .next()
                .map_or(String::new(), |f| f.to_uppercase().chain(c).collect());
            self.toast(&msg);
            return;
        }
        self.state.error = None;
        if let Some(live) = &mut self.live {
            crate::config_store::save(&self.state.cfg);
            live.start_hotspot(&self.state.cfg);
            return;
        }
        self.state.run = RunState::Starting;
        let s = sender.clone();
        glib::timeout_add_local_once(Duration::from_millis(1500), move || {
            s.input(Msg::DemoStarted)
        });
    }

    fn show_bridge_dialog(&self, sender: &ComponentSender<Self>) {
        let d = adw::AlertDialog::new(
            Some(tr("Switch to Bridge mode?")),
            Some(tr(
                "Devices will join your wired network and get addresses from your router. Your wired connection drops for a few seconds while the bridge is set up.",
            )),
        );
        d.add_response("cancel", tr("Cancel"));
        d.add_response("bridge", tr("Use Bridge mode"));
        d.set_response_appearance("bridge", adw::ResponseAppearance::Suggested);
        let s = sender.clone();
        d.connect_response(None, move |_, r| {
            s.input(Msg::BridgeConfirmed(r == "bridge"))
        });
        d.present(Some(&self.window));
    }

    fn apply_scenario(&mut self, sc: Scenario) {
        let st = &mut self.state;
        match sc {
            Scenario::Normal => {
                let run = st.run;
                *st = demo::initial();
                st.run = run;
                self.next_start_fails = false;
            }
            Scenario::StartFails => {
                self.next_start_fails = true;
                self.toast(tr("The next start will fail"));
            }
            Scenario::NoAdapter => st.problem = Some(Problem::NoAdapter),
            Scenario::NmMissing => st.problem = Some(Problem::NmMissing),
            Scenario::BridgeNoEthernet => {
                st.cfg.mode = Mode::Bridge;
                st.problem = Some(Problem::BridgeNoEthernet);
            }
            Scenario::AdapterGone => {
                st.run = RunState::Off;
                st.problem = Some(Problem::AdapterGone);
            }
            Scenario::HelperReady => st.helper = HelperStatus::Ready,
            Scenario::HelperMissing => st.helper = HelperStatus::Missing,
            Scenario::HelperUnauthorized => st.helper = HelperStatus::Unauthorized,
            Scenario::HelperTooOld => st.helper = HelperStatus::TooOld,
            Scenario::NoDevices => st.clients.clear(),
            Scenario::NewDevice => {
                let c = demo::new_device();
                if st.client(&c.mac).is_none() {
                    let name = c.hostname.clone().unwrap_or_default();
                    st.policies.remove(&c.mac);
                    st.clients.push(c);
                    let msg = if st.defaults.require_approval {
                        trf("“{name}” wants to join — see Clients", &[("name", &name)])
                    } else {
                        trf(
                            "“{name}” joined with the default settings",
                            &[("name", &name)],
                        )
                    };
                    self.toast(&msg);
                }
            }
        }
    }

    fn helper_dialog(&self, sender: &ComponentSender<Self>) {
        if self.live.is_some() {
            return self.live_helper_dialog(sender);
        }
        let (heading, body) = match self.state.helper {
            HelperStatus::Missing => (
                tr("Install the helper"),
                tr(
                    "Per-device speed limits, approval and data caps need a small helper that runs with administrator rights.\n\nOn Linux Mint, run:\n\nsudo apt install linuxeasyhotspot",
                ),
            ),
            HelperStatus::TooOld => (
                tr("Update the helper"),
                tr(
                    "Your Linux Easy Hotspot helper is older than this app. Update it with your system's software updater, or run:\n\nsudo apt install --only-upgrade linuxeasyhotspot",
                ),
            ),
            _ => (tr("Helper ready"), tr("Per-device control is available.")),
        };
        let d = adw::AlertDialog::new(Some(heading), Some(body));
        d.set_body_use_markup(false);
        d.add_response("close", tr("Close"));
        d.add_response("check", tr("Check again"));
        d.set_response_appearance("check", adw::ResponseAppearance::Suggested);
        let s = sender.clone();
        d.connect_response(Some("check"), move |_, _| s.input(Msg::HelperCheck));
        d.present(Some(&self.window));
    }

    /// Live mode: one-click install, plus a command to copy.
    fn live_helper_dialog(&self, sender: &ComponentSender<Self>) {
        let update = self.state.helper == HelperStatus::TooOld;
        let can_install = live::source_root().is_some() && !live::no_new_privs();
        let body = if update {
            tr("The installed helper is older than this app and needs an update.")
        } else {
            tr(
                "Speed limits, approval of new devices and data caps need a small helper that runs with administrator rights. Everything else works without it.",
            )
        };
        let d = adw::AlertDialog::new(
            Some(if update {
                tr("Update the helper")
            } else {
                tr("Install the helper")
            }),
            Some(body),
        );

        let cmd = live::install_command();
        let code = gtk::Label::builder()
            .label(&cmd)
            .selectable(true)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::Char)
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["monospace"])
            // Selectable with the mouse, but don't grab focus (and select
            // everything) when the dialog opens.
            .focusable(false)
            .build();
        // Wrap long paths without inserting misleading hyphens.
        let attrs = gtk::pango::AttrList::new();
        attrs.insert(gtk::pango::AttrInt::new_insert_hyphens(false));
        code.set_attributes(Some(&attrs));
        let copy = gtk::Button::builder()
            .icon_name("edit-copy-symbolic")
            .tooltip_text(tr("Copy command"))
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        copy.update_property(&[gtk::accessible::Property::Label(tr("Copy command"))]);
        let s = sender.clone();
        copy.connect_clicked(move |_| s.input(Msg::CopyText(cmd.clone())));
        let code_box = gtk::Box::builder()
            .spacing(8)
            .css_classes(["card", "code-box"])
            .build();
        code_box.append(&code);
        code_box.append(&copy);
        let hint = gtk::Label::builder()
            .label(if can_install {
                tr("Or run this in a terminal:")
            } else if live::no_new_privs() {
                tr("This app was started from a terminal that can't ask for administrator rights. Run this in a normal terminal:")
            } else {
                tr("Run this in a terminal:")
            })
            .wrap(true)
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .build();
        let extra = gtk::Box::new(gtk::Orientation::Vertical, 8);
        extra.append(&hint);
        extra.append(&code_box);
        d.set_extra_child(Some(&extra));

        d.add_response("close", tr("Close"));
        if can_install {
            d.add_response(
                "install",
                if update {
                    tr("Update now")
                } else {
                    tr("Install now")
                },
            );
            d.set_response_appearance("install", adw::ResponseAppearance::Suggested);
            d.set_default_response(Some("install"));
            let s = sender.clone();
            d.connect_response(Some("install"), move |_, _| s.input(Msg::InstallHelper));
        } else {
            d.add_response("check", tr("Check again"));
            let s = sender.clone();
            d.connect_response(Some("check"), move |_, _| s.input(Msg::HelperCheck));
        }
        d.present(Some(&self.window));
    }
}

fn status_page_for(p: Problem) -> (&'static str, &'static str, &'static str, &'static str) {
    match p {
        Problem::NmMissing => (
            "network-error-symbolic",
            tr("NetworkManager is required"),
            tr(
                "Linux Easy Hotspot creates hotspots through NetworkManager 1.30 or newer, which isn't running on this system.",
            ),
            tr("Learn more"),
        ),
        Problem::NoAdapter => (
            "network-wireless-disabled-symbolic",
            tr("Your Wi-Fi card can't create hotspots"),
            tr(
                "None of your Wi-Fi adapters support access-point mode. A USB Wi-Fi adapter that supports AP mode will work.",
            ),
            tr("Refresh"),
        ),
        Problem::BridgeNoEthernet => (
            "network-wired-disconnected-symbolic",
            tr("Plug in an Ethernet cable"),
            tr(
                "Bridge mode connects your devices to a wired network. Connect a cable, or share your internet in Router mode instead.",
            ),
            tr("Use Router mode"),
        ),
        Problem::AdapterGone => (
            "network-wireless-offline-symbolic",
            tr("Wi-Fi adapter unavailable"),
            tr("The Wi-Fi adapter was turned off or unplugged."),
            tr("Retry"),
        ),
    }
}

fn save_screenshot(window: &adw::ApplicationWindow, path: &str) {
    let Some(content) = window.content() else {
        return;
    };
    let (w, h) = (content.width(), content.height());
    let paintable = gtk::WidgetPaintable::new(Some(&content));
    let snapshot = gtk::Snapshot::new();
    paintable.snapshot(&snapshot, f64::from(w), f64::from(h));
    let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer()) else {
        eprintln!(
            "screenshot: nothing to render ({w}x{h}, mapped={})",
            window.is_mapped()
        );
        return;
    };
    let tex = renderer.render_texture(&node, None);
    if let Err(e) = tex.save_to_png(path) {
        eprintln!("screenshot: {e}");
    }
}

fn build_menu(demo: bool) -> gio::Menu {
    let menu = gio::Menu::new();
    if demo {
        let demo_menu = gio::Menu::new();
        for (_, id, label) in Scenario::ALL {
            demo_menu.append(Some(label), Some(&format!("win.demo::{id}")));
        }
        menu.append_submenu(Some(tr("Demo scenarios")), &demo_menu);
    }
    let rest = gio::Menu::new();
    rest.append(Some(tr("Keyboard Shortcuts")), Some("win.shortcuts"));
    rest.append(Some(tr("About Linux Easy Hotspot")), Some("win.about"));
    menu.append_section(None, &rest);
    menu
}

fn shortcuts_window(parent: &adw::ApplicationWindow) {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let ui = r#"<interface>
  <object class="GtkShortcutsWindow" id="w">
    <property name="modal">1</property>
    <child><object class="GtkShortcutsSection"><property name="section-name">main</property>
      <child><object class="GtkShortcutsGroup"><property name="title">@GENERAL@</property>
        <child><object class="GtkShortcutsShortcut"><property name="title">@TOGGLE@</property><property name="accelerator">&lt;Ctrl&gt;space</property></object></child>
        <child><object class="GtkShortcutsShortcut"><property name="title">@COPY@</property><property name="accelerator">&lt;Ctrl&gt;&lt;Shift&gt;c</property></object></child>
        <child><object class="GtkShortcutsShortcut"><property name="title">@SETTINGS@</property><property name="accelerator">&lt;Ctrl&gt;comma</property></object></child>
        <child><object class="GtkShortcutsShortcut"><property name="title">@QUIT@</property><property name="accelerator">&lt;Ctrl&gt;q</property></object></child>
      </object></child>
    </object></child>
  </object>
</interface>"#;
    let ui = ui
        .replace("@GENERAL@", &esc(tr("General")))
        .replace("@TOGGLE@", &esc(tr("Turn hotspot on or off")))
        .replace("@COPY@", &esc(tr("Copy password")))
        .replace("@SETTINGS@", &esc(tr("Settings")))
        .replace("@QUIT@", &esc(tr("Quit")));
    let b = gtk::Builder::from_string(&ui);
    if let Some(w) = b.object::<gtk::ShortcutsWindow>("w") {
        w.set_transient_for(Some(parent));
        w.present();
    }
}

fn tray_state(s: &UiState) -> crate::tray::TrayState {
    let on = s.run == RunState::On;
    crate::tray::TrayState {
        running: on,
        busy: matches!(s.run, RunState::Starting | RunState::Stopping),
        devices: if on { s.connected_count() } else { 0 },
        pending: if on && s.helper_ready() {
            s.clients
                .iter()
                .filter(|c| s.effective(c).access == Access::Pending)
                .map(|c| (c.mac.clone(), s.display_name(c)))
                .collect()
        } else {
            Vec::new()
        },
    }
}

impl App {
    fn ask_restart(&self, sender: &ComponentSender<Self>) {
        let d = adw::AlertDialog::new(
            Some(tr("Restart to change the language")),
            Some(tr(
                "Linux Easy Hotspot needs to restart to show the new language. A running hotspot keeps working.",
            )),
        );
        d.add_response("later", tr("Later"));
        d.add_response("restart", tr("Restart now"));
        d.set_response_appearance("restart", adw::ResponseAppearance::Suggested);
        let s = sender.clone();
        d.connect_response(Some("restart"), move |_, _| s.input(Msg::Restart));
        d.present(Some(&self.window));
    }

    fn handle(&mut self, msg: Msg, sender: &ComponentSender<Self>) {
        let sender = sender.clone();
        let st = &mut self.state;
        match msg {
            Msg::Toggle => match st.run {
                RunState::Off => self.start(&sender),
                RunState::On if self.live.is_some() => {
                    if let Some(l) = &self.live {
                        l.controller.stop();
                    }
                }
                RunState::On => {
                    st.run = RunState::Stopping;
                    let s = sender.clone();
                    glib::timeout_add_local_once(Duration::from_millis(600), move || {
                        s.input(Msg::DemoStopped)
                    });
                }
                _ => {}
            },
            Msg::DemoStarted => {
                if self.next_start_fails {
                    self.next_start_fails = false;
                    st.run = RunState::Off;
                    let why = leh_model::reasons::device(9);
                    st.error = Some((
                        tr("Couldn't start the hotspot").into(),
                        crate::i18n::tr_dyn(&why.message),
                    ));
                } else if st.run == RunState::Starting {
                    st.run = RunState::On;
                }
            }
            Msg::DemoStopped => {
                st.run = RunState::Off;
                self.toast(tr("Hotspot stopped"));
            }
            Msg::Ssid(s) => st.cfg.ssid = s,
            Msg::Password(p) => st.cfg.password = p,
            Msg::RegeneratePassword => {
                st.cfg.password = password::generate();
                self.toast(tr("New password generated"));
            }
            Msg::CopyPassword => {
                let pw = st.cfg.password.clone();
                self.window.clipboard().set_text(&pw);
                self.toast(tr("Password copied"));
            }
            Msg::Mode(Mode::Bridge) if st.cfg.mode != Mode::Bridge => {
                // Re-render so the selector snaps back until confirmed.
                self.show_bridge_dialog(&sender);
            }
            Msg::Mode(Mode::LocalOnly) if !st.local_only_supported => {
                let v = st.nm_version.clone();
                self.toast(&trf(
                    "Local only needs NetworkManager 1.52 or newer (you have {version})",
                    &[("version", &v)],
                ));
            }
            Msg::Mode(m) => st.cfg.mode = m,
            Msg::BridgeConfirmed(true) => {
                st.cfg.mode = Mode::Bridge;
                // Only wired sources can be bridged.
                let wired = st
                    .cfg
                    .upstream_iface
                    .as_deref()
                    .and_then(|i| st.uplink(i))
                    .is_some_and(|u| u.can_bridge());
                if !wired {
                    st.cfg.upstream_iface = st
                        .uplinks
                        .iter()
                        .find(|u| u.can_bridge())
                        .map(|u| u.iface.clone());
                }
                if st.cfg.upstream_iface.is_none() {
                    st.problem = Some(Problem::BridgeNoEthernet);
                }
            }
            Msg::BridgeConfirmed(false) => {}
            Msg::Adapter(a) => st.cfg.ap_iface = a,
            Msg::Upstream(u) => {
                if st.cfg.mode == Mode::Bridge && u.is_none() {
                    // Bridge always needs an explicit Ethernet device.
                } else {
                    st.cfg.upstream_iface = u;
                }
            }
            Msg::Band(Band::Ghz5) if !st.ap_supports_5ghz() => {
                self.toast(tr("This adapter doesn't support 5 GHz"));
            }
            Msg::Band(b) => {
                if st.cfg.band != b {
                    st.cfg.band = b;
                    st.cfg.channel = 0;
                }
            }
            Msg::Channel(c) => st.cfg.channel = c,
            Msg::Hidden(h) => st.cfg.hidden = h,
            Msg::TotalDown(v) => st.total_down_kbps = v,
            Msg::TotalUp(v) => st.total_up_kbps = v,
            Msg::Approval(a) => st.defaults.require_approval = a,
            Msg::Default(f) => f.apply(&mut st.defaults.limits),
            Msg::Autostart(a) => {
                st.cfg.autostart = a;
                if !a {
                    st.cfg.start_at_login = false;
                }
                if self.live.is_some() {
                    crate::autostart::set(a);
                }
            }
            Msg::StartAtLogin(a) => st.cfg.start_at_login = a,
            Msg::Tray(t) => {
                st.cfg.show_tray = t;
                if !t {
                    self.tray = None;
                } else if self.tray.is_none() && self.live.is_some() {
                    crate::tray::spawn(sender.input_sender().clone());
                }
            }
            Msg::ExportDiagnostics if self.live.is_some() => {
                self.toast(tr("Diagnostics export is coming in a later version"));
            }
            Msg::ExportDiagnostics => {
                self.toast(tr("Demo: diagnostics would be saved to a .txt file"))
            }
            Msg::Allow(mac) => {
                self.policy_mut(&mac).access = Access::Allowed;
                self.toast(tr("Device allowed"));
            }
            Msg::Block(mac) => {
                self.policy_mut(&mac).access = Access::Blocked;
                self.toast(tr("Device blocked"));
            }
            Msg::OpenDevice(mac) => {
                let d = DeviceDialog::new(sender.input_sender().clone(), &mac);
                d.update(&self.state);
                d.dialog.present(Some(&self.window));
                // Don't open with the name field focused and selected.
                let scroll = d.scroll.clone();
                glib::timeout_add_local_once(Duration::from_millis(60), move || {
                    scroll.grab_focus();
                });
                *self.device.borrow_mut() = Some(d);
                self.state.open_device = Some(mac);
            }
            Msg::DeviceClosed => {
                self.device.borrow_mut().take();
                st.open_device = None;
            }
            Msg::DeviceAlias(mac, name) => {
                let name = name.trim().to_owned();
                self.policy_mut(&mac).alias = (!name.is_empty()).then_some(name);
            }
            Msg::DeviceAccess(mac, a) => self.policy_mut(&mac).access = a,
            Msg::DeviceFollow(mac, f) => {
                let defaults = self.state.defaults.limits;
                let p = self.policy_mut(&mac);
                if p.follow_defaults && !f {
                    // Start the override from the current defaults.
                    p.limits = defaults;
                }
                p.follow_defaults = f;
            }
            Msg::DeviceLimit(mac, f) => f.apply(&mut self.policy_mut(&mac).limits),
            Msg::ForgetDevice(mac) => {
                st.policies.remove(&mac);
                st.clients.retain(|c| c.mac != mac);
                if let Some(d) = self.device.borrow_mut().take() {
                    d.dialog.close();
                }
                self.toast(tr("Device forgotten"));
            }
            Msg::HelperAction if self.live.is_some() && st.helper == HelperStatus::Unauthorized => {
                // Try again; polkit shows its dialog if needed.
                st.helper = HelperStatus::Ready;
                if let Some(live) = &mut self.live {
                    live.reset_shaping();
                }
            }
            Msg::HelperAction => match st.helper {
                HelperStatus::Unauthorized => {
                    st.helper = HelperStatus::Ready;
                    self.toast(tr("Demo: the system would ask for your password here"));
                }
                _ => self.helper_dialog(&sender),
            },
            Msg::HelperCheck if self.live.is_some() => {
                if let Some(live) = &self.live {
                    live.probe_helper();
                }
            }
            Msg::HelperCheck => {
                st.helper = HelperStatus::Ready;
                self.toast(tr("Helper found — per-device control enabled"));
            }
            Msg::ErrorDetails => {
                if let Some((title, details)) = st.error.clone() {
                    let d = adw::AlertDialog::new(Some(&title), Some(&details));
                    d.add_response("close", tr("Close"));
                    d.add_response("retry", tr("Try again"));
                    d.set_response_appearance("retry", adw::ResponseAppearance::Suggested);
                    let s = sender.clone();
                    d.connect_response(Some("retry"), move |_, _| s.input(Msg::Toggle));
                    d.present(Some(&self.window));
                    self.state.error = None;
                    if let Some(live) = &mut self.live {
                        live.dismiss_error();
                    }
                }
            }
            Msg::ProblemAction if self.live.is_some() => {
                let live = self.live.as_mut().expect("checked");
                match st.problem {
                    Some(Problem::NmMissing) => {
                        gtk::UriLauncher::new("https://networkmanager.dev/").launch(
                            Some(&self.window),
                            gio::Cancellable::NONE,
                            |_| {},
                        );
                        live.controller.refresh();
                    }
                    Some(Problem::BridgeNoEthernet) => {
                        st.cfg.mode = Mode::Router;
                        st.problem = None;
                        live.dismiss_error();
                    }
                    Some(Problem::AdapterGone) => {
                        st.problem = None;
                        live.controller.refresh();
                        live.start_hotspot(&st.cfg);
                    }
                    _ => live.controller.refresh(),
                }
            }
            Msg::ProblemAction => match st.problem.take() {
                Some(Problem::NmMissing) => {
                    st.problem = Some(Problem::NmMissing);
                    gtk::UriLauncher::new("https://networkmanager.dev/").launch(
                        Some(&self.window),
                        gio::Cancellable::NONE,
                        |_| {},
                    );
                }
                Some(Problem::BridgeNoEthernet) => st.cfg.mode = Mode::Router,
                Some(Problem::AdapterGone) => self.start(&sender),
                _ => {}
            },
            Msg::Scenario(sc) => self.apply_scenario(sc),
            Msg::Tick => {
                if st.run != RunState::On {
                    return;
                }
                demo::tick(st);
            }
            Msg::Snapshot(snap) => {
                if let Some(live) = &mut self.live {
                    live.apply(st, *snap);
                }
                if self.start_pending && st.problem.is_none() && !st.adapters.is_empty() {
                    self.start_pending = false;
                    if st.run == RunState::Off {
                        self.start(&sender);
                    }
                }
            }
            Msg::ShowWindow => self.window.present(),
            Msg::Quit => relm4::main_application().quit(),
            Msg::TrayStarted(h) => {
                st.tray_available = true;
                if st.cfg.show_tray {
                    self.tray = Some(h);
                }
            }
            Msg::TrayReady => st.tray_available = true,
            Msg::TrayGone => {
                st.tray_available = false;
                self.tray = None;
                // Without a tray there'd be no way back to a hidden window.
                if !self.window.is_visible() && !self.start_pending && st.run == RunState::Off {
                    self.window.present();
                }
            }
            Msg::Language(l) => {
                if l != st.cfg.language {
                    st.cfg.language = l;
                    self.ask_restart(&sender);
                }
            }
            Msg::Restart => {
                crate::RESTART.store(true, std::sync::atomic::Ordering::Relaxed);
                relm4::main_application().quit();
            }
            Msg::Clients(list) => {
                if self.live.is_some() {
                    for name in live::merge_clients(st, &list) {
                        self.toast(&trf(
                            "“{name}” wants to join — see Clients",
                            &[("name", &name)],
                        ));
                    }
                }
            }
            Msg::InstallHelper => {
                if let Some(live) = &self.live {
                    live.install_helper();
                }
            }
            Msg::InstallProgress(text) => self.toast(&text),
            Msg::InstallDone(Ok(())) => {
                self.toast(tr("Helper installed"));
                if let Some(live) = &self.live {
                    live.watch_helper();
                }
            }
            Msg::InstallDone(Err(e)) if e == "cancelled" => {
                self.toast(tr("Installation cancelled"))
            }
            Msg::InstallDone(Err(e)) => {
                let d = adw::AlertDialog::new(Some(tr("Couldn't install the helper")), Some(&e));
                d.add_response("close", tr("Close"));
                d.present(Some(&self.window));
            }
            Msg::CopyText(text) => {
                self.window.clipboard().set_text(&text);
                self.toast(tr("Copied to the clipboard"));
            }
            Msg::Helper(h) => {
                let was = st.helper;
                st.helper = h;
                if h == HelperStatus::Ready && was != HelperStatus::Ready && self.page != "home" {
                    self.toast(tr("Helper found: per-device control is on"));
                }
            }
            Msg::ShapeFlush(g) => {
                if let Some(live) = &mut self.live {
                    live.flush_shaping(g);
                }
            }
            Msg::ShapeResult(applied, res) => {
                if let Some(live) = &mut self.live
                    && let Some(msg) = live.on_shape_result(st, applied, res)
                {
                    self.toast(&msg);
                }
            }
            Msg::Counters(map) => {
                if let Some(live) = &mut self.live {
                    live.on_counters(st, map);
                }
            }
            Msg::IfaceBytes(rx, tx) => {
                if let Some(live) = &mut self.live {
                    live.on_iface_bytes(st, rx, tx);
                }
            }
            Msg::Page(p) => {
                // Fresh connection states when looking at Settings.
                if p == "settings"
                    && let Some(live) = &self.live
                {
                    live.controller.refresh();
                }
                self.page = p;
            }
            Msg::GoTo(p) => self.view_stack.set_visible_child_name(p),
            Msg::About => {
                let helper = match self.state.helper {
                    HelperStatus::Ready => tr("Helper: installed"),
                    HelperStatus::Missing => tr("Helper: not installed"),
                    HelperStatus::Unauthorized => tr("Helper: installed, not authorized"),
                    HelperStatus::TooOld => tr("Helper: needs an update"),
                };
                let about = adw::AboutDialog::builder()
                    .application_name(leh_model::APP_NAME)
                    .application_icon(crate::APP_ID)
                    .developer_name(tr("Linux Easy Hotspot contributors"))
                    .version(env!("CARGO_PKG_VERSION"))
                    .comments(format!(
                        "{}\n\n{helper}",
                        tr("The simplest way to share a Wi-Fi hotspot.")
                    ))
                    .license_type(gtk::License::Gpl30)
                    .build();
                about.present(Some(&self.window));
            }
        }
    }
}

impl SimpleComponent for App {
    type Init = Args;
    type Input = Msg;
    type Output = ();
    type Root = adw::ApplicationWindow;
    type Widgets = Widgets;

    fn init_root() -> Self::Root {
        adw::ApplicationWindow::builder()
            .title(leh_model::APP_NAME)
            .default_width(420)
            .default_height(640)
            .width_request(360)
            .height_request(294)
            .build()
    }

    fn init(args: Args, window: Self::Root, sender: ComponentSender<Self>) -> ComponentParts<Self> {
        let tx = sender.input_sender().clone();
        // App icons: installed ones come from the system theme; when running
        // from the source tree, also look in data/icons.
        if let Some(display) = gtk::gdk::Display::default() {
            let dev_icons = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons");
            if std::path::Path::new(dev_icons).exists() {
                gtk::IconTheme::for_display(&display).add_search_path(dev_icons);
            }
        }
        gtk::Window::set_default_icon_name(crate::APP_ID);

        let home = HomePage::new(tx.clone());
        let settings = SettingsPage::new(tx.clone());
        let clients = ClientsPage::new(tx.clone());

        let view_stack = adw::ViewStack::new();
        view_stack.add_titled_with_icon(&home.root, Some("home"), tr("Home"), crate::ICON_SYMBOLIC);
        view_stack.add_titled_with_icon(
            &settings.root,
            Some("settings"),
            tr("Settings"),
            "emblem-system-symbolic",
        );
        view_stack.add_titled_with_icon(
            &clients.root,
            Some("clients"),
            tr("Clients"),
            "system-users-symbolic",
        );
        let s = sender.clone();
        view_stack.connect_visible_child_name_notify(move |v| {
            s.input(Msg::Page(
                v.visible_child_name()
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
            ));
        });

        let switcher = adw::ViewSwitcher::builder()
            .stack(&view_stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build();
        let header = adw::HeaderBar::builder().title_widget(&switcher).build();
        let menu_btn = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&build_menu(args.demo))
            .tooltip_text(tr("Main menu"))
            .primary(true)
            .build();
        header.pack_end(&menu_btn);

        let helper_banner = adw::Banner::new("");
        let s = sender.clone();
        helper_banner.connect_button_clicked(move |_| s.input(Msg::HelperAction));
        let error_banner = adw::Banner::new("");
        error_banner.set_button_label(Some(tr("Details")));
        let s = sender.clone();
        error_banner.connect_button_clicked(move |_| s.input(Msg::ErrorDetails));

        let problem = adw::StatusPage::new();
        let problem_btn = gtk::Button::builder()
            .halign(gtk::Align::Center)
            .css_classes(["pill", "suggested-action"])
            .build();
        let s = sender.clone();
        problem_btn.connect_clicked(move |_| s.input(Msg::ProblemAction));
        problem.set_child(Some(&problem_btn));

        let content = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .build();
        content.add_named(&view_stack, Some("pages"));
        content.add_named(&problem, Some("problem"));

        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&content));

        let switcher_bar = adw::ViewSwitcherBar::builder().stack(&view_stack).build();
        let toolbar = adw::ToolbarView::new();
        toolbar.add_top_bar(&header);
        toolbar.add_top_bar(&helper_banner);
        toolbar.add_top_bar(&error_banner);
        toolbar.set_content(Some(&toasts));
        toolbar.add_bottom_bar(&switcher_bar);
        window.set_content(Some(&toolbar));

        // Narrow: switcher moves to the bottom bar.
        let narrow =
            adw::Breakpoint::new(adw::BreakpointCondition::parse("max-width: 500sp").unwrap());
        narrow.add_setter(
            &header,
            "title-widget",
            Some(&None::<gtk::Widget>.to_value()),
        );
        narrow.add_setter(&switcher_bar, "reveal", Some(&true.to_value()));
        window.add_breakpoint(narrow);
        // Wide: QR beside the details on Home.
        let wide =
            adw::Breakpoint::new(adw::BreakpointCondition::parse("min-width: 720sp").unwrap());
        home.wide_setters(&wide);
        window.add_breakpoint(wide);

        // Actions and shortcuts.
        let add = |name: &str, msg: fn() -> Msg| {
            let a = gio::SimpleAction::new(name, None);
            let s = sender.clone();
            a.connect_activate(move |_, _| s.input(msg()));
            window.add_action(&a);
        };
        add("toggle", || Msg::Toggle);
        add("copy-password", || Msg::CopyPassword);
        add("settings", || Msg::GoTo("settings"));
        add("about", || Msg::About);
        add("helper", || Msg::HelperAction);
        let demo_action = gio::SimpleAction::new("demo", Some(glib::VariantTy::STRING));
        let s = sender.clone();
        demo_action.connect_activate(move |_, p| {
            if let Some(sc) = p
                .and_then(|v| v.get::<String>())
                .and_then(|id| Scenario::from_id(&id))
            {
                s.input(Msg::Scenario(sc));
            }
        });
        window.add_action(&demo_action);
        let shortcuts = gio::SimpleAction::new("shortcuts", None);
        let w = window.clone();
        shortcuts.connect_activate(move |_, _| shortcuts_window(&w));
        window.add_action(&shortcuts);
        add("quit", || Msg::Quit);

        // With a tray icon, closing the window keeps the app (and the
        // per-device rules) running in the background.
        let hide_on_close = Rc::new(Cell::new(false));
        let h = hide_on_close.clone();
        window.connect_close_request(move |w| {
            if h.get() {
                w.set_visible(false);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });

        let app = relm4::main_application();
        // Show the window on every launch, except the first one at login.
        let first = Cell::new(args.hidden);
        let w = window.clone();
        app.connect_activate(move |_| {
            if !first.replace(false) {
                w.present();
            }
        });
        app.set_accels_for_action("win.toggle", &["<Control>space"]);
        app.set_accels_for_action("win.copy-password", &["<Control><Shift>c"]);
        app.set_accels_for_action("win.settings", &["<Control>comma"]);
        app.set_accels_for_action("win.quit", &["<Control>q"]);

        if args.demo {
            // Fake traffic once per second.
            let s = sender.clone();
            glib::timeout_add_seconds_local(1, move || {
                s.input(Msg::Tick);
                glib::ControlFlow::Continue
            });
        }

        if let Some((w, h)) = args.size {
            window.set_default_size(w, h);
        }
        if let Some(path) = args.screenshot.clone() {
            let w = window.clone();
            glib::timeout_add_local_once(Duration::from_millis(2500), move || {
                save_screenshot(&w, &path);
                w.application().inspect(|a| a.quit());
            });
        }
        if args.dark {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceDark);
        } else if args.light {
            adw::StyleManager::default().set_color_scheme(adw::ColorScheme::ForceLight);
        }
        let live = (!args.demo).then(|| Live::start(sender.input_sender().clone()));
        if let Some(l) = &live {
            l.watch_helper();
        }
        let mut state = if live.is_some() {
            live::initial_state(crate::config_store::load())
        } else {
            demo::initial()
        };
        if args.running && live.is_none() {
            state.run = RunState::On;
            demo::tick(&mut state);
        }
        if let Some(p) = &args.page {
            view_stack.set_visible_child_name(p);
        }
        if let Some(sc) = args.scenario.as_deref().and_then(Scenario::from_id) {
            sender.input(Msg::Scenario(sc));
        }
        if let Some(mac) = args.device {
            sender.input(Msg::OpenDevice(mac));
        }
        let start_pending = args.hidden && live.is_some() && state.cfg.start_at_login;
        if live.is_some() {
            // Keep the entry pointing at this binary.
            crate::autostart::set(state.cfg.autostart);
        }
        if live.is_some() && state.cfg.show_tray {
            crate::tray::spawn(sender.input_sender().clone());
        }
        let model = App {
            state,
            page: args.page.unwrap_or_else(|| "home".into()),
            next_start_fails: false,
            window: window.clone(),
            toasts,
            device: RefCell::new(None),
            view_stack,
            live,
            tray: None,
            hide_on_close,
            start_pending,
        };
        let widgets = Widgets {
            home,
            settings,
            clients,
            content,
            problem,
            switcher,
            switcher_bar,
            helper_banner,
            error_banner,
        };
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Msg, sender: ComponentSender<Self>) {
        let before = self.state.cfg.clone();
        let store_before = self.device_store();
        self.handle(msg, &sender);
        if self.live.is_some() {
            if self.state.cfg != before {
                crate::config_store::save(&self.state.cfg);
            }
            let store = self.device_store();
            if store != store_before {
                crate::config_store::save_devices(&store);
            }
        }
        if let Some(live) = &mut self.live {
            live.sync_shaping(&self.state);
        }
        self.hide_on_close
            .set(self.tray.is_some() && self.state.cfg.show_tray);
        if let Some(t) = &mut self.tray {
            t.update(tray_state(&self.state));
        }
    }

    fn update_view(&self, w: &mut Widgets, _sender: ComponentSender<Self>) {
        let s = &self.state;

        match s.problem {
            Some(p) => {
                let (icon, title, desc, action) = status_page_for(p);
                w.problem.set_icon_name(Some(icon));
                w.problem.set_title(title);
                w.problem.set_description(Some(desc));
                if let Some(b) = w.problem.child().and_downcast::<gtk::Button>() {
                    b.set_label(action);
                }
                w.content.set_visible_child_name("problem");
                w.switcher.set_sensitive(false);
                w.switcher_bar.set_sensitive(false);
            }
            None => {
                w.content.set_visible_child_name("pages");
                w.switcher.set_sensitive(true);
                w.switcher_bar.set_sensitive(true);
            }
        }

        let on_helper_page = self.page == "settings" || self.page == "clients";
        let (title, button) = match s.helper {
            HelperStatus::Ready => ("", None),
            HelperStatus::Missing => (
                tr("Per-device speed limits need the Linux Easy Hotspot helper"),
                Some(tr("Set up…")),
            ),
            HelperStatus::Unauthorized => (
                tr("Allow Linux Easy Hotspot to manage speed limits"),
                Some(tr("Authorize")),
            ),
            HelperStatus::TooOld => (
                tr("Update the helper to use per-device control"),
                Some(tr("Set up…")),
            ),
        };
        w.helper_banner.set_title(title);
        w.helper_banner.set_button_label(button);
        w.helper_banner
            .set_revealed(s.problem.is_none() && on_helper_page && !s.helper_ready());

        match &s.error {
            Some((title, _)) => {
                w.error_banner.set_title(title);
                w.error_banner.set_revealed(self.page == "home");
            }
            None => w.error_banner.set_revealed(false),
        }

        w.home.update(s);
        w.settings.update(s);
        w.clients.update(s);
        if let Some(d) = self.device.borrow().as_ref() {
            d.update(s);
        }

        // Badge on the Clients tab for pending approvals.
        if let Some(page) = self.view_stack.child_by_name("clients") {
            let p = self.view_stack.page(&page);
            let pending = s
                .clients
                .iter()
                .filter(|c| s.effective(c).access == Access::Pending)
                .count();
            let show = s.helper_ready() && s.run == RunState::On && pending > 0;
            p.set_needs_attention(show);
            p.set_badge_number(if show { pending as u32 } else { 0 });
        }
    }
}
