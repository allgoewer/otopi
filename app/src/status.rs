//! The status screen inside the LCARS frame: a title and a detail line in
//! the tall 8×16 font (the 8×8 font from `font8x8`, doubled in height).

use font8x8::{UnicodeFonts, BASIC_FONTS, LATIN_FONTS};

use crate::display::Image;
use crate::lcars::{ORANGE, PEACH};

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
            .or_else(|| LATIN_FONTS.get(ch))
            .or_else(|| BASIC_FONTS.get('?'))
            .unwrap();
        for (gy, bits) in glyph.iter().enumerate() {
            for gx in 0..8 {
                if bits & (1 << gx) != 0 {
                    let px = x + (i * 8 + gx) * scale;
                    fill_rect(img, px, y + gy * 2 * scale, scale, 2 * scale, color);
                }
            }
        }
    }
}

/// Title and detail line of a status screen, laid out for an image width.
struct Block {
    scale: usize,
    detail_scale: usize,
    width: usize,
    height: usize,
}

impl Block {
    fn new(img_width: usize, title: &str, detail: &str) -> Block {
        let scale = (img_width / 400).max(1);
        let detail_scale = scale.saturating_sub(1).max(1);
        let line_h = 16 * scale;
        let (width, height) = if detail.is_empty() {
            (text_width(title, scale), line_h)
        } else {
            (
                text_width(title, scale).max(text_width(detail, detail_scale)),
                line_h + line_h / 2 + 16 * detail_scale,
            )
        };
        Block {
            scale,
            detail_scale,
            width,
            height,
        }
    }

    /// Draws the block with its top-left corner at (x, y), lines centered.
    fn draw(&self, img: &mut Image, (x, y): (usize, usize), title: &str, detail: &str) {
        let tx = x + self.width.saturating_sub(text_width(title, self.scale)) / 2;
        draw_text(img, tx, y, title, self.scale, ORANGE);
        if !detail.is_empty() {
            let line_h = 16 * self.scale;
            let dy = y + line_h + line_h / 2;
            let dx = x + self
                .width
                .saturating_sub(text_width(detail, self.detail_scale))
                / 2;
            draw_text(img, dx, dy, detail, self.detail_scale, PEACH);
        }
    }
}

/// Clears `img` and draws a title line and an optional detail line, centered,
/// in upper case.
pub fn screen(img: &mut Image, title: &str, detail: &str) {
    let (title, detail) = (title.to_uppercase(), detail.to_uppercase());
    let (title, detail) = (title.as_str(), detail.as_str());
    img.data.fill(0);
    let b = Block::new(img.width, title, detail);
    let pos = (
        img.width.saturating_sub(b.width) / 2,
        img.height.saturating_sub(b.height) / 2,
    );
    b.draw(img, pos, title, detail);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draws_inside_bounds() {
        let mut img = Image::new(640, 360);
        screen(&mut img, "Searching for otoscope...", "hndec-*");
        assert!(img.data.chunks(4).any(|p| p == ORANGE));
        assert!(img.data.chunks(4).any(|p| p == PEACH));
        // a very long line must not panic
        screen(&mut img, &"x".repeat(500), &"y".repeat(500));
    }
}
