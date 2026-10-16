//! Keeps the image level with the NE3 tilt sensor (spec section "tilt sensor
//! and image rotation").

use crate::display::Image;

/// Weight of a new tilt sample in the moving average (per frame).
const SMOOTHING: f32 = 0.3;
/// Below this rotation the image is shown unrotated.
pub const MIN_DEGREES: f32 = 0.5;
/// Snap mode: within this distance of a multiple of 90° the angle snaps to it,
/// and it stays snapped until the distance exceeds SNAP_LEAVE (hysteresis, so
/// it does not flicker at the edge).
const SNAP_ENTER: f32 = 8.0;
const SNAP_LEAVE: f32 = 12.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// No rotation.
    Off,
    /// Follow the tilt sensor at any angle.
    Free,
    /// Like Free, but snap to 0/90/180/270° when close to it.
    Snap,
}

/// Rotation in degrees (counterclockwise positive) for a tilt vector,
/// following the reference client: angle = atan2(x, y), rotation = -angle - 90.
fn rotation_for(x: f32, y: f32) -> f32 {
    normalize(-x.atan2(y).to_degrees() - 90.0)
}

/// Maps an angle to (-180, 180].
fn normalize(deg: f32) -> f32 {
    let d = deg.rem_euclid(360.0);
    if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

/// Turns tilt samples into a smoothed rotation angle.
pub struct Leveler {
    mode: Mode,
    /// Moving average of the tilt vector; averaging the vector instead of the
    /// angle has no wrap-around problem at ±180°.
    smoothed: Option<(f32, f32)>,
    /// Unsnapped rotation.
    degrees: f32,
    /// Snap mode: the multiple of 90° the rotation is currently snapped to.
    snapped: Option<f32>,
}

impl Leveler {
    pub fn new(mode: Mode) -> Leveler {
        Leveler {
            mode,
            smoothed: None,
            degrees: 0.0,
            snapped: None,
        }
    }

    /// Returns the rotation for the current frame. Without a tilt sample the
    /// last rotation stays.
    pub fn update(&mut self, tilt: Option<(i32, i32)>) -> f32 {
        match self.mode {
            Mode::Off => 0.0,
            Mode::Free => self.track(tilt),
            Mode::Snap => {
                let d = self.track(tilt);
                self.snap(d)
            }
        }
    }

    fn snap(&mut self, d: f32) -> f32 {
        if let Some(s) = self.snapped {
            if normalize(d - s).abs() <= SNAP_LEAVE {
                return s;
            }
            self.snapped = None;
        }
        let nearest = normalize((d / 90.0).round() * 90.0);
        if normalize(d - nearest).abs() <= SNAP_ENTER {
            self.snapped = Some(nearest);
            return nearest;
        }
        d
    }

    fn track(&mut self, tilt: Option<(i32, i32)>) -> f32 {
        match tilt {
            // The reference client uses angle 90° here ("turn off rotation if
            // tilted up or down"). The device seems to send exactly this value
            // when the tilt is not usable, so it is not averaged.
            Some((0, 1024)) => {
                self.smoothed = None;
                self.degrees = normalize(-90.0 - 90.0);
            }
            Some((x, y)) => {
                let (x, y) = (x as f32, y as f32);
                let (sx, sy) = match self.smoothed {
                    Some((sx, sy)) => (sx + SMOOTHING * (x - sx), sy + SMOOTHING * (y - sy)),
                    None => (x, y),
                };
                self.smoothed = Some((sx, sy));
                if sx != 0.0 || sy != 0.0 {
                    self.degrees = rotation_for(sx, sy);
                }
            }
            None => {}
        }
        self.degrees
    }
}

/// Canvas for showing `src` rotated on a `screen`: the whole screen, at the
/// scale of the unrotated image (so the zoom never changes with the angle).
/// Corners that leave the screen are cut off.
pub fn canvas_size(src: (usize, usize), screen: (usize, usize)) -> (usize, usize) {
    let (w, h) = (src.0 as f32, src.1 as f32);
    let (sw, sh) = (screen.0 as f32, screen.1 as f32);
    // Screen pixels per image pixel. The tolerance absorbs float noise.
    let scale = (sw / w).min(sh / h);
    let up = |v: f32| (v - 0.01).ceil() as usize;
    (up(sw / scale), up(sh / scale))
}

/// Range of steps `t` in 0..n for which `(p0 + t * step) >> 16` may lie in
/// 0..limit; one step of margin on each side (the loop still checks).
fn span(p0: i32, step: i32, limit: usize, n: usize) -> (usize, usize) {
    let hi_pos = ((limit as i64) << 16) as f64;
    if step == 0 {
        return if p0 >= 0 && (p0 as f64) < hi_pos {
            (0, n)
        } else {
            (0, 0)
        };
    }
    let a = -(p0 as f64) / step as f64;
    let b = (hi_pos - p0 as f64) / step as f64;
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let lo = (lo.floor() - 1.0).clamp(0.0, n as f64) as usize;
    let hi = (hi.ceil() + 1.0).clamp(0.0, n as f64) as usize;
    (lo, hi.max(lo))
}

/// Rotates `src` by `degrees` (counterclockwise) about its center into `dst`
/// of `size`, centered there, at scale 1. Nearest neighbour; pixels outside
/// are black.
pub fn rotate_into(src: &Image, dst: &mut Image, degrees: f32, size: (usize, usize)) {
    let (w, h) = (src.width, src.height);
    let (dw, dh) = size;
    dst.width = dw;
    dst.height = dh;
    dst.data.resize(dw * dh * 4, 0);

    // Inverse mapping for a destination offset (dx, dy) from the center
    // (y down, counterclockwise positive as in OpenCV):
    //   sx = cx + cos*dx - sin*dy,  sy = cy + sin*dx + cos*dy
    // in 16.16 fixed point, stepping (cos, sin) along each row.
    const ONE: f32 = 65536.0;
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (step_x, step_y) = ((cos * ONE) as i32, (sin * ONE) as i32);
    let dx0 = 0.5 - dw as f32 / 2.0;

    for y in 0..dh {
        let dy = y as f32 + 0.5 - dh as f32 / 2.0;
        let sx0 = ((cx + cos * dx0 - sin * dy) * ONE) as i32;
        let sy0 = ((cy + sin * dx0 + cos * dy) * ONE) as i32;
        let row = &mut dst.data[y * dw * 4..(y + 1) * dw * 4];
        // Only the part of the row that can hit the source is sampled.
        let (ax, bx) = span(sx0, step_x, w, dw);
        let (ay, by) = span(sy0, step_y, h, dw);
        let (lo, hi) = (ax.max(ay), bx.min(by).max(ax.max(ay)));
        row[..lo * 4].fill(0);
        row[hi * 4..].fill(0);
        let mut sx = sx0.wrapping_add(step_x.wrapping_mul(lo as i32));
        let mut sy = sy0.wrapping_add(step_y.wrapping_mul(lo as i32));
        for px in row[lo * 4..hi * 4].chunks_exact_mut(4) {
            let (ix, iy) = (sx >> 16, sy >> 16);
            if (ix as u32) < w as u32 && (iy as u32) < h as u32 {
                let o = (iy as usize * w + ix as usize) * 4;
                px.copy_from_slice(&src.data[o..o + 4]);
            } else {
                px.copy_from_slice(&[0, 0, 0, 0]);
            }
            sx += step_x;
            sy += step_y;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        normalize(a - b).abs() < 1.0
    }

    #[test]
    fn angle_rule_from_spec() {
        assert!(close(rotation_for(-1024.0, 0.0), 0.0));
        assert!(close(rotation_for(0.0, 1000.0), -90.0));
        assert!(close(rotation_for(1024.0, 0.0), 180.0));
        assert!(close(rotation_for(0.0, -1024.0), 90.0));
        let mut l = Leveler::new(Mode::Free);
        assert!(
            close(l.update(Some((0, 1024))), 180.0),
            "special case: angle 90"
        );
    }

    #[test]
    fn leveler_keeps_last_and_can_be_disabled() {
        let mut l = Leveler::new(Mode::Free);
        assert_eq!(l.update(None), 0.0);
        let d = l.update(Some((-1024, 0)));
        assert!(close(l.update(None), d));
        assert_eq!(Leveler::new(Mode::Off).update(Some((1024, 0))), 0.0);
    }

    #[test]
    fn smoothing_across_180() {
        // Around rotation 90° (x ≈ 0, y < 0) the angle of single samples jumps
        // between about ±180° + 90°; the vector average must not.
        let mut l = Leveler::new(Mode::Free);
        for i in 0..50 {
            let x = if i % 2 == 0 { 30 } else { -30 };
            let d = l.update(Some((x, -1000)));
            assert!(close(d, 90.0) || i == 0, "sample {i}: {d}");
        }
        // converges to a new direction
        for _ in 0..30 {
            l.update(Some((-1024, 0)));
        }
        assert!(close(l.update(None), 0.0));
    }

    /// Tilt vector for a given rotation (inverse of rotation_for).
    fn tilt_for(deg: f32) -> Option<(i32, i32)> {
        let a = (-deg - 90.0).to_radians();
        Some((
            (a.sin() * 1024.0).round() as i32,
            (a.cos() * 1024.0).round() as i32,
        ))
    }

    #[test]
    fn snap_with_hysteresis() {
        let mut free = Leveler::new(Mode::Free);
        assert!(close(free.update(tilt_for(5.0)), 5.0));

        let mut l = Leveler::new(Mode::Snap);
        let mut settle = |deg: f32| {
            for _ in 0..40 {
                l.update(tilt_for(deg));
            }
            l.update(None)
        };
        assert_eq!(settle(5.0), 0.0);
        assert_eq!(settle(-7.0), 0.0);
        assert_eq!(settle(10.0), 0.0, "stays snapped below SNAP_LEAVE");
        assert!(close(settle(20.0), 20.0));
        assert!(close(settle(10.0), 10.0), "re-snaps only within SNAP_ENTER");
        assert_eq!(settle(84.0), 90.0);
        assert_eq!(settle(-95.0), -90.0);
        assert_eq!(settle(176.0).abs(), 180.0);
        assert_eq!(settle(-176.0).abs(), 180.0);
        assert!(close(settle(45.0), 45.0));
    }

    fn marked(w: usize, h: usize) -> Image {
        let mut img = Image::new(w, h);
        for (i, px) in img.data.chunks_exact_mut(4).enumerate() {
            px.copy_from_slice(&(i as u32 + 1).to_le_bytes());
        }
        img
    }

    fn at(img: &Image, x: usize, y: usize) -> u32 {
        let o = (y * img.width + x) * 4;
        u32::from_le_bytes(img.data[o..o + 4].try_into().unwrap())
    }

    #[test]
    fn rotate_identity_and_180() {
        let src = marked(8, 4);
        let mut dst = Image::default();
        rotate_into(&src, &mut dst, 0.0, (8, 4));
        assert_eq!(dst.data, src.data);
        rotate_into(&src, &mut dst, 180.0, (8, 4));
        assert_eq!(at(&dst, 7, 3), at(&src, 0, 0));
        assert_eq!(at(&dst, 0, 0), at(&src, 7, 3));
    }

    #[test]
    fn rotate_90_is_counterclockwise() {
        let src = marked(4, 4);
        let mut dst = Image::default();
        rotate_into(&src, &mut dst, 90.0, (4, 4));
        // counterclockwise: the top-right corner moves to the top-left
        assert_eq!(at(&dst, 0, 0), at(&src, 3, 0));
        assert_eq!(at(&dst, 0, 3), at(&src, 0, 0));
        let mut back = Image::default();
        rotate_into(&dst, &mut back, -90.0, (4, 4));
        assert_eq!(back.data, src.data);
    }

    #[test]
    fn rotate_45_has_black_corners() {
        let src = Image {
            width: 64,
            height: 36,
            data: vec![0xff; 64 * 36 * 4],
        };
        let mut dst = Image::default();
        rotate_into(&src, &mut dst, 45.0, (64, 36));
        assert_eq!(at(&dst, 0, 0), 0);
        assert_eq!(at(&dst, 32, 18), 0xffff_ffff);
    }

    #[test]
    fn canvas_is_the_screen_at_unrotated_scale() {
        let src = (640, 360);
        assert_eq!(canvas_size(src, (1920, 1080)), (640, 360));
        assert_eq!(canvas_size(src, (1280, 720)), (640, 360));
        assert_eq!(canvas_size(src, (1024, 768)), (640, 480));
        assert_eq!(canvas_size(src, (2560, 1080)), (854, 360), "21:9 screen");
    }

    #[test]
    fn rotated_image_uses_the_whole_canvas() {
        // 90° on a 4:3 screen: the image stands upright at full scale, cut at
        // the top and bottom of the screen, black left and right.
        let (w, h) = (64, 36);
        let src = Image {
            width: w,
            height: h,
            data: vec![0xff; w * h * 4],
        };
        let size = canvas_size((w, h), (1024, 768));
        assert_eq!(size, (64, 48));
        let mut dst = Image::default();
        rotate_into(&src, &mut dst, 90.0, size);
        assert_eq!(at(&dst, 32, 0), 0xffff_ffff);
        assert_eq!(at(&dst, 32, 47), 0xffff_ffff);
        assert_eq!(at(&dst, 0, 24), 0);
        assert_eq!(at(&dst, 63, 24), 0);
        // the upright image is 36 wide
        let row = (0..64).filter(|&x| at(&dst, x, 24) != 0).count();
        assert!((35..=37).contains(&row), "{row}");
    }
}
