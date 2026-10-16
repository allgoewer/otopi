//! NE3 wire format: client-to-device packets and parsing of device video chunks.
//!
//! See `doc/NE3-NE7-Earpick-Protocol-Specification.md`, sections "common framing",
//! "client-to-device packets" and "video packets".

pub const MAGIC_CLIENT: u8 = 0xef;
pub const MAGIC_DEVICE: u8 = 0x93;

pub const TYPE_CONNECT: u8 = 0x00;
pub const TYPE_BATCH: u8 = 0x02;
pub const TYPE_VIDEO: u8 = 0x01;
pub const TYPE_CONTROL: u8 = 0x04;

/// Bytes of JPEG scan data per chunk.
pub const CHUNK_DATA_LEN: usize = 1024;
/// Size of the video chunk header.
pub const VIDEO_HEADER_LEN: usize = 56;

const BATCH_PREFIX: [u8; 4] = [0x02, 0x02, 0x00, 0x01];
const BATCH_MAGIC2: [u8; 8] = [0x0a, 0x4b, 0x14, 0x2d, 0x00, 0x00, 0x00, 0x00];

const MSG_ACK: u32 = 1;
const MSG_REQUEST: u32 = 3;

/// Connect packet (`ef 00`): starts the stream.
pub fn connect() -> [u8; 4] {
    [MAGIC_CLIENT, TYPE_CONNECT, 0x04, 0x00]
}

/// Message batch with 0 messages: tells the device to continue the current frame.
pub fn continue_batch() -> Vec<u8> {
    batch(&[])
}

/// ACK for frame `n` + request for frame `n + 1`.
pub fn ack_and_request(n: u64) -> Vec<u8> {
    let mut ack = Vec::with_capacity(20);
    push_msg_header(&mut ack, n, MSG_ACK, 20);
    ack.extend_from_slice(&[0xff; 4]);

    let mut req = Vec::with_capacity(16);
    push_msg_header(&mut req, n.wrapping_add(1), MSG_REQUEST, 16);

    batch(&[&ack, &req])
}

fn push_msg_header(buf: &mut Vec<u8>, img_number: u64, msg_type: u32, msg_len: u32) {
    buf.extend_from_slice(&img_number.to_le_bytes());
    buf.extend_from_slice(&msg_type.to_le_bytes());
    buf.extend_from_slice(&msg_len.to_le_bytes());
}

fn batch(messages: &[&[u8]]) -> Vec<u8> {
    let total = 40 + messages.iter().map(|m| m.len()).sum::<usize>();
    let mut buf = Vec::with_capacity(total);
    buf.push(MAGIC_CLIENT);
    buf.push(TYPE_BATCH);
    buf.extend_from_slice(&(total as u16).to_le_bytes());
    buf.extend_from_slice(&BATCH_PREFIX);
    buf.extend_from_slice(&(messages.len() as u64).to_le_bytes());
    buf.extend_from_slice(&[0; 8]);
    buf.extend_from_slice(&BATCH_MAGIC2);
    for m in messages {
        buf.extend_from_slice(m);
    }
    buf.extend_from_slice(&[0; 8]);
    debug_assert_eq!(buf.len(), total);
    buf
}

/// One decoded `93 01` video datagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoChunk<'a> {
    pub img_number: u64,
    pub packet_number: u32,
    pub packet_count: u32,
    pub img_size: u32,
    pub width: u16,
    pub height: u16,
    pub quality: u8,
    /// Scan data: at most the first 1024 bytes of the payload.
    pub data: &'a [u8],
    /// Tilt sensor (x, y), only in chunk 0 (spec section "tilt sensor").
    pub tilt: Option<(i32, i32)>,
}

/// What a received datagram on the video port is.
#[derive(Debug, PartialEq, Eq)]
pub enum Datagram<'a> {
    Video(VideoChunk<'a>),
    Control,
    Other,
}

pub fn parse(buf: &[u8]) -> Datagram<'_> {
    if buf.len() < 2 || buf[0] != MAGIC_DEVICE {
        return Datagram::Other;
    }
    match buf[1] {
        TYPE_VIDEO => parse_video(buf).map_or(Datagram::Other, Datagram::Video),
        TYPE_CONTROL => Datagram::Control,
        _ => Datagram::Other,
    }
}

fn parse_video(buf: &[u8]) -> Option<VideoChunk<'_>> {
    if buf.len() < VIDEO_HEADER_LEN {
        return None;
    }
    let u16_at = |o: usize| u16::from_le_bytes([buf[o], buf[o + 1]]);
    let u32_at = |o: usize| u32::from_le_bytes(buf[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap());

    let packet_number = u32_at(32);
    let packet_count = u32_at(36);
    if packet_count == 0 || packet_number >= packet_count {
        return None;
    }
    let payload = &buf[VIDEO_HEADER_LEN..];
    Some(VideoChunk {
        img_number: u64_at(8),
        packet_number,
        packet_count,
        img_size: u32_at(40),
        width: u16_at(44),
        height: u16_at(46),
        quality: buf[48],
        data: &payload[..payload.len().min(CHUNK_DATA_LEN)],
        tilt: if packet_number == 0 {
            parse_tilt(payload)
        } else {
            None
        },
    })
}

/// Chunk 0 carries x (5 ASCII bytes), one separator byte, then y (rest of the
/// datagram) after the 1024 bytes of scan data. The exact text format is not
/// known, so allow spaces, a sign and NUL padding.
fn parse_tilt(payload: &[u8]) -> Option<(i32, i32)> {
    let num = |b: &[u8]| -> Option<i32> {
        let s = std::str::from_utf8(b).ok()?;
        s.trim_matches(|c: char| c.is_whitespace() || c == '\0')
            .parse()
            .ok()
    };
    let x = num(payload.get(CHUNK_DATA_LEN..CHUNK_DATA_LEN + 5)?)?;
    let y = num(payload.get(CHUNK_DATA_LEN + 6..)?)?;
    Some((x, y))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        s.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    #[test]
    fn continue_matches_spec() {
        let expected = hex("ef 02 28 00 02 02 00 01 00 00 00 00 00 00 00 00
             00 00 00 00 00 00 00 00 0a 4b 14 2d 00 00 00 00
             00 00 00 00 00 00 00 00");
        assert_eq!(continue_batch(), expected);
    }

    #[test]
    fn ack_request_matches_spec() {
        let expected = hex("ef 02 4c 00 02 02 00 01 02 00 00 00 00 00 00 00
             00 00 00 00 00 00 00 00 0a 4b 14 2d 00 00 00 00
             07 00 00 00 00 00 00 00 01 00 00 00 14 00 00 00
             ff ff ff ff 08 00 00 00 00 00 00 00 03 00 00 00
             10 00 00 00 00 00 00 00 00 00 00 00");
        assert_eq!(ack_and_request(7), expected);
    }

    pub(crate) fn video_packet(img: u64, num: u32, count: u32, data: &[u8]) -> Vec<u8> {
        let mut p = vec![0u8; VIDEO_HEADER_LEN];
        p[0] = MAGIC_DEVICE;
        p[1] = TYPE_VIDEO;
        let len = (VIDEO_HEADER_LEN + data.len()) as u16;
        p[2..4].copy_from_slice(&len.to_le_bytes());
        p[8..16].copy_from_slice(&img.to_le_bytes());
        p[32..36].copy_from_slice(&num.to_le_bytes());
        p[36..40].copy_from_slice(&count.to_le_bytes());
        p[40..44].copy_from_slice(&(count * 1024).to_le_bytes());
        p[44..46].copy_from_slice(&640u16.to_le_bytes());
        p[46..48].copy_from_slice(&360u16.to_le_bytes());
        p[48] = 50;
        p.extend_from_slice(data);
        p
    }

    #[test]
    fn parse_video_chunk() {
        let mut data = vec![0xaa; 1024];
        data.extend_from_slice(b"  123,-1000");
        let p = video_packet(42, 0, 3, &data);
        match parse(&p) {
            Datagram::Video(c) => {
                assert_eq!(c.img_number, 42);
                assert_eq!(c.packet_number, 0);
                assert_eq!(c.packet_count, 3);
                assert_eq!((c.width, c.height, c.quality), (640, 360, 50));
                assert_eq!(c.data.len(), 1024, "tilt string must be cut off");
                assert_eq!(c.tilt, Some((123, -1000)));
            }
            d => panic!("unexpected {d:?}"),
        }
    }

    fn tilt_of(num: u32, tail: &[u8]) -> Option<(i32, i32)> {
        let mut data = vec![0xaa; 1024];
        data.extend_from_slice(tail);
        match parse(&video_packet(1, num, 2, &data)) {
            Datagram::Video(c) => c.tilt,
            d => panic!("unexpected {d:?}"),
        }
    }

    #[test]
    fn tilt_formats() {
        assert_eq!(tilt_of(0, b"-0512,0890"), Some((-512, 890)));
        assert_eq!(tilt_of(0, b"    0 1024\0\0"), Some((0, 1024)));
        assert_eq!(tilt_of(0, b"12345;-7"), Some((12345, -7)));
        assert_eq!(tilt_of(0, b"abcde,12"), None, "garbage");
        assert_eq!(tilt_of(0, b"12"), None, "too short");
        assert_eq!(tilt_of(0, b""), None, "no tilt string");
        assert_eq!(tilt_of(1, b"  123,-1000"), None, "only chunk 0 has tilt");
    }

    #[test]
    fn parse_rejects_bad_packets() {
        assert_eq!(parse(&[]), Datagram::Other);
        assert_eq!(parse(&[0xef, 0x01, 0, 0]), Datagram::Other);
        assert_eq!(parse(&[0x93, 0x04, 0, 0]), Datagram::Control);
        assert_eq!(parse(&[0x93, 0x01, 0, 0]), Datagram::Other, "too short");
        let bad = video_packet(1, 3, 3, &[0; 10]);
        assert_eq!(
            parse(&bad),
            Datagram::Other,
            "packet_number >= packet_count"
        );
    }
}
