//! Fullscreen output via DRM/KMS (atomic API, no display server, no fbdev).
//!
//! Preferred path: the dumb buffers have the image size (640×360) and the primary
//! plane scales them to the screen in hardware (vc4 HVS). If the driver refuses a
//! scaled plane, the buffers have the mode size and we scale on the CPU.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::{AsFd, BorrowedFd};

use drm::buffer::DrmFourcc;
use drm::control::{
    atomic, connector, crtc, dumbbuffer::DumbBuffer, framebuffer, plane, property,
    AtomicCommitFlags, Device as ControlDevice, ModeTypeFlags, PlaneType,
};
use drm::Device as BasicDevice;
use log::{info, warn};

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
struct Rect {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
}

pub struct Display {
    card: Card,
    connector: connector::Handle,
    crtc: crtc::Handle,
    plane: plane::Handle,
    mode: drm::control::Mode,
    props: HashMap<(u32, &'static str), property::Handle>,
    buffers: Vec<Buffer>,
    /// Size of the dumb buffers.
    buf_size: (usize, usize),
    /// Minimum buffer size with hardware scaling, so images up to this size
    /// need no reallocation (the plane shows only the used part).
    reserve: (usize, usize),
    back: usize,
    /// None until the first commit; false after hardware scaling was refused.
    hw_scaling: Option<bool>,
    modeset_done: bool,
}

fn other(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::Other, msg.into())
}

/// Largest rectangle with the aspect ratio of `src` that fits into `dst`, centered.
fn letterbox(src: (usize, usize), dst: (usize, usize)) -> Rect {
    let (sw, sh) = (src.0 as u64, src.1 as u64);
    let (dw, dh) = (dst.0 as u64, dst.1 as u64);
    let (w, h) = if dw * sh <= dh * sw {
        (dw, dw * sh / sw)
    } else {
        (dh * sw / sh, dh)
    };
    Rect {
        x: ((dw - w) / 2) as i32,
        y: ((dh - h) / 2) as i32,
        w: w as u32,
        h: h as u32,
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

        let primary = (PlaneType::Primary as u32).into();
        let plane = card
            .plane_handles()?
            .iter()
            .copied()
            .filter(|&p| {
                card.get_plane(p)
                    .is_ok_and(|info| res.filter_crtcs(info.possible_crtcs()).contains(&crtc))
            })
            .find(|&p| {
                card.get_properties(p).is_ok_and(|props| {
                    props.iter().any(|(&id, &val)| {
                        card.get_property(id)
                            .is_ok_and(|i| i.name().to_bytes() == b"type")
                            && val == primary
                    })
                })
            })
            .ok_or_else(|| other("no primary plane"))?;

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

        let (w, h) = mode.size();
        info!(
            "{path}: {:?} {}x{}@{} on crtc {crtc:?}",
            con.interface(),
            w,
            h,
            mode.vrefresh()
        );
        Ok(Display {
            card,
            connector: con.handle(),
            crtc,
            plane,
            mode,
            props,
            buffers: Vec::new(),
            buf_size: (0, 0),
            reserve: (0, 0),
            back: 0,
            hw_scaling: None,
            modeset_done: false,
        })
    }

    pub fn mode_size(&self) -> (usize, usize) {
        let (w, h) = self.mode.size();
        (w as usize, h as usize)
    }

    fn prop(&self, obj: impl Into<u32>, name: &'static str) -> io::Result<property::Handle> {
        self.props
            .get(&(obj.into(), name))
            .copied()
            .ok_or_else(|| other(format!("missing property {name}")))
    }

    /// Images up to `size` will be shown without reallocating the buffers.
    pub fn reserve(&mut self, size: (usize, usize)) {
        self.reserve = (self.reserve.0.max(size.0), self.reserve.1.max(size.1));
    }

    /// Makes the buffers `size` exactly, or (not `exact`) at least `size`.
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
        for b in self.buffers.drain(..) {
            let _ = self.card.destroy_framebuffer(b.fb);
            let _ = self.card.destroy_dumb_buffer(b.db);
        }
        for _ in 0..2 {
            let mut db = self.card.create_dumb_buffer(
                (size.0 as u32, size.1 as u32),
                DrmFourcc::Xrgb8888,
                32,
            )?;
            self.card.map_dumb_buffer(&mut db)?.as_mut().fill(0);
            let fb = self.card.add_framebuffer(&db, 24, 32)?;
            self.buffers.push(Buffer { db, fb });
        }
        self.buf_size = size;
        Ok(())
    }

    fn commit(
        &mut self,
        src: (usize, usize),
        dst: Rect,
        flags: AtomicCommitFlags,
    ) -> io::Result<()> {
        let fb = self.buffers[self.back].fb;
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
        let p = self.plane;
        use property::Value::{SignedRange as S, UnsignedRange as U};
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
        let flags = if self.modeset_done {
            flags
        } else {
            flags | AtomicCommitFlags::ALLOW_MODESET
        };
        self.card.atomic_commit(flags, req)?;
        Ok(())
    }

    /// Shows `img` fullscreen (aspect ratio kept). Blocks until the flip is done.
    pub fn present(&mut self, img: &Image) -> io::Result<()> {
        let src = (img.width, img.height);
        let mode = self.mode_size();

        if self.hw_scaling.is_none() {
            self.ensure_buffers(src, false)?;
            let ok = match self.commit(src, letterbox(src, mode), AtomicCommitFlags::TEST_ONLY) {
                Ok(()) => true,
                // EINVAL/ERANGE: the driver refuses this plane configuration.
                Err(e) if matches!(e.raw_os_error(), Some(22 | 34)) => false,
                Err(e) => return Err(e),
            };
            info!(
                "hardware plane scaling: {}",
                if ok { "yes" } else { "no, using CPU" }
            );
            self.hw_scaling = Some(ok);
        }

        if self.hw_scaling == Some(true) {
            self.ensure_buffers(src, false)?;
            self.blit(
                img,
                Rect {
                    x: 0,
                    y: 0,
                    w: src.0 as u32,
                    h: src.1 as u32,
                },
            )?;
            self.commit(src, letterbox(src, mode), AtomicCommitFlags::empty())?;
        } else {
            self.ensure_buffers(mode, true)?;
            self.blit(img, letterbox(src, mode))?;
            let full = Rect {
                x: 0,
                y: 0,
                w: mode.0 as u32,
                h: mode.1 as u32,
            };
            self.commit(mode, full, AtomicCommitFlags::empty())?;
        }
        self.modeset_done = true;
        self.back ^= 1;
        Ok(())
    }

    /// Copies `img` into the back buffer at `dst`, nearest-neighbour scaled if needed.
    fn blit(&mut self, img: &Image, dst: Rect) -> io::Result<()> {
        use drm::buffer::Buffer as _;
        let back = &mut self.buffers[self.back];
        let pitch = back.db.pitch() as usize;
        let (bw, bh) = self.buf_size;
        let mut map = self.card.map_dumb_buffer(&mut back.db)?;
        let out = map.as_mut();
        let row_bytes = img.width * 4;

        if dst.w as usize == img.width && dst.h as usize == img.height {
            for y in 0..img.height {
                let o = (dst.y as usize + y) * pitch + dst.x as usize * 4;
                out[o..o + row_bytes]
                    .copy_from_slice(&img.data[y * row_bytes..(y + 1) * row_bytes]);
            }
            return Ok(());
        }

        // Clear the letterbox bars on every frame: both buffers alternate, and it is cheap.
        let (dx, dy, dw, dh) = (
            dst.x as usize,
            dst.y as usize,
            dst.w as usize,
            dst.h as usize,
        );
        let xs: Vec<usize> = (0..dw).map(|x| x * img.width / dw * 4).collect();
        for y in 0..bh {
            let row = &mut out[y * pitch..y * pitch + bw * 4];
            if y < dy || y >= dy + dh {
                row.fill(0);
                continue;
            }
            row[..dx * 4].fill(0);
            row[(dx + dw) * 4..].fill(0);
            let src_row = &img.data[(y - dy) * img.height / dh * row_bytes..][..row_bytes];
            for (px, &sx) in row[dx * 4..(dx + dw) * 4].chunks_exact_mut(4).zip(&xs) {
                px.copy_from_slice(&src_row[sx..sx + 4]);
            }
        }
        Ok(())
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        for b in self.buffers.drain(..) {
            let _ = self.card.destroy_framebuffer(b.fb);
            if let Err(e) = self.card.destroy_dumb_buffer(b.db) {
                warn!("destroy dumb buffer: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_fits() {
        assert_eq!(
            letterbox((640, 360), (1920, 1080)),
            Rect {
                x: 0,
                y: 0,
                w: 1920,
                h: 1080
            }
        );
        assert_eq!(
            letterbox((640, 360), (1280, 1024)),
            Rect {
                x: 0,
                y: 152,
                w: 1280,
                h: 720
            }
        );
        assert_eq!(
            letterbox((640, 360), (2560, 1080)),
            Rect {
                x: 320,
                y: 0,
                w: 1920,
                h: 1080
            }
        );
    }
}
