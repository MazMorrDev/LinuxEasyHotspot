//! LinuxEasyHotspot GTK app.

mod app;
mod autostart;
mod config_store;
mod i18n;
mod live;
mod pages;
mod qr;
mod state;
mod tray;
mod widgets;

use std::sync::atomic::{AtomicBool, Ordering};

use relm4::RelmApp;

/// Set by "Restart now" after a language change.
pub static RESTART: AtomicBool = AtomicBool::new(false);

pub const APP_ID: &str = "io.github.OWNER.LinuxEasyHotspot";
pub const ICON_SYMBOLIC: &str = "io.github.OWNER.LinuxEasyHotspot-symbolic";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let init = app::Args {
        demo: flag("--demo"),
        page: value("--page"),
        running: flag("--running"),
        dark: flag("--dark"),
        light: flag("--light"),
        scenario: value("--scenario"),
        device: value("--device"),
        screenshot: value("--screenshot"),
        hidden: flag("--hidden"),
        size: value("--size").and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        }),
    };
    let lang = config_store::load().language;
    match_toolkit_language(lang, &args);
    i18n::init(lang);
    let app = RelmApp::new(APP_ID).visible_on_activate(false);
    relm4::set_global_css(include_str!("style.css"));
    // GTK must not see our own flags.
    app.with_args(Vec::new()).run::<app::App>(init);

    if RESTART.load(Ordering::Relaxed) {
        use std::os::unix::process::CommandExt as _;
        if let Ok(exe) = std::env::current_exe() {
            let args = args.iter().filter(|a| *a != "--hidden");
            // Let the new process pick LANGUAGE for the new setting.
            let err = std::process::Command::new(exe)
                .args(args)
                .env_remove("LEH_LANGUAGE_SET")
                .env_remove("LANGUAGE")
                .exec();
            eprintln!("restart failed: {err}");
        }
    }
}

/// GTK and libadwaita translate their own labels (About dialog, shortcuts
/// window…) from `LANGUAGE`. When a language is picked explicitly and the
/// environment says otherwise, start again with it set; this happens before
/// GTK is loaded, so it costs nothing visible.
fn match_toolkit_language(lang: leh_model::Language, args: &[String]) {
    use std::os::unix::process::CommandExt as _;
    if lang == leh_model::Language::System || std::env::var_os("LEH_LANGUAGE_SET").is_some() {
        return;
    }
    let code = i18n::resolve(lang);
    let current = std::env::var("LANGUAGE").unwrap_or_default();
    if current.split(':').next().unwrap_or("").starts_with(code) {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let err = std::process::Command::new(exe)
        .args(args)
        .env("LANGUAGE", code)
        .env("LEH_LANGUAGE_SET", "1")
        .exec();
    eprintln!("language: couldn't restart with LANGUAGE={code}: {err}");
}
