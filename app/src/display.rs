//! Fullscreen output via DRM/KMS (atomic API, no display server, no fbdev).
//!
//! The LCARS frame (`lcars`) surrounds the picture. Preferred path: the frame
//! is on a full-screen ARGB overlay plane, transparent over the viewport, and
//! the primary plane below shows the picture at its own size (640×360),
//! scaled into the viewport in hardware (vc4 HVS). The frame is redrawn only
//! where it changes. If the driver refuses that, the buffers have the mode
//! size and we put the frame and the scaled picture together on the CPU.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::{AsFd, BorrowedFd};

use drm::buffer::{Buffer as _, DrmFourcc};
use drm::control::{
    atomic, connector, crtc, dumbbuffer::DumbBuffer, framebuffer, plane, property,
    AtomicCommitFlags, Device as ControlDevice, ModeTypeFlags, PlaneType,
};
use drm::Device as BasicDevice;
use log::{info, warn};

use crate::lcars::{self, Layout, Rect};

/// A BGRA image in memory (4 bytes per pixel, no row padding).
#[derive(Clone, Default)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub data: Vec<u8>,
}

impl Image {
    pub fn new(width: usize, height: usize) -> Image {
        Image {
            width,
            height,
            data: vec![0; width * height * 4],
        }
    }
}

struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}
impl BasicDevice for Card {}
impl ControlDevice for Card {}

struct Buffer {
    db: DumbBuffer,
    fb: framebuffer::Handle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Path {
    /// Frame on the overlay plane, picture scaled by the primary plane.
    Overlay,
    /// Frame and scaled picture put together on the CPU, at the mode size.
    Composite,
}

/// The LCARS frame, rendered at 1/`scale` of the screen size.
struct Chrome {
    scale: usize,
    layout: Layout,
    state: Option<lcars::State>,
    img: Image,
    /// Overlay path: the overlay plane's buffers.
    buffers: Vec<Buffer>,
    front: usize,
    /// Per destination buffer (overlay or primary): rectangles of `img`
    /// changed since that buffer was last updated.
    pending: [Vec<Rect>; 2],
}

pub struct Display {
    card: Card,
    connector: connector::Handle,
    crtc: crtc::Handle,
    plane: plane::Handle,
    overlay: Option<plane::Handle>,
    mode: drm::control::Mode,
    props: HashMap<(u32, &'static str), property::Handle>,
    buffers: Vec<Buffer>,
    /// Size of the primary dumb buffers.
    buf_size: (usize, usize),
    /// Minimum buffer size on the overlay path, so pictures up to this size
    /// need no reallocation (the plane shows only the used part).
    reserve: (usize, usize),
    back: usize,
    /// The primary buffer on screen and the picture size in it.
    shown: Option<(usize, (usize, usize))>,
    /// Composite path: the last picture, to redraw it under a changed frame.
    last: Option<Image>,
    /// None until the first `present`.
    path: Option<Path>,
    chrome: Chrome,
    modeset_done: bool,
}

fn other(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg.into())
}

/// Largest rectangle with the aspect ratio of `src` that fits into `dst`, centered.
fn letterbox(src: (usize, usize), dst: Rect) -> Rect {
    let (sw, sh) = (src.0 as u64, src.1 as u64);
    let (dw, dh) = (dst.w as u64, dst.h as u64);
    let (w, h) = if dw * sh <= dh * sw {
        (dw, dw * sh / sw)
    } else {
        (dh * sw / sh, dh)
    };
    Rect {
        x: dst.x + ((dw - w) / 2) as usize,
        y: dst.y + ((dh - h) / 2) as usize,
        w: w as usize,
        h: h as usize,
    }
}

fn scaled(r: Rect, s: usize) -> Rect {
    Rect {
        x: r.x * s,
        y: r.y * s,
        w: r.w * s,
        h: r.h * s,
    }
}

/// Copies `r` of `img` into a mapped buffer, each pixel as a `scale`² block,
/// leaving out `hole` (in buffer pixels).
fn copy_scaled(out: &mut [u8], pitch: usize, img: &Image, r: Rect, scale: usize, hole: Rect) {
    for y in r.y * scale..r.bottom() * scale {
        let src = &img.data[(y / scale * img.width) * 4..][..img.width * 4];
        let row = &mut out[y * pitch..];
        let in_hole = y >= hole.y && y < hole.bottom();
        for x in r.x * scale..r.right() * scale {
            if in_hole && x >= hole.x && x < hole.right() {
                continue;
            }
            row[x * 4..x * 4 + 4].copy_from_slice(&src[x / scale * 4..x / scale * 4 + 4]);
        }
    }
}

impl Display {
    /// Opens the first DRM card with a connected output.
    pub fn open() -> io::Result<Display> {
        let mut last_err = other("no DRM device found");
        for i in 0..4 {
            let path = format!("/dev/dri/card{i}");
            match Self::open_card(&path) {
                Ok(d) => return Ok(d),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => last_err = other(format!("{path}: {e}")),
            }
        }
        Err(last_err)
    }

    fn open_card(path: &str) -> io::Result<Display> {
        let card = Card(OpenOptions::new().read(true).write(true).open(path)?);
        card.set_client_capability(drm::ClientCapability::UniversalPlanes, true)?;
        card.set_client_capability(drm::ClientCapability::Atomic, true)?;
        let res = card.resource_handles()?;

        let con = res
            .connectors()
            .iter()
            .filter_map(|&c| card.get_connector(c, true).ok())
            .find(|c| c.state() == connector::State::Connected && !c.modes().is_empty())
            .ok_or_else(|| other("no connected display"))?;
        let mode = *con
            .modes()
            .iter()
            .find(|m| m.mode_type().contains(ModeTypeFlags::PREFERRED))
            .unwrap_or(&con.modes()[0]);

        let crtc = con
            .encoders()
            .iter()
            .filter_map(|&e| card.get_encoder(e).ok())
            .flat_map(|e| res.filter_crtcs(e.possible_crtcs()))
            .next()
            .ok_or_else(|| other("no CRTC for connector"))?;

        let on_crtc: Vec<plane::Handle> = card
            .plane_handles()?
            .iter()
            .copied()
            .filter(|&p| {
                card.get_plane(p)
                    .is_ok_and(|info| res.filter_crtcs(info.possible_crtcs()).contains(&crtc))
            })
            .collect();
        let is_type = |p: plane::Handle, t: PlaneType| {
            let t = (t as u32).into();
            card.get_properties(p).is_ok_and(|props| {
                props.iter().any(|(&id, &val)| {
                    card.get_property(id)
                        .is_ok_and(|i| i.name().to_bytes() == b"type")
                        && val == t
                })
            })
        };
        let plane = on_crtc
            .iter()
            .copied()
            .find(|&p| is_type(p, PlaneType::Primary))
            .ok_or_else(|| other("no primary plane"))?;
        // For the LCARS frame; optional.
        let overlay = on_crtc.iter().copied().find(|&p| {
            is_type(p, PlaneType::Overlay)
                && card
                    .get_plane(p)
                    .is_ok_and(|i| i.formats().contains(&(DrmFourcc::Argb8888 as u32)))
        });

        let mut props = HashMap::new();
        let mut load = |id: u32, handles: drm::control::PropertyValueSet| -> io::Result<()> {
            for (&prop, _) in handles.iter() {
                let info = card.get_property(prop)?;
                const NAMES: &[&str] = &[
                    "CRTC_ID", "MODE_ID", "ACTIVE", "FB_ID", "SRC_X", "SRC_Y", "SRC_W", "SRC_H",
                    "CRTC_X", "CRTC_Y", "CRTC_W", "CRTC_H",
                ];
                if let Some(&n) = NAMES
                    .iter()
                    .find(|&&n| info.name().to_bytes() == n.as_bytes())
                {
                    props.insert((id, n), prop);
                }
            }
            Ok(())
        };
        load(con.handle().into(), card.get_properties(con.handle())?)?;
        load(crtc.into(), card.get_properties(crtc)?)?;
        load(plane.into(), card.get_properties(plane)?)?;
        if let Some(o) = overlay {
            load(o.into(), card.get_properties(o)?)?;
        }

        let (w, h) = mode.size();
        info!(
            "{path}: {:?} {}x{}@{} on crtc {crtc:?}",
            con.interface(),
            w,
            h,
            mode.vrefresh()
        );
        // Half resolution on large screens: a quarter of the memory traffic
        // for scanning out the overlay, and the frame is made of big shapes.
        let scale = if h >= 1000 { 2 } else { 1 };
        let size = (w as usize / scale, h as usize / scale);
        Ok(Display {
            card,
            connector: con.handle(),
            crtc,
            plane,
            overlay,
            mode,
            props,
            buffers: Vec::new(),
            buf_size: (0, 0),
            reserve: (0, 0),
            back: 0,
            shown: None,
            last: None,
            path: None,
            chrome: Chrome {
                scale,
                layout: Layout::new(size),
                state: None,
                img: Image::new(size.0, size.1),
                buffers: Vec::new(),
                front: 0,
                pending: [Vec::new(), Vec::new()],
            },
            modeset_done: false,
        })
    }

    pub fn mode_size(&self) -> (usize, usize) {
        let (w, h) = self.mode.size();
        (w as usize, h as usize)
    }

    /// Size of the area inside the frame, in screen pixels.
    pub fn viewport_size(&self) -> (usize, usize) {
        let v = scaled(self.chrome.layout.viewport, self.chrome.scale);
        (v.w, v.h)
    }

    fn viewport(&self) -> Rect {
        scaled(self.chrome.layout.viewport, self.chrome.scale)
    }

    fn prop(&self, obj: impl Into<u32>, name: &'static str) -> io::Result<property::Handle> {
        self.props
            .get(&(obj.into(), name))
            .copied()
            .ok_or_else(|| other(format!("missing property {name}")))
    }

    /// Pictures up to `size` will be shown without reallocating the buffers.
    pub fn reserve(&mut self, size: (usize, usize)) {
        self.reserve = (self.reserve.0.max(size.0), self.reserve.1.max(size.1));
    }

    fn make_buffers(&mut self, size: (usize, usize), format: DrmFourcc) -> io::Result<Vec<Buffer>> {
        let (depth, bpp) = if format == DrmFourcc::Argb8888 {
            (32, 32)
        } else {
            (24, 32)
        };
        let mut v = Vec::new();
        for _ in 0..2 {
            let mut db =
                self.card
                    .create_dumb_buffer((size.0 as u32, size.1 as u32), format, 32)?;
            self.card.map_dumb_buffer(&mut db)?.as_mut().fill(0);
            let fb = self.card.add_framebuffer(&db, depth, bpp)?;
            v.push(Buffer { db, fb });
        }
        Ok(v)
    }

    fn free_buffers(&mut self, buffers: Vec<Buffer>) {
        for b in buffers {
            let _ = self.card.destroy_framebuffer(b.fb);
            if let Err(e) = self.card.destroy_dumb_buffer(b.db) {
                warn!("destroy dumb buffer: {e}");
            }
        }
    }

    /// Makes the primary buffers `size` exactly, or (not `exact`) at least `size`.
    fn ensure_buffers(&mut self, size: (usize, usize), exact: bool) -> io::Result<()> {
        let (bw, bh) = self.buf_size;
        let fits = if exact {
            self.buf_size == size
        } else {
            bw >= size.0 && bh >= size.1
        };
        if fits && !self.buffers.is_empty() {
            return Ok(());
        }
        let size = if exact {
            size
        } else {
            (size.0.max(self.reserve.0), size.1.max(self.reserve.1))
        };
        let old = std::mem::take(&mut self.buffers);
        self.free_buffers(old);
        self.buffers = self.make_buffers(size, DrmFourcc::Xrgb8888)?;
        self.buf_size = size;
        self.shown = None;
        if self.path == Some(Path::Composite) {
            // New buffers hold no frame yet.
            let full = self.chrome.layout.full();
            self.chrome.pending = [vec![full], vec![full]];
        }
        Ok(())
    }

    /// Sets the planes: the primary shows `src` of buffer `buf` at `dst`;
    /// the overlay shows the frame on the overlay path and is off otherwise.
    fn commit(
        &mut self,
        buf: usize,
        src: (usize, usize),
        dst: Rect,
        flags: AtomicCommitFlags,
    ) -> io::Result<()> {
        use property::Value::{SignedRange as S, UnsignedRange as U};
        let mut req = atomic::AtomicModeReq::new();
        if !self.modeset_done {
            let blob = self.card.create_property_blob(&self.mode)?;
            req.add_property(
                self.connector,
                self.prop(self.connector, "CRTC_ID")?,
                property::Value::CRTC(Some(self.crtc)),
            );
            req.add_property(self.crtc, self.prop(self.crtc, "MODE_ID")?, blob);
            req.add_property(
                self.crtc,
                self.prop(self.crtc, "ACTIVE")?,
                property::Value::Boolean(true),
            );
        }
        let mut set = |p: plane::Handle,
                       fb: framebuffer::Handle,
                       src: (usize, usize),
                       dst: Rect|
         -> io::Result<()> {
            req.add_property(
                p,
                self.prop(p, "FB_ID")?,
                property::Value::Framebuffer(Some(fb)),
            );
            req.add_property(
                p,
                self.prop(p, "CRTC_ID")?,
                property::Value::CRTC(Some(self.crtc)),
            );
            req.add_property(p, self.prop(p, "SRC_X")?, U(0));
            req.add_property(p, self.prop(p, "SRC_Y")?, U(0));
            req.add_property(p, self.prop(p, "SRC_W")?, U((src.0 as u64) << 16));
            req.add_property(p, self.prop(p, "SRC_H")?, U((src.1 as u64) << 16));
            req.add_property(p, self.prop(p, "CRTC_X")?, S(dst.x as i64));
            req.add_property(p, self.prop(p, "CRTC_Y")?, S(dst.y as i64));
            req.add_property(p, self.prop(p, "CRTC_W")?, U(dst.w as u64));
            req.add_property(p, self.prop(p, "CRTC_H")?, U(dst.h as u64));
            Ok(())
        };
        set(self.plane, self.buffers[buf].fb, src, dst)?;
        if let Some(o) = self.overlay {
            match self.chrome.buffers.get(self.chrome.front) {
                Some(b) if self.path != Some(Path::Composite) => {
                    let (w, h) = self.mode_size();
                    let full = Rect { x: 0, y: 0, w, h };
                    set(o, b.fb, self.chrome.layout.size(), full)?;
                }
                _ => {
                    req.add_property(
                        o,
                        self.prop(o, "FB_ID")?,
                        property::Value::Framebuffer(None),
                    );
                    req.add_property(o, self.prop(o, "CRTC_ID")?, property::Value::CRTC(None));
                }
            }
        }
        let flags = if self.modeset_done {
            flags
        } else {
            flags | AtomicCommitFlags::ALLOW_MODESET
        };
        self.card.atomic_commit(flags, req)?;
        Ok(())
    }

    /// Tries the overlay path with `img`; on success it stays set up.
    fn try_overlay(&mut self, img: &Image) -> io::Result<bool> {
        if self.overlay.is_none() {
            return Ok(false);
        }
        let src = (img.width, img.height);
        self.ensure_buffers(src, false)?;
        self.chrome.buffers = self.make_buffers(self.chrome.layout.size(), DrmFourcc::Argb8888)?;
        let dst = letterbox(src, self.viewport());
        match self.commit(0, src, dst, AtomicCommitFlags::TEST_ONLY) {
            Ok(()) => Ok(true),
            // EINVAL/ERANGE: the driver refuses this plane configuration.
            Err(e) if matches!(e.raw_os_error(), Some(22 | 34)) => {
                let b = std::mem::take(&mut self.chrome.buffers);
                self.free_buffers(b);
                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    /// Updates the frame image to `state` and records what changed. False if
    /// nothing did.
    fn update_chrome(&mut self, state: &lcars::State) -> bool {
        let c = &mut self.chrome;
        let rects = match &c.state {
            Some(old) => c.layout.changed(old, state),
            None => vec![c.layout.full()],
        };
        for &r in &rects {
            lcars::render(&mut c.img, &c.layout, state, r);
        }
        for p in &mut c.pending {
            p.extend_from_slice(&rects);
        }
        c.state = Some(state.clone());
        !rects.is_empty()
    }

    /// Shows `img` (None: the last one) inside the LCARS frame described by
    /// `state`. Returns without a commit if nothing changed. Blocks until
    /// the flip is done.
    pub fn present(&mut self, img: Option<&Image>, state: &lcars::State) -> io::Result<()> {
        let mode = self.mode_size();
        if self.path.is_none() {
            let Some(img) = img else {
                return Ok(());
            };
            let ok = self.try_overlay(img)?;
            info!(
                "LCARS frame on the overlay plane, picture scaled by the hardware: {}",
                if ok { "yes" } else { "no, using CPU" }
            );
            self.path = Some(if ok { Path::Overlay } else { Path::Composite });
        }
        let changed = self.update_chrome(state);
        if img.is_none() && !changed {
            return Ok(());
        }
        let Some((img_size, buf)) = img
            .map(|i| ((i.width, i.height), self.back))
            .or(self.shown.map(|(b, s)| (s, b)))
        else {
            return Ok(());
        };

        let vp = self.viewport();
        let (fb_size, dst) = match self.path {
            Some(Path::Overlay) => {
                if let Some(img) = img {
                    self.ensure_buffers(img_size, false)?;
                    let r = Rect {
                        x: 0,
                        y: 0,
                        w: img.width,
                        h: img.height,
                    };
                    self.blit(img, r, r)?;
                }
                let c = &mut self.chrome;
                let next = c.front ^ 1;
                if !c.pending[next].is_empty() {
                    let db = &mut c.buffers[next].db;
                    let pitch = db.pitch() as usize;
                    let mut map = self.card.map_dumb_buffer(db)?;
                    for r in c.pending[next].drain(..) {
                        copy_scaled(map.as_mut(), pitch, &c.img, r, 1, Rect::default());
                    }
                    c.front = next;
                }
                (img_size, letterbox(img_size, vp))
            }
            _ => {
                self.ensure_buffers(mode, true)?;
                if let Some(img) = img {
                    self.last = Some(img.clone());
                }
                let last = self.last.take().unwrap();
                self.blit(&last, letterbox(img_size, vp), vp)?;
                self.last = Some(last);
                let c = &mut self.chrome;
                let db = &mut self.buffers[self.back].db;
                let pitch = db.pitch() as usize;
                let mut map = self.card.map_dumb_buffer(db)?;
                for r in c.pending[self.back].drain(..) {
                    copy_scaled(map.as_mut(), pitch, &c.img, r, c.scale, vp);
                }
                let full = Rect {
                    x: 0,
                    y: 0,
                    w: mode.0,
                    h: mode.1,
                };
                (mode, full)
            }
        };
        let buf = if self.path == Some(Path::Composite) {
            self.back
        } else {
            buf
        };

        self.commit(buf, fb_size, dst, AtomicCommitFlags::empty())?;
        self.modeset_done = true;
        self.shown = Some((buf, img_size));
        if buf == self.back {
            self.back ^= 1;
        }
        Ok(())
    }

    /// Copies `img` into the back buffer at `dst`, nearest-neighbour scaled
    /// if needed, and clears the rest of `area` (which contains `dst`).
    fn blit(&mut self, img: &Image, dst: Rect, area: Rect) -> io::Result<()> {
        let back = &mut self.buffers[self.back];
        let pitch = back.db.pitch() as usize;
        let mut map = self.card.map_dumb_buffer(&mut back.db)?;
        let out = map.as_mut();
        let row_bytes = img.width * 4;

        if dst.w == img.width && dst.h == img.height {
            for y in 0..img.height {
                let o = (dst.y + y) * pitch + dst.x * 4;
                out[o..o + row_bytes]
                    .copy_from_slice(&img.data[y * row_bytes..(y + 1) * row_bytes]);
            }
            return Ok(());
        }

        // Clear the letterbox bars on every frame: both buffers alternate, and it is cheap.
        let xs: Vec<usize> = (0..dst.w).map(|x| x * img.width / dst.w * 4).collect();
        for y in area.y..area.bottom() {
            let row = &mut out[y * pitch + area.x * 4..y * pitch + area.right() * 4];
            if y < dst.y || y >= dst.bottom() {
                row.fill(0);
                continue;
            }
            let (l, r) = (dst.x - area.x, dst.right() - area.x);
            row[..l * 4].fill(0);
            row[r * 4..].fill(0);
            let src_row = &img.data[(y - dst.y) * img.height / dst.h * row_bytes..][..row_bytes];
            for (px, &sx) in row[l * 4..r * 4].chunks_exact_mut(4).zip(&xs) {
                px.copy_from_slice(&src_row[sx..sx + 4]);
            }
        }
        Ok(())
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        let b = std::mem::take(&mut self.chrome.buffers);
        self.free_buffers(b);
        let b = std::mem::take(&mut self.buffers);
        self.free_buffers(b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: usize, y: usize, w: usize, h: usize) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn letterbox_fits() {
        let screen = |w, h| rect(0, 0, w, h);
        assert_eq!(
            letterbox((640, 360), screen(1920, 1080)),
            screen(1920, 1080)
        );
        assert_eq!(
            letterbox((640, 360), screen(1280, 1024)),
            rect(0, 152, 1280, 720)
        );
        assert_eq!(
            letterbox((640, 360), screen(2560, 1080)),
            rect(320, 0, 1920, 1080)
        );
        assert_eq!(
            letterbox((640, 360), rect(100, 50, 640, 400)),
            rect(100, 70, 640, 360)
        );
    }

    #[test]
    fn copy_scaled_doubles_and_skips_the_hole() {
        let img = Image {
            width: 2,
            height: 1,
            data: vec![1, 1, 1, 1, 2, 2, 2, 2],
        };
        let pitch = 4 * 4;
        let mut out = vec![0u8; pitch * 2];
        copy_scaled(&mut out, pitch, &img, rect(0, 0, 2, 1), 2, rect(3, 1, 1, 1));
        let px = |x: usize, y: usize| out[y * pitch + x * 4];
        assert_eq!([px(0, 0), px(1, 0), px(2, 0), px(3, 0)], [1, 1, 2, 2]);
        assert_eq!([px(0, 1), px(1, 1), px(2, 1), px(3, 1)], [1, 1, 2, 0]);
    }
}
