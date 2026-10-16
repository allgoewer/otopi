//! Text rendering for the status screen, with the 8×8 font from `font8x8`.

use font8x8::{UnicodeFonts, BASIC_FONTS, LATIN_FONTS};

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
            .or_else(|| LATIN_FONTS.get(ch))
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

/// Title and detail line of a status screen, laid out for an image width.
struct Block {
    scale: usize,
    detail_scale: usize,
    width: usize,
    height: usize,
}

impl Block {
    fn new(img_width: usize, title: &str, detail: &str) -> Block {
        let scale = (img_width / 320).max(1);
        let detail_scale = scale.saturating_sub(1).max(1);
        let line_h = 8 * scale;
        let (width, height) = if detail.is_empty() {
            (text_width(title, scale), line_h)
        } else {
            (
                text_width(title, scale).max(text_width(detail, detail_scale)),
                line_h + line_h / 2 + 8 + 8 * detail_scale,
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
    fn draw(
        &self,
        img: &mut Image,
        (x, y): (usize, usize),
        title: &str,
        detail: &str,
        color: [u8; 4],
    ) {
        let tx = x + self.width.saturating_sub(text_width(title, self.scale)) / 2;
        draw_text(img, tx, y, title, self.scale, color);
        if !detail.is_empty() {
            let line_h = 8 * self.scale;
            let dy = y + line_h + line_h / 2 + 8;
            let dx = x + self
                .width
                .saturating_sub(text_width(detail, self.detail_scale))
                / 2;
            draw_text(img, dx, dy, detail, self.detail_scale, GREY);
        }
    }
}

/// Clears `img` and draws a title line and an optional detail line, centered.
pub fn screen(img: &mut Image, title: &str, detail: &str) {
    img.data.fill(0);
    let b = Block::new(img.width, title, detail);
    let pos = (
        img.width.saturating_sub(b.width) / 2,
        img.height.saturating_sub(b.height) / 2,
    );
    b.draw(img, pos, title, detail, WHITE);
}

/// Title colors of the bouncing screen (BGRA), changed on every bounce.
const BOUNCE_COLORS: [[u8; 4]; 6] = [
    [0xff, 0xff, 0xff, 0xff], // white
    [0x40, 0xd0, 0xff, 0xff], // yellow
    [0xff, 0xa0, 0x40, 0xff], // light blue
    [0x60, 0xff, 0x60, 0xff], // green
    [0xff, 0x70, 0xff, 0xff], // pink
    [0x40, 0x90, 0xff, 0xff], // orange
];

/// Moves a status screen's text across the screen and bounces it off the
/// edges, like the DVD logo. Slow, and it also avoids burn-in.
pub struct Bouncer {
    pos: Option<(f32, f32)>,
    /// Pixels per second (image pixels) in x and y.
    vel: (f32, f32),
    color: usize,
}

impl Bouncer {
    /// Image pixels per second, per axis.
    const SPEED: f32 = 24.0;

    pub fn new() -> Bouncer {
        Bouncer {
            pos: None,
            vel: (Self::SPEED, Self::SPEED),
            color: 0,
        }
    }

    /// Advances by `dt` seconds and draws the screen at the new position.
    pub fn screen(&mut self, img: &mut Image, title: &str, detail: &str, dt: f32) {
        img.data.fill(0);
        let b = Block::new(img.width, title, detail);
        let max = (
            img.width.saturating_sub(b.width) as f32,
            img.height.saturating_sub(b.height) as f32,
        );
        // Starts centered.
        let (mut x, mut y) = self.pos.unwrap_or((max.0 / 2.0, max.1 / 2.0));
        x += self.vel.0 * dt;
        y += self.vel.1 * dt;
        let mut bounced = false;
        for (p, v, m) in [
            (&mut x, &mut self.vel.0, max.0),
            (&mut y, &mut self.vel.1, max.1),
        ] {
            if *p <= 0.0 {
                *p = 0.0;
                *v = v.abs();
                bounced = true;
            } else if *p >= m {
                *p = m;
                *v = -v.abs();
                bounced = true;
            }
        }
        if bounced {
            self.color = (self.color + 1) % BOUNCE_COLORS.len();
        }
        self.pos = Some((x, y));
        let color = BOUNCE_COLORS[self.color];
        b.draw(img, (x as usize, y as usize), title, detail, color);
    }
}

/// A line of text on a semi-transparent dark box, as a BGRA image with alpha
/// (premultiplied: the box is black). Font size 8 * `scale` pixels.
pub fn label(text: &str, scale: usize) -> Image {
    let pad = scale;
    let mut img = Image::new(text_width(text, scale) + 2 * pad, 8 * scale + 2 * pad);
    for px in img.data.chunks_exact_mut(4) {
        px.copy_from_slice(&[0, 0, 0, 0xa0]);
    }
    draw_text(&mut img, pad, pad, text, scale, WHITE);
    img
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
        let l = label("CPU  12%  48\u{b0}C  v0.1.0-3-gabcdef0", 2);
        assert_eq!((l.width, l.height), (33 * 16 + 4, 20));
        assert!(l.data.chunks(4).any(|p| p == WHITE));
        assert!(l.data.chunks(4).any(|p| p == [0, 0, 0, 0xa0]));
        assert!(img.data.chunks(4).any(|p| p == WHITE));
        // a very long line must not panic
        screen(&mut img, &"x".repeat(500), &"y".repeat(500));
    }

    /// Bounding box of all non-black pixels.
    fn bbox(img: &Image) -> (usize, usize, usize, usize) {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for (i, p) in img.data.chunks(4).enumerate() {
            if p[..3] != [0, 0, 0] {
                let (x, y) = (i % img.width, i / img.width);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
        (x0, y0, x1, y1)
    }

    #[test]
    fn bouncer_moves_bounces_and_stays_inside() {
        let mut img = Image::new(640, 360);
        let mut b = Bouncer::new();
        b.screen(
            &mut img,
            "Searching for otoscope...",
            "Switch on the otoscope",
            0.0,
        );
        let start = bbox(&img);
        b.screen(
            &mut img,
            "Searching for otoscope...",
            "Switch on the otoscope",
            1.0,
        );
        let moved = bbox(&img);
        assert_eq!(
            (moved.0, moved.1),
            (start.0 + 24, start.1 + 24),
            "down-right first"
        );
        let mut colors = std::collections::HashSet::new();
        for _ in 0..150 {
            b.screen(
                &mut img,
                "Searching for otoscope...",
                "Switch on the otoscope",
                0.1,
            );
            let (_, _, x1, y1) = bbox(&img);
            assert!(x1 < 640 && y1 < 360);
            colors.insert(b.color);
        }
        assert!(colors.len() > 1, "color changes on bounce");
        // even a block wider than the screen must not panic
        b.screen(&mut img, &"x".repeat(500), "", 0.1);
    }
}
