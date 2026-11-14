//! Easy Hotspot: share this computer's internet over Wi-Fi, on Linux
//! (NetworkManager) and Windows (the built-in Mobile Hotspot).

// No console window behind the app on Windows (release builds).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod activity;
mod autostart;
mod backend;
mod config_store;
mod i18n;
mod model;
mod msg;
mod single;
mod state;
mod tray;
mod ui;

use std::borrow::Cow;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::{App, AppContext as _, AssetSource, QuitMode, Result, SharedString};

/// Set by "Restart now" after a language change.
pub static RESTART: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// `--dark` / `--light` was given: don't follow the system appearance.
    pub static FORCED_THEME: Cell<bool> = const { Cell::new(false) };
}

/// Desktop/application id (icons, desktop entry, autostart file).
pub const APP_ID: &str = "io.github.OWNER.LinuxEasyHotspot";

/// Installed system-wide (e.g. from the .deb), not run from a source tree.
pub fn packaged() -> bool {
    cfg!(unix) && std::env::current_exe().is_ok_and(|p| p.starts_with("/usr/"))
}

gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        AlarmClock,
        Ban,
        Cable,
        Check,
        ChevronLeft,
        ChevronRight,
        CircleQuestionMark,
        CircleX,
        Copy,
        Eye,
        EyeOff,
        Info,
        Laptop,
        Menu,
        Play,
        RefreshCw,
        Settings,
        Smartphone,
        Square,
        TriangleAlert,
        Tv,
        UserMinus,
        UserPlus,
        Users,
        Wifi,
        WifiOff,
    ]
);

/// The components' own icons plus ours.
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some(found) = ExtraIcons.load(path)? {
            return Ok(Some(found));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut all = ExtraIcons.list(path)?;
        all.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(all)
    }
}

fn main() {
    // `RUST_LOG=info` (or debug) shows what GPUI and the backends do.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let margs = model::Args {
        demo: flag("--demo"),
        page: value("--page"),
        running: flag("--running"),
        scenario: value("--scenario"),
        device: value("--device"),
        hidden: flag("--hidden"),
    };
    let size = value("--size").and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some((w.parse().ok()?, h.parse().ok()?))
    });
    let (dark, light) = (flag("--dark"), flag("--light"));

    // A second launch shows the running instance (demo windows don't count).
    let server = if margs.demo {
        None
    } else {
        match single::claim() {
            Ok(s) => s,
            Err(()) => return,
        }
    };

    i18n::init(config_store::load().language);

    gpui_kit::application()
        .with_assets(Assets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            ui::bind_keys(cx);
            if dark || light {
                ui::force_theme(dark, cx);
            }
            let model = cx.new(|cx| model::Model::new(&margs, cx));
            if let Some(server) = server {
                let tx = model.read(cx).sender();
                server.serve(move || tx.emit(msg::Msg::ShowWindow));
            }
            cx.subscribe(&model, move |model, ev: &model::ModelEvent, cx| {
                if *ev == model::ModelEvent::ShowWindow {
                    ui::show_window(&model, size, cx);
                }
            })
            .detach();
            // At login, stay in the background (if there's a tray to come
            // back from).
            let hidden = margs.hidden && model.read(cx).keeps_running_or_will();
            if !hidden {
                ui::show_window(&model, size, cx);
            }
        });

    if RESTART.load(Ordering::Relaxed) {
        restart(&args);
    }
}

/// Starts the app again (after a language change).
fn restart(args: &[String]) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let args = args.iter().filter(|a| *a != "--hidden");
    let mut cmd = std::process::Command::new(exe);
    cmd.args(args).env(single::RESTARTED, "1");
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        let err = cmd.exec();
        eprintln!("restart failed: {err}");
    }
    #[cfg(not(unix))]
    if let Err(e) = cmd.spawn() {
        eprintln!("restart failed: {e}");
    }
}
