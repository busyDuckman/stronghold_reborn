// --------------------------------------------------------------------------------------------------------------------
// The macroquad bits the editor needs that macroquad does not provide.
//
// These are deliberately copied rather than shared with the project: a util that reaches
// into the project's rendering module is a util that stops compiling every time that module
// moves. There is not much here, and it is not going to change.

use macroquad::miniquad::graphics::{BlendFactor, BlendState, BlendValue, Equation};
use macroquad::prelude::*;

/// Straight alpha: out = src*a + dst*(1-a). What a shader author usually means by RGBA.
pub fn alpha_blend_plain() -> BlendState {
    BlendState::new(
        Equation::Add,
        BlendFactor::Value(BlendValue::SourceAlpha),
        BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
    )
}

/// Premultiplied alpha: out.rgb = src + dst*(1-src.a). For shaders that output pRGBA.
pub fn alpha_blend_premul() -> BlendState {
    BlendState::new(Equation::Add, BlendFactor::One, BlendFactor::OneMinusValue(BlendValue::SourceAlpha))
}

/// The transparency checkerboard drawn behind the preview.
/// Snapped to whole pixels, or the cell edges shimmer as the window resizes.
pub fn draw_checkerboard(origin: Vec2, size: Vec2, cell: f32) {
    if cell <= 1.0 { return; }
    let cols = ((size.x / cell).ceil() as i32).max(1);
    let rows = ((size.y / cell).ceil() as i32).max(1);
    let c0 = Color::from_rgba(200, 200, 200, 255);
    let c1 = Color::from_rgba(140, 140, 140, 255);

    let ox = origin.x.floor();
    let oy = origin.y.floor();
    for iy in 0..rows {
        for ix in 0..cols {
            let x = ox + ix as f32 * cell;
            let y = oy + iy as f32 * cell;
            let w = (origin.x + size.x - x).clamp(0.0, cell);
            let h = (origin.y + size.y - y).clamp(0.0, cell);
            if w > 0.0 && h > 0.0 {
                draw_rectangle(x, y, w, h, if (ix + iy) % 2 == 0 { c0 } else { c1 });
            }
        }
    }
}

/// A checkerboard texture, bound to any sampler the shader did not give a stand-in for.
pub fn make_checker_tex(w: u32, h: u32) -> Texture2D {
    let mut img = Image::gen_image_color(w as u16, h as u16, WHITE);
    for y in 0..h {
        for x in 0..w {
            let c = if ((x / 32) + (y / 32)) % 2 == 0 { 210 } else { 120 };
            img.set_pixel(x, y, Color::from_rgba(c, c, c, 255));
        }
    }
    Texture2D::from_image(&img)
}
