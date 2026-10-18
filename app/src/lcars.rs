//! LCARS-style frame around the picture: two elbows joined by a column of
//! sidebar blocks, a title bar on top and a bar with the status and system
//! info at the bottom. Drawn into a BGRA image with alpha, opaque black
//! except for the viewport, where the picture shows through.

use font8x8::{UnicodeFonts, BASIC_FONTS, LATIN_FONTS};

use crate::display::Image;

const fn bgra(r: u8, g: u8, b: u8) -> [u8; 4] {
    [b, g, r, 0xff]
}

pub const ORANGE: [u8; 4] = bgra(0xff, 0x99, 0x00);
pub const PEACH: [u8; 4] = bgra(0xff, 0xcc, 0x99);
const LILAC: [u8; 4] = bgra(0xcc, 0x99, 0xcc);
const PERIWINKLE: [u8; 4] = bgra(0x99, 0x99, 0xff);
const TAN: [u8; 4] = bgra(0xcc, 0x99, 0x66);
const RED: [u8; 4] = bgra(0xdd, 0x55, 0x44);
const GOLD: [u8; 4] = bgra(0xff, 0xcc, 0x66);
const BLUE: [u8; 4] = bgra(0x99, 0xcc, 0xff);
/// Sidebar block lit by the scanning animation.
const LIT: [u8; 4] = bgra(0xff, 0xee, 0xdd);
const BLACK: [u8; 4] = [0, 0, 0, 0xff];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

const TITLE: &str = "OTOPI";
/// Sidebar blocks: relative height, color, code.
const BLOCKS: [(usize, [u8; 4], &str); 4] = [
    (3, PEACH, "03-62"),
    (2, LILAC, "04-17"),
    (4, TAN, "05-89"),
    (2, BLUE, "06-31"),
];
/// Widest status text the pill is sized for.
const STATUS_CHARS: usize = 12;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

impl Rect {
    fn new(x: usize, y: usize, w: usize, h: usize) -> Rect {
        Rect { x, y, w, h }
    }

    /// From the half-open ranges x0..x1, y0..y1 (empty if reversed).
    fn span(x0: usize, y0: usize, x1: usize, y1: usize) -> Rect {
        Rect::new(x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
    }

    pub fn right(&self) -> usize {
        self.x + self.w
    }

    pub fn bottom(&self) -> usize {
        self.y + self.h
    }

    pub fn intersect(&self, o: &Rect) -> Rect {
        Rect::span(
            self.x.max(o.x),
            self.y.max(o.y),
            self.right().min(o.right()),
            self.bottom().min(o.bottom()),
        )
    }
}

/// What the frame shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// Text in the status pill, upper case.
    pub status: String,
    /// Red status pill.
    pub alert: bool,
    /// System info in the bottom bar.
    pub info: String,
    /// Sidebar scanning animation step; None: no block lit.
    pub phase: Option<usize>,
}

/// Geometry of the frame for an image size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    size: (usize, usize),
    /// Base unit: 1/54 of the image height (20 px at 1080 lines).
    u: usize,
    /// The whole frame (inside the margins).
    frame: Rect,
    sidebar_w: usize,
    bar_h: usize,
    /// Gap between neighbouring shapes.
    gap: usize,
    /// Scale of the 8×16 text.
    ts: usize,
    /// Where the picture shows through.
    pub viewport: Rect,
    /// The parts that change with the state.
    status: Rect,
    info: Rect,
    blocks: Rect,
}

impl Layout {
    pub fn new(size: (usize, usize)) -> Layout {
        let (w, h) = size;
        let u = (h / 54).max(4);
        let frame = Rect::new(u / 2, u / 2, w.saturating_sub(u), h.saturating_sub(u));
        let sidebar_w = 7 * u;
        let bar_h = 2 * u;
        let gap = (u / 4).max(1);
        let ts = ((bar_h * 4 / 5 + 8) / 16).max(1);
        let viewport = Rect::span(
            frame.x + sidebar_w + u,
            frame.y + bar_h + u,
            frame.right().saturating_sub(u / 2),
            frame.bottom().saturating_sub(bar_h + u),
        );
        let elbow_h = 5 * u;
        let blocks = Rect::span(
            frame.x,
            frame.y + elbow_h + gap,
            frame.x + sidebar_w,
            frame.bottom().saturating_sub(elbow_h + gap),
        );
        let bar_y = frame.bottom().saturating_sub(bar_h);
        let status_x = frame.x + sidebar_w + 4 * u + gap;
        let status = Rect::new(status_x, bar_y, STATUS_CHARS * 8 * ts + 2 * u, bar_h);
        let info = Rect::span(
            status.right() + gap,
            bar_y,
            cap_x(frame, u) - gap,
            frame.bottom(),
        );
        Layout {
            size,
            u,
            frame,
            sidebar_w,
            bar_h,
            gap,
            ts,
            viewport,
            status,
            info,
            blocks,
        }
    }

    pub fn size(&self) -> (usize, usize) {
        self.size
    }

    /// Rectangles to redraw for a change from `old` to `new`.
    pub fn changed(&self, old: &State, new: &State) -> Vec<Rect> {
        let mut v = Vec::new();
        if old.status != new.status || old.alert != new.alert {
            v.push(self.status);
        }
        if old.info != new.info {
            v.push(self.info);
        }
        if old.phase != new.phase {
            v.push(self.blocks);
        }
        v
    }

    pub fn full(&self) -> Rect {
        Rect::new(0, 0, self.size.0, self.size.1)
    }
}

/// Left edge of the rounded cap at the right end of a bar.
fn cap_x(frame: Rect, u: usize) -> usize {
    frame.right().saturating_sub(2 * u)
}

/// Width of `text` in the 8×16 font at scale `ts`.
fn text_w(text: &str, ts: usize) -> usize {
    text.chars().count() * 8 * ts
}

/// Draws clipped to a rectangle of the image.
struct Painter<'a> {
    img: &'a mut Image,
    clip: Rect,
}

impl Painter<'_> {
    fn rect(&mut self, r: Rect, color: [u8; 4]) {
        let r = r.intersect(&self.clip);
        for y in r.y..r.bottom() {
            let row = &mut self.img.data[(y * self.img.width + r.x) * 4..][..r.w * 4];
            for px in row.chunks_exact_mut(4) {
                px.copy_from_slice(&color);
            }
        }
    }

    /// Fills the part of `r` inside (`convex`) or outside the circle of
    /// radius `rad` around (`cx`, `cy`), antialiased against black.
    fn arc(&mut self, r: Rect, (cx, cy): (f32, f32), rad: f32, convex: bool, color: [u8; 4]) {
        const N: usize = 4;
        let r = r.intersect(&self.clip);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                let mut n = 0;
                for sy in 0..N {
                    for sx in 0..N {
                        let dx = x as f32 + (sx as f32 + 0.5) / N as f32 - cx;
                        let dy = y as f32 + (sy as f32 + 0.5) / N as f32 - cy;
                        if (dx * dx + dy * dy < rad * rad) == convex {
                            n += 1;
                        }
                    }
                }
                let o = (y * self.img.width + x) * 4;
                for c in 0..3 {
                    self.img.data[o + c] = (color[c] as usize * n / (N * N)) as u8;
                }
                self.img.data[o + 3] = 0xff;
            }
        }
    }

    /// A bar with a half-circle right end.
    fn cap(&mut self, r: Rect, color: [u8; 4]) {
        let rad = r.h / 2;
        self.rect(Rect::new(r.x, r.y, r.w - rad, r.h), color);
        let c = ((r.right() - rad) as f32, r.y as f32 + r.h as f32 / 2.0);
        self.arc(
            Rect::new(r.right() - rad, r.y, rad, r.h),
            c,
            rad as f32,
            true,
            color,
        );
    }

    /// A pill with two half-circle ends.
    fn pill(&mut self, r: Rect, color: [u8; 4]) {
        let rad = r.h / 2;
        let cy = r.y as f32 + r.h as f32 / 2.0;
        self.rect(Rect::new(r.x + rad, r.y, r.w - 2 * rad, r.h), color);
        let left = Rect::new(r.x, r.y, rad, r.h);
        self.arc(left, ((r.x + rad) as f32, cy), rad as f32, true, color);
        let right = Rect::new(r.right() - rad, r.y, rad, r.h);
        self.arc(
            right,
            ((r.right() - rad) as f32, cy),
            rad as f32,
            true,
            color,
        );
    }

    /// Text in the tall 8×16 font (the 8×8 glyphs doubled in height).
    fn text(&mut self, x: usize, y: usize, text: &str, ts: usize, color: [u8; 4]) {
        for (i, ch) in text.chars().enumerate() {
            let glyph = BASIC_FONTS
                .get(ch)
                .or_else(|| LATIN_FONTS.get(ch))
                .or_else(|| BASIC_FONTS.get('?'))
                .unwrap();
            for (gy, bits) in glyph.iter().enumerate() {
                for gx in 0..8 {
                    if bits & (1 << gx) != 0 {
                        let px = Rect::new(x + (i * 8 + gx) * ts, y + gy * 2 * ts, ts, 2 * ts);
                        self.rect(px, color);
                    }
                }
            }
        }
    }

    /// An elbow: the sidebar column joined to a bar at the frame's top
    /// (`top`) or bottom edge, with a large outer and a small inner radius.
    fn elbow(&mut self, l: &Layout, top: bool, bar_end: usize, color: [u8; 4], code: &str) {
        let f = l.frame;
        let (sw, bh, u) = (l.sidebar_w, l.bar_h, l.u);
        let (outer, inner) = (3 * u, u);
        let len = 5 * u;
        // Vertical rectangles are given from the frame edge inwards and
        // mirrored for the bottom elbow.
        let v = |d0: usize, d1: usize| {
            if top {
                (f.y + d0, f.y + d1)
            } else {
                (f.bottom() - d1, f.bottom() - d0)
            }
        };
        let (y0, y1) = v(outer, len);
        self.rect(Rect::span(f.x, y0, f.x + sw, y1), color);
        let (y0, y1) = v(0, outer);
        self.rect(Rect::span(f.x + outer, y0, f.x + sw, y1), color);
        let (y0, y1) = v(0, bh);
        self.rect(Rect::span(f.x + sw, y0, bar_end, y1), color);
        // outer corner
        let (y0, y1) = v(0, outer);
        let cy = if top { f.y + outer } else { f.bottom() - outer } as f32;
        let c = ((f.x + outer) as f32, cy);
        self.arc(
            Rect::span(f.x, y0, f.x + outer, y1),
            c,
            outer as f32,
            true,
            color,
        );
        // inner fillet
        let (y0, y1) = v(bh, bh + inner);
        let cy = if top {
            f.y + bh + inner
        } else {
            f.bottom() - bh - inner
        } as f32;
        let c = ((f.x + sw + inner) as f32, cy);
        let r = Rect::span(f.x + sw, y0, f.x + sw + inner, y1);
        self.arc(r, c, inner as f32, false, color);
        // code in the corner next to the bar
        let ts = l.ts;
        let tx = (f.x + sw).saturating_sub(text_w(code, ts) + l.gap * 2);
        let ty = if top {
            f.y + len - 16 * ts - l.gap
        } else {
            f.bottom() - len + l.gap
        };
        self.text(tx, ty, code, ts, BLACK);
    }
}

/// Draws the part of the frame inside `clip`.
pub fn render(img: &mut Image, l: &Layout, s: &State, clip: Rect) {
    assert_eq!((img.width, img.height), l.size);
    let clip = clip.intersect(&l.full());
    let mut p = Painter { img, clip };
    p.rect(l.full(), BLACK);
    p.rect(l.viewport, CLEAR);

    let (f, u, g, ts) = (l.frame, l.u, l.gap, l.ts);
    let text_y = |bar_y: usize| bar_y + (l.bar_h - 16 * ts) / 2;

    // top: elbow, a second segment, the title, the cap
    let cap = cap_x(f, u);
    let title_w = text_w(TITLE, ts);
    let title_x = cap.saturating_sub(title_w + u / 2);
    let seg_x = f.x + l.sidebar_w + 9 * u;
    p.elbow(l, true, seg_x - g, ORANGE, "01-0254");
    // two short segments before the long one
    let mut x = seg_x;
    for (w, color) in [(3 * u, RED), (12 * u, GOLD)] {
        p.rect(
            Rect::span(x, f.y, (x + w).min(title_x), f.y + l.bar_h),
            color,
        );
        x += w + g;
    }
    p.rect(
        Rect::span(x, f.y, title_x.saturating_sub(u / 2), f.y + l.bar_h),
        PERIWINKLE,
    );
    p.text(title_x, text_y(f.y), TITLE, ts, ORANGE);
    p.cap(Rect::span(cap, f.y, f.right(), f.y + l.bar_h), ORANGE);

    // sidebar blocks
    let b = l.blocks;
    let total: usize = BLOCKS.iter().map(|b| b.0).sum();
    let free = b.h.saturating_sub(g * (BLOCKS.len() - 1));
    let mut y = b.y;
    for (i, &(weight, color, code)) in BLOCKS.iter().enumerate() {
        let h = if i == BLOCKS.len() - 1 {
            b.bottom().saturating_sub(y)
        } else {
            free * weight / total
        };
        let color = if s.phase.is_some_and(|ph| ph % BLOCKS.len() == i) {
            LIT
        } else {
            color
        };
        p.rect(Rect::new(b.x, y, b.w, h), color);
        if h >= 16 * ts + 2 * g {
            let tx = b.right().saturating_sub(text_w(code, ts) + 2 * g);
            p.text(tx, y + h - 16 * ts - g, code, ts, BLACK);
        }
        y += h + g;
    }

    // bottom: elbow, status pill, a segment, the info text, the cap
    let bar_y = f.bottom() - l.bar_h;
    p.elbow(l, false, l.status.x - g, TAN, "02-7311");
    let st = l.status;
    p.pill(st, if s.alert { RED } else { LILAC });
    let status: String = s.status.chars().take(STATUS_CHARS).collect();
    let sx = st.x + (st.w - text_w(&status, ts)) / 2;
    p.text(sx, text_y(bar_y), &status, ts, BLACK);
    let info = l.info;
    let iw = text_w(&s.info, ts).min(info.w.saturating_sub(4 * u));
    let ix = info.right() - iw - u / 2;
    p.rect(Rect::span(info.x, bar_y, ix - u / 2, f.bottom()), GOLD);
    let shown: String = s.info.chars().take(iw / (8 * ts)).collect();
    p.text(ix, text_y(bar_y), &shown, ts, PEACH);
    p.cap(Rect::span(cap, bar_y, f.right(), f.bottom()), TAN);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State {
            status: "SEARCHING".into(),
            alert: false,
            info: "CPU  12%  48\u{b0}C  v1.0.1-2-g9d306ce-dirty".into(),
            phase: Some(1),
        }
    }

    fn px(img: &Image, x: usize, y: usize) -> [u8; 4] {
        let o = (y * img.width + x) * 4;
        img.data[o..o + 4].try_into().unwrap()
    }

    #[test]
    fn viewport_is_clear_and_frame_is_opaque() {
        for size in [(960, 540), (1280, 720), (640, 480), (1920, 1200)] {
            let l = Layout::new(size);
            let mut img = Image::new(size.0, size.1);
            render(&mut img, &l, &state(), l.full());
            let v = l.viewport;
            assert!(
                v.w > size.0 * 3 / 4 && v.h > size.1 * 3 / 4,
                "{size:?}: {v:?}"
            );
            assert_eq!(px(&img, v.x, v.y)[3], 0);
            assert_eq!(px(&img, v.right() - 1, v.bottom() - 1)[3], 0);
            assert_eq!(px(&img, v.x - 1, v.y)[3], 0xff);
            assert_eq!(px(&img, 0, 0)[3], 0xff);
            // the bars reach the right edge of the frame, inside the image
            assert!(l.frame.right() <= size.0 && l.frame.bottom() <= size.1);
            assert_ne!(
                px(&img, l.frame.right() - l.u, l.frame.y + l.u)[..3],
                [0, 0, 0]
            );
        }
    }

    #[test]
    fn partial_render_matches_full() {
        let l = Layout::new((960, 540));
        let (a, mut b) = (state(), state());
        b.status = "STREAMING".into();
        b.alert = true;
        b.info = "CPU 100%  80\u{b0}C  v1".into();
        b.phase = None;
        let mut img = Image::new(960, 540);
        render(&mut img, &l, &a, l.full());
        let rects = l.changed(&a, &b);
        assert_eq!(rects.len(), 3);
        for r in rects {
            render(&mut img, &l, &b, r);
        }
        let mut full = Image::new(960, 540);
        render(&mut full, &l, &b, l.full());
        assert!(img.data == full.data);
        assert!(l.changed(&b, &b).is_empty());
    }

    /// Writes the frame as PPM files for a look on the host:
    /// OTOPI_PREVIEW=dir cargo test preview -- --ignored
    #[test]
    #[ignore]
    fn preview() {
        let dir = std::env::var("OTOPI_PREVIEW").unwrap_or_else(|_| ".".into());
        for (name, size, s) in [
            ("searching", (1920, 1080), state()),
            (
                "streaming",
                (1920, 1080),
                State {
                    status: "STREAMING".into(),
                    phase: None,
                    ..state()
                },
            ),
            (
                "alert",
                (1280, 720),
                State {
                    status: "RECONNECTING".into(),
                    alert: true,
                    phase: None,
                    ..state()
                },
            ),
        ] {
            let l = Layout::new(size);
            let mut img = Image::new(size.0, size.1);
            render(&mut img, &l, &s, l.full());
            let v = l.viewport;
            let vh = v.h.min(v.w * 9 / 16);
            let video = Rect::new(
                v.x + (v.w - vh * 16 / 9) / 2,
                v.y + (v.h - vh) / 2,
                vh * 16 / 9,
                vh,
            );
            let mut ppm = format!("P6 {} {} 255\n", size.0, size.1).into_bytes();
            for (i, p) in img.data.chunks_exact(4).enumerate() {
                // grey where the picture goes
                let (x, y) = (i % size.0, i / size.0);
                let inside = video.intersect(&Rect::new(x, y, 1, 1)).w == 1;
                let rgb = match (p[3], inside) {
                    (0, true) => [90; 3],
                    (0, false) => [0; 3],
                    _ => [p[2], p[1], p[0]],
                };
                ppm.extend_from_slice(&rgb);
            }
            std::fs::write(format!("{dir}/{name}.ppm"), ppm).unwrap();
        }
    }
}
