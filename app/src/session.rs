//! One NE3 streaming session (spec section "session flow and timers").
//!
//! `Session` is a sans-IO state machine: it gets datagrams and timeouts with a
//! timestamp and returns the packets to send. `run` drives it with a real UDP socket.

use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use log::{debug, info, warn};

use crate::jpeg::HeaderCache;
use crate::proto::{self, Datagram, VideoChunk};

pub const RECV_TIMEOUT: Duration = Duration::from_millis(25);
const CONTINUE_INTERVAL: Duration = Duration::from_millis(25);
const RECONNECT_AFTER: Duration = Duration::from_millis(500);
const NULL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct Config {
    pub device: IpAddr,
    pub video_port: u16,
    pub null_port: u16,
    pub bind: SocketAddr,
    /// Silence after which the session gives up and returns to the supervisor.
    pub end_after: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            device: [192, 168, 169, 1].into(),
            video_port: 8800,
            null_port: 1234,
            bind: ([0, 0, 0, 0], 36000).into(),
            end_after: Duration::from_secs(3),
        }
    }
}

/// A complete, decodable JPEG file.
#[derive(Debug)]
pub struct Frame {
    pub img_number: u64,
    pub jpeg: Vec<u8>,
    /// Tilt sensor (x, y) from chunk 0, if the device sent it.
    pub tilt: Option<(i32, i32)>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Packet {
    /// To the video/control port (8800).
    Video(Vec<u8>),
    /// Empty datagram to the null port (1234).
    Null,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SessionEnd {
    Silence,
}

/// Collects the chunks of the current frame.
#[derive(Default)]
struct Reassembler {
    img_number: Option<u64>,
    chunks: Vec<Option<Vec<u8>>>,
    stored: usize,
    complete: bool,
    meta: (u16, u16, u8, u32),
    tilt: Option<(i32, i32)>,
}

impl Reassembler {
    fn reset(&mut self) {
        *self = Reassembler::default();
    }

    fn is_open(&self) -> bool {
        self.img_number.is_some() && !self.complete
    }

    /// Stores a chunk. Returns the joined scan data when the frame becomes complete.
    fn add(&mut self, c: &VideoChunk) -> Option<Vec<u8>> {
        match self.img_number {
            Some(cur) if c.img_number < cur => return None, // late packet
            Some(cur) if c.img_number == cur => {
                if self.complete {
                    return None; // duplicate of a finished frame
                }
            }
            _ => {
                self.img_number = Some(c.img_number);
                self.chunks = vec![None; c.packet_count as usize];
                self.stored = 0;
                self.complete = false;
                self.meta = (c.width, c.height, c.quality, c.img_size);
                self.tilt = None;
            }
        }
        if c.tilt.is_some() {
            self.tilt = c.tilt;
        }
        let idx = c.packet_number as usize;
        if idx >= self.chunks.len() {
            // packet_count changed within one frame; ignore the chunk.
            return None;
        }
        if self.chunks[idx].is_none() {
            self.stored += 1;
        }
        self.chunks[idx] = Some(c.data.to_vec());
        if self.stored < self.chunks.len() {
            return None;
        }
        self.complete = true;
        let mut scan: Vec<u8> = self
            .chunks
            .iter_mut()
            .flat_map(|c| c.take().unwrap())
            .collect();
        let img_size = self.meta.3 as usize;
        if img_size > 0 && scan.len() > img_size {
            scan.truncate(img_size);
        }
        Some(scan)
    }
}

pub struct Session {
    end_after: Duration,
    frame: Reassembler,
    headers: HeaderCache,
    last_complete: u64,
    last_rx: Instant,
    last_continue: Option<Instant>,
    last_null: Instant,
    frames: u64,
}

impl Session {
    /// Starts a session. Returns the session and the packets to send first.
    pub fn start(now: Instant, end_after: Duration) -> (Session, Vec<Packet>) {
        let s = Session {
            end_after,
            frame: Reassembler::default(),
            headers: HeaderCache::default(),
            last_complete: 0,
            last_rx: now,
            last_continue: None,
            last_null: now,
            frames: 0,
        };
        (
            s,
            vec![Packet::Null, Packet::Video(proto::connect().to_vec())],
        )
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }

    /// Handles one datagram from the device.
    pub fn on_datagram(
        &mut self,
        now: Instant,
        buf: &[u8],
        out: &mut Vec<Packet>,
    ) -> Option<Frame> {
        self.last_rx = now;
        self.null_timer(now, out);
        let chunk = match proto::parse(buf) {
            Datagram::Video(c) => c,
            Datagram::Control => {
                debug!("control packet: {:02x?}", buf);
                return None;
            }
            Datagram::Other => return None,
        };
        let (w, h, q) = (chunk.width, chunk.height, chunk.quality);
        let n = chunk.img_number;
        match self.frame.add(&chunk) {
            Some(scan) => {
                self.last_complete = n;
                self.frames += 1;
                out.push(Packet::Video(proto::ack_and_request(n)));
                Some(Frame {
                    img_number: n,
                    jpeg: self.headers.build(w, h, q, &scan),
                    tilt: self.frame.tilt,
                })
            }
            None => {
                if self.frame.is_open() {
                    self.send_continue(now, out);
                }
                None
            }
        }
    }

    /// Handles a receive timeout. Returns `Err` when the session should end.
    pub fn on_timeout(&mut self, now: Instant, out: &mut Vec<Packet>) -> Result<(), SessionEnd> {
        self.null_timer(now, out);
        let silence = now.saturating_duration_since(self.last_rx);
        if silence > self.end_after {
            return Err(SessionEnd::Silence);
        }
        if silence > RECONNECT_AFTER {
            self.frame.reset();
            self.last_complete = 0;
            out.push(Packet::Video(proto::connect().to_vec()));
        } else if silence >= RECV_TIMEOUT {
            if self.frame.is_open() {
                self.send_continue(now, out);
            } else {
                out.push(Packet::Video(proto::ack_and_request(self.last_complete)));
            }
        }
        Ok(())
    }

    fn send_continue(&mut self, now: Instant, out: &mut Vec<Packet>) {
        if self
            .last_continue
            .is_some_and(|t| now - t < CONTINUE_INTERVAL)
        {
            return;
        }
        self.last_continue = Some(now);
        out.push(Packet::Video(proto::continue_batch()));
    }

    fn null_timer(&mut self, now: Instant, out: &mut Vec<Packet>) {
        if now - self.last_null >= NULL_INTERVAL {
            self.last_null = now;
            out.push(Packet::Null);
        }
    }
}

/// Runs a session on a real socket until it ends. Calls `on_frame` for each complete frame.
pub fn run(cfg: &Config, mut on_frame: impl FnMut(Frame)) -> io::Result<SessionEnd> {
    let sock = UdpSocket::bind(cfg.bind)?;
    sock.set_read_timeout(Some(RECV_TIMEOUT))?;
    let video_addr = SocketAddr::new(cfg.device, cfg.video_port);
    let null_addr = SocketAddr::new(cfg.device, cfg.null_port);

    let send = |out: &mut Vec<Packet>| {
        for p in out.drain(..) {
            let r = match &p {
                Packet::Video(b) => sock.send_to(b, video_addr),
                Packet::Null => sock.send_to(&[], null_addr),
            };
            if let Err(e) = r {
                // Typical while the link is down (ENETUNREACH). The silence timer ends the session.
                debug!("send failed: {e}");
            }
        }
    };

    info!("session start, device {}", cfg.device);
    let (mut session, mut out) = Session::start(Instant::now(), cfg.end_after);
    send(&mut out);

    let mut buf = [0u8; 2048];
    loop {
        match sock.recv_from(&mut buf) {
            Ok((len, from)) => {
                if from.ip() != cfg.device {
                    continue;
                }
                if let Some(frame) = session.on_datagram(Instant::now(), &buf[..len], &mut out) {
                    on_frame(frame);
                }
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if let Err(end) = session.on_timeout(Instant::now(), &mut out) {
                    info!("session end ({end:?}) after {} frames", session.frames());
                    return Ok(end);
                }
            }
            Err(e) => {
                // e.g. ECONNREFUSED from an ICMP error; keep going, the timers decide.
                warn!("recv failed: {e}");
                std::thread::sleep(RECV_TIMEOUT);
                if let Err(end) = session.on_timeout(Instant::now(), &mut out) {
                    return Ok(end);
                }
            }
        }
        send(&mut out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::tests::video_packet;

    const MS: Duration = Duration::from_millis(1);

    fn chunk(img: u64, num: u32, count: u32) -> Vec<u8> {
        video_packet(img, num, count, &[num as u8; 1024])
    }

    fn videos(out: &[Packet]) -> Vec<&[u8]> {
        out.iter()
            .filter_map(|p| match p {
                Packet::Video(b) => Some(&b[..]),
                Packet::Null => None,
            })
            .collect()
    }

    #[test]
    fn start_sends_null_and_connect() {
        let (_, out) = Session::start(Instant::now(), Duration::from_secs(3));
        assert_eq!(out, vec![Packet::Null, Packet::Video(vec![0xef, 0, 4, 0])]);
    }

    #[test]
    fn frame_out_of_order_with_duplicate() {
        let t0 = Instant::now();
        let (mut s, _) = Session::start(t0, Duration::from_secs(3));
        let mut out = vec![];
        assert!(s.on_datagram(t0, &chunk(5, 2, 3), &mut out).is_none());
        assert_eq!(videos(&out), vec![&proto::continue_batch()[..]]);
        out.clear();
        assert!(s.on_datagram(t0 + MS, &chunk(5, 0, 3), &mut out).is_none());
        assert!(out.is_empty(), "continue is rate-limited");
        assert!(
            s.on_datagram(t0 + MS, &chunk(5, 0, 3), &mut out).is_none(),
            "duplicate"
        );
        let f = s
            .on_datagram(t0 + 2 * MS, &chunk(5, 1, 3), &mut out)
            .expect("frame");
        assert_eq!(f.img_number, 5);
        assert_eq!(videos(&out), vec![&proto::ack_and_request(5)[..]]);
        // header + 3 chunks in index order (with img_size = 3072) + EOI
        let scan = &f.jpeg[crate::jpeg::HEADER_LEN..f.jpeg.len() - 2];
        assert_eq!(scan.len(), 3072);
        assert_eq!((scan[0], scan[1024], scan[2048]), (0, 1, 2));
        assert_eq!(&f.jpeg[f.jpeg.len() - 2..], &[0xff, 0xd9]);
        // late duplicate of the finished frame changes nothing
        out.clear();
        assert!(s
            .on_datagram(t0 + 3 * MS, &chunk(5, 1, 3), &mut out)
            .is_none());
        assert!(out.is_empty());
    }

    #[test]
    fn frame_carries_tilt_of_chunk_0() {
        let t0 = Instant::now();
        let (mut s, _) = Session::start(t0, Duration::from_secs(3));
        let mut out = vec![];
        let mut c0 = vec![0u8; 1024];
        c0.extend_from_slice(b"  -40,01000");
        s.on_datagram(t0, &video_packet(3, 1, 2, &[1; 1024]), &mut out);
        let f = s
            .on_datagram(t0, &video_packet(3, 0, 2, &c0), &mut out)
            .unwrap();
        assert_eq!(f.tilt, Some((-40, 1000)));
        // next frame without a tilt string: no stale value
        let f = s.on_datagram(t0, &chunk(4, 0, 1), &mut out).unwrap();
        assert_eq!(f.tilt, None);
    }

    #[test]
    fn newer_frame_drops_incomplete_and_late_packets_are_ignored() {
        let t0 = Instant::now();
        let (mut s, _) = Session::start(t0, Duration::from_secs(3));
        let mut out = vec![];
        s.on_datagram(t0, &chunk(1, 0, 2), &mut out);
        s.on_datagram(t0, &chunk(2, 0, 2), &mut out);
        assert!(
            s.on_datagram(t0, &chunk(1, 1, 2), &mut out).is_none(),
            "late"
        );
        assert_eq!(
            s.on_datagram(t0, &chunk(2, 1, 2), &mut out)
                .unwrap()
                .img_number,
            2
        );
    }

    #[test]
    fn timeouts() {
        let t0 = Instant::now();
        let (mut s, _) = Session::start(t0, Duration::from_secs(3));
        let mut out = vec![];
        s.on_datagram(t0, &chunk(1, 0, 1), &mut out).unwrap();
        out.clear();

        // Short silence, frame done: resend ACK + request.
        s.on_timeout(t0 + 30 * MS, &mut out).unwrap();
        assert_eq!(videos(&out), vec![&proto::ack_and_request(1)[..]]);
        out.clear();

        // Open frame: continue.
        let t1 = t0 + 40 * MS;
        s.on_datagram(t1, &chunk(2, 0, 2), &mut out);
        out.clear();
        s.on_timeout(t1 + 30 * MS, &mut out).unwrap();
        assert_eq!(videos(&out), vec![&proto::continue_batch()[..]]);
        out.clear();

        // Long silence: connect, frame dropped, next ACK refers to frame 0.
        s.on_timeout(t1 + 600 * MS, &mut out).unwrap();
        assert_eq!(videos(&out), vec![&proto::connect()[..]]);
        assert!(!s.frame.is_open());
        assert_eq!(s.last_complete, 0);

        // After a reconnect the device may start with a lower frame number.
        out.clear();
        let t2 = t1 + 700 * MS;
        assert!(s.on_datagram(t2, &chunk(0, 0, 1), &mut out).is_some());

        // Null packet every 5 s; session ends after 3 s of silence.
        out.clear();
        s.on_timeout(t0 + Duration::from_millis(5001), &mut out)
            .unwrap_err();
        assert!(out.contains(&Packet::Null));
    }
}
