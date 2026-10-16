//! Text rendering for the status screen, with the 8×8 font from `font8x8`.

use font8x8::{UnicodeFonts, BASIC_FONTS};

use crate::display::Image;

const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
const GREY: [u8; 4] = [0xa0, 0xa0, 0xa0, 0xff];
const BADGE_BG: [u8; 4] = [0x20, 0x20, 0xc0, 0xff]; // BGRA: dark red

fn fill_rect(img: &mut Image, x: usize, y: usize, w: usize, h: usize, color: [u8; 4]) {
    for row in y..(y + h).min(img.height) {
        for col in x..(x + w).min(img.width) {
            let o = (row * img.width + col) * 4;
            img.data[o..o + 4].copy_from_slice(&color);
        }
    }
}

fn text_width(text: &str, scale: usize) -> usize {
    text.chars().count() * 8 * scale
}

fn draw_text(img: &mut Image, x: usize, y: usize, text: &str, scale: usize, color: [u8; 4]) {
    for (i, ch) in text.chars().enumerate() {
        let glyph = BASIC_FONTS
            .get(ch)
            .or_else(|| BASIC_FONTS.get('?'))
            .unwrap();
        for (gy, bits) in glyph.iter().enumerate() {
            for gx in 0..8 {
                if bits & (1 << gx) != 0 {
                    let px = x + (i * 8 + gx) * scale;
                    fill_rect(img, px, y + gy * scale, scale, scale, color);
                }
            }
        }
    }
}

/// Clears `img` and draws a title line and an optional detail line, centered.
pub fn screen(img: &mut Image, title: &str, detail: &str) {
    img.data.fill(0);
    let scale = (img.width / 320).max(1);
    let line_h = 8 * scale;
    let total_h = if detail.is_empty() {
        line_h
    } else {
        line_h * 2 + line_h / 2 + 8
    };
    let mut y = img.height.saturating_sub(total_h) / 2;
    let x = img.width.saturating_sub(text_width(title, scale)) / 2;
    draw_text(img, x, y, title, scale, WHITE);
    if !detail.is_empty() {
        y += line_h + line_h / 2 + 8;
        let s = scale.saturating_sub(1).max(1);
        let x = img.width.saturating_sub(text_width(detail, s)) / 2;
        draw_text(img, x, y, detail, s, GREY);
    }
}

/// Draws a small label in the top-left corner, over the current content.
pub fn badge(img: &mut Image, text: &str) {
    let pad = 4;
    let w = text_width(text, 1) + 2 * pad;
    fill_rect(img, 8, 8, w, 8 + 2 * pad, BADGE_BG);
    draw_text(img, 8 + pad, 8 + pad, text, 1, WHITE);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_inside_bounds() {
        let mut img = Image::new(640, 360);
        screen(&mut img, "Searching for otoscope...", "hndec-*");
        badge(&mut img, "Reconnecting");
        assert!(img.data.chunks(4).any(|p| p == WHITE));
        // a very long line must not panic
        screen(&mut img, &"x".repeat(500), &"y".repeat(500));
    }
}
