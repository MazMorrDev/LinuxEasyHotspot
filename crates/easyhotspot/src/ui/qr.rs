//! The QR code to join the hotspot, painted as squares. Always dark on white
//! with a 4-module quiet zone, regardless of the theme, so every phone camera
//! can read it.

use std::rc::Rc;

use gpui_kit::{
    AnyElement, IntoElement, ParentElement, Styled, Window, black, canvas, div, fill, point, px,
    size, white,
};
use qrcode::{Color, QrCode};

const QUIET: usize = 4;

/// The modules of the last payload, so the code is only computed on change.
#[derive(Default)]
pub struct Qr {
    payload: String,
    width: usize,
    dark: Rc<Vec<bool>>,
}

impl Qr {
    fn update(&mut self, payload: &str) {
        if self.payload == payload && self.width > 0 {
            return;
        }
        self.payload = payload.to_owned();
        match QrCode::new(payload.as_bytes()) {
            Ok(code) => {
                self.width = code.width();
                self.dark = Rc::new(
                    code.to_colors()
                        .into_iter()
                        .map(|c| c == Color::Dark)
                        .collect(),
                );
            }
            Err(_) => {
                self.width = 0;
                self.dark = Rc::default();
            }
        }
    }

    /// The code at about `target` pixels (whole pixels per module).
    pub fn render(&mut self, payload: &str, target: f32) -> AnyElement {
        self.update(payload);
        let w = self.width;
        let total = w + 2 * QUIET;
        let module = (target / total.max(1) as f32).floor().max(2.0);
        let side = module * total as f32;
        let dark = self.dark.clone();
        div()
            .size(px(side))
            .bg(white())
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window: &mut Window, _| {
                        let o = bounds.origin;
                        for y in 0..w {
                            for x in 0..w {
                                if !dark[y * w + x] {
                                    continue;
                                }
                                let at = point(
                                    o.x + px((x + QUIET) as f32 * module),
                                    o.y + px((y + QUIET) as f32 * module),
                                );
                                window.paint_quad(fill(
                                    gpui_kit::Bounds::new(at, size(px(module), px(module))),
                                    black(),
                                ));
                            }
                        }
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}
