//! Display thread: decodes the newest frame and shows it, or shows the status screen.

use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use log::{error, info, warn};

use crate::decode;
use crate::display::{Display, Image};
use crate::rotate::{self, Leveler};
use crate::session::Frame;
use crate::status;

/// After this long without video, the last frame is replaced by the status screen.
const LAST_FRAME_KEEP: Duration = Duration::from_secs(10);
const DISPLAY_RETRY: Duration = Duration::from_secs(2);
const STATUS_SIZE: (usize, usize) = (640, 360);
/// Frame interval of the moving "Searching" screen.
const ANIMATION_FRAME: Duration = Duration::from_millis(40);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Searching,
    Connecting(String),
    WaitingForVideo,
    Streaming,
}

impl Status {
    fn text(&self) -> (String, String) {
        match self {
            Status::Searching => (
                "Searching for otoscope...".into(),
                "Switch on the otoscope".into(),
            ),
            Status::Connecting(ssid) => ("Connecting...".into(), ssid.clone()),
            Status::WaitingForVideo => ("Waiting for video...".into(), String::new()),
            Status::Streaming => ("Connected".into(), String::new()),
        }
    }
}

struct State {
    status: Status,
    /// Newest undisplayed frame. A new frame replaces an older one that was not shown yet.
    frame: Option<Frame>,
    dirty: bool,
}

pub struct Ui {
    state: Mutex<State>,
    cv: Condvar,
}

impl Ui {
    pub fn new() -> Arc<Ui> {
        Arc::new(Ui {
            state: Mutex::new(State {
                status: Status::Searching,
                frame: None,
                dirty: true,
            }),
            cv: Condvar::new(),
        })
    }

    pub fn set_status(&self, status: Status) {
        let mut s = self.state.lock().unwrap();
        if s.status != status {
            info!("status: {status:?}");
            s.status = status;
            s.dirty = true;
            self.cv.notify_one();
        }
    }

    pub fn push_frame(&self, frame: Frame) {
        let mut s = self.state.lock().unwrap();
        s.frame = Some(frame);
        s.dirty = true;
        self.cv.notify_one();
    }

    pub fn spawn(
        self: &Arc<Self>,
        use_display: bool,
        rotate: rotate::Mode,
    ) -> thread::JoinHandle<()> {
        let ui = Arc::clone(self);
        thread::Builder::new()
            .name("display".into())
            .spawn(move || ui.run(use_display, rotate))
            .expect("spawn display thread")
    }

    fn run(&self, use_display: bool, rotate: rotate::Mode) {
        let mut display: Option<Display> = None;
        let mut last_open_try: Option<Instant> = None;
        let mut img = Image::new(STATUS_SIZE.0, STATUS_SIZE.1);
        let mut decoded = Image::default();
        let mut leveler = Leveler::new(rotate);
        // The last frame as shown (rotated), with the time it arrived.
        let mut last_frame: Option<(Image, Instant)> = None;
        let mut shown: Option<(Status, bool)> = None; // what the status path last drew
        let mut stats = Stats::new();
        let mut bouncer = status::Bouncer::new();
        // Time of the last animation frame while the "Searching" screen moves.
        let mut animating: Option<Instant> = None;

        loop {
            let (status, frame) = {
                let mut s = self.state.lock().unwrap();
                if !s.dirty {
                    let timeout = if animating.is_some() {
                        ANIMATION_FRAME
                    } else {
                        Duration::from_secs(1)
                    };
                    s = self.cv.wait_timeout(s, timeout).unwrap().0;
                }
                s.dirty = false;
                (s.status.clone(), s.frame.take())
            };

            if use_display
                && display.is_none()
                && last_open_try.is_none_or(|t| t.elapsed() >= DISPLAY_RETRY)
            {
                last_open_try = Some(Instant::now());
                match Display::open() {
                    Ok(d) => {
                        display = Some(d);
                        shown = None;
                    }
                    Err(e) => warn!("display: {e}"),
                }
            }

            // Set again below only while the "Searching" screen moves.
            let last_anim = animating.take();
            let to_show = if let Some(frame) = frame {
                let t = Instant::now();
                match decode::decode_bgra(&frame.jpeg, &mut decoded.data) {
                    Ok((w, h)) => {
                        let decode_time = t.elapsed();
                        decoded.width = w;
                        decoded.height = h;
                        let degrees = leveler.update(frame.tilt);
                        let t = Instant::now();
                        let mut frame_img = last_frame.take().map(|(i, _)| i).unwrap_or_default();
                        if degrees.abs() >= rotate::MIN_DEGREES {
                            let src = (w, h);
                            let screen = display.as_ref().map_or(src, |d| d.mode_size());
                            let size = rotate::canvas_size(src, screen);
                            // Switching between rotated (canvas) and level
                            // (decoded) frames must not reallocate the buffers.
                            if let Some(d) = display.as_mut() {
                                d.reserve(size);
                            }
                            rotate::rotate_into(&decoded, &mut frame_img, degrees, size);
                        } else {
                            std::mem::swap(&mut decoded, &mut frame_img);
                        }
                        stats.frame(decode_time, t.elapsed(), degrees);
                        last_frame = Some((frame_img, Instant::now()));
                        shown = None;
                        last_frame.as_ref().map(|(i, _)| i)
                    }
                    Err(e) => {
                        warn!("frame {}: decode failed: {e:?}", frame.img_number);
                        None
                    }
                }
            } else if status == Status::Streaming {
                None // keep the last frame on screen
            } else {
                let recent = last_frame
                    .as_ref()
                    .is_some_and(|(_, t)| t.elapsed() < LAST_FRAME_KEEP);
                if status == Status::Searching && !recent {
                    let now = Instant::now();
                    let dt = last_anim.map_or(0.0, |t| (now - t).as_secs_f32());
                    animating = Some(now);
                    if (img.width, img.height) != STATUS_SIZE {
                        img = Image::new(STATUS_SIZE.0, STATUS_SIZE.1);
                    }
                    let (title, detail) = status.text();
                    bouncer.screen(&mut img, &title, &detail, dt);
                    shown = None;
                    Some(&img)
                } else if shown.as_ref() == Some(&(status.clone(), recent)) {
                    None
                } else {
                    let (title, detail) = status.text();
                    match last_frame.as_ref().filter(|_| recent) {
                        Some((f, _)) => {
                            img.clone_from(f);
                            status::badge(&mut img, &title);
                        }
                        None => {
                            if (img.width, img.height) != STATUS_SIZE {
                                img = Image::new(STATUS_SIZE.0, STATUS_SIZE.1);
                            }
                            status::screen(&mut img, &title, &detail);
                        }
                    }
                    shown = Some((status, recent));
                    Some(&img)
                }
            };

            if let (Some(d), Some(image)) = (display.as_mut(), to_show) {
                if let Err(e) = d.present(image) {
                    error!("display: {e}; reopening");
                    display = None;
                    shown = None;
                }
            }
        }
    }
}

/// Logs frame rate, decode and rotation time every 5 s.
struct Stats {
    since: Instant,
    frames: u32,
    decode: Duration,
    rotate: Duration,
}

impl Stats {
    fn new() -> Self {
        Stats {
            since: Instant::now(),
            frames: 0,
            decode: Duration::ZERO,
            rotate: Duration::ZERO,
        }
    }

    fn frame(&mut self, decode: Duration, rotate: Duration, degrees: f32) {
        self.frames += 1;
        self.decode += decode;
        self.rotate += rotate;
        let elapsed = self.since.elapsed();
        if elapsed >= Duration::from_secs(5) {
            let ms = |d: Duration| d.as_secs_f64() * 1000.0 / self.frames as f64;
            info!(
                "{:.1} fps, decode {:.1} ms, rotate {:.1} ms avg, angle {degrees:.0}°",
                self.frames as f64 / elapsed.as_secs_f64(),
                ms(self.decode),
                ms(self.rotate)
            );
            *self = Stats::new();
        }
    }
}
