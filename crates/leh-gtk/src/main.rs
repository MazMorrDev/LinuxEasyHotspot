//! LinuxEasyHotspot GTK app.

mod app;
mod config_store;
mod live;
mod pages;
mod qr;
mod state;
mod widgets;

use relm4::RelmApp;

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
        size: value("--size").and_then(|s| {
            let (w, h) = s.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        }),
    };
    let app = RelmApp::new(APP_ID);
    relm4::set_global_css(include_str!("style.css"));
    // GTK must not see our own flags.
    app.with_args(Vec::new()).run::<app::App>(init);
}
