//! Colors on top of the component theme: a blue accent and solid
//! destructive buttons (the component default is neutral), and a page
//! background a shade away from the cards.

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::{App, Hsla, Window, rgb};

fn hex(c: u32) -> Hsla {
    rgb(c).into()
}

/// Re-applies our colors; call after every theme change.
pub fn apply(cx: &mut App) {
    Theme::update(cx, recolor);
}

fn recolor(t: &mut Theme) {
    let dark = t.is_dark();
    let (blue, blue_hover, blue_active) = if dark {
        (hex(0x3584e4), hex(0x4a90e8), hex(0x2a75d0))
    } else {
        (hex(0x3584e4), hex(0x2a78d6), hex(0x1c6bc7))
    };
    let (red, red_hover, red_active) = if dark {
        (hex(0xc01c28), hex(0xd02a35), hex(0xa51d26))
    } else {
        (hex(0xe01b24), hex(0xc81820), hex(0xb0151c))
    };
    let white = hex(0xffffff);
    t.primary = blue;
    t.primary_hover = blue_hover;
    t.primary_active = blue_active;
    t.primary_foreground = white;
    t.button_primary = blue;
    t.button_primary_hover = blue_hover;
    t.button_primary_active = blue_active;
    t.button_primary_foreground = white;
    t.button_danger = red;
    t.button_danger_hover = red_hover;
    t.button_danger_active = red_active;
    t.button_danger_foreground = white;
    t.ring = blue;
    t.progress_bar = blue;
    t.link = blue;
    t.success = if dark { hex(0x26a269) } else { hex(0x2ec27e) };
    t.warning = if dark { hex(0xcd9309) } else { hex(0xc88800) };
    t.danger = red;
}

/// Follows the desktop's light/dark setting.
pub fn follow_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
    apply(cx);
}

/// `--dark` / `--light`.
pub fn force(dark: bool, cx: &mut App) {
    Theme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    apply(cx);
}

/// The window background: a shade darker than the cards in light mode,
/// the theme background in dark mode.
pub fn page_bg(cx: &App) -> Hsla {
    let t = cx.theme();
    if t.is_dark() {
        t.background
    } else {
        hex(0xf6f6f7)
    }
}

/// Background of rows and cards.
pub fn card_bg(cx: &App) -> Hsla {
    let t = cx.theme();
    if t.is_dark() {
        let mut c = t.background;
        c.l = (c.l + 0.06).min(1.0);
        c
    } else {
        hex(0xffffff)
    }
}
