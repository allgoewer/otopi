//! JPEG decoding to BGRA (memory layout of DRM XRGB8888 on little-endian).

use zune_jpeg::errors::DecodeErrors;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

/// Decodes `jpeg` into `out` (resized as needed). Returns (width, height).
pub fn decode_bgra(jpeg: &[u8], out: &mut Vec<u8>) -> Result<(usize, usize), DecodeErrors> {
    let opts = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::BGRA)
        // The device sometimes sends truncated scan data; show what we have.
        .set_strict_mode(false);
    let mut dec = JpegDecoder::new_with_options(ZCursor::new(jpeg), opts);
    dec.decode_headers()?;
    let info = dec
        .info()
        .ok_or(DecodeErrors::FormatStatic("no image info"))?;
    let size = dec
        .output_buffer_size()
        .ok_or(DecodeErrors::FormatStatic("no output size"))?;
    out.resize(size, 0);
    dec.decode_into(out)?;
    Ok((info.width as usize, info.height as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jpeg::HeaderCache;
    use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

    /// Returns the entropy-coded data between SOS and EOI.
    fn scan_data(jpeg: &[u8]) -> &[u8] {
        let mut i = 2;
        loop {
            assert_eq!(jpeg[i], 0xff);
            let marker = jpeg[i + 1];
            let len = u16::from_be_bytes([jpeg[i + 2], jpeg[i + 3]]) as usize;
            if marker == 0xda {
                let start = i + 2 + len;
                return &jpeg[start..jpeg.len() - 2];
            }
            i += 2 + len;
        }
    }

    #[test]
    fn round_trip_with_reconstructed_header() {
        let (w, h) = (640u16, 360u16);
        let mut rgb = Vec::with_capacity(w as usize * h as usize * 3);
        for y in 0..h as usize {
            for x in 0..w as usize {
                rgb.extend_from_slice(&[(x / 3) as u8, (y * 255 / 360) as u8, 128]);
            }
        }
        for q in [25u8, 50, 75] {
            let mut encoded = Vec::new();
            let mut enc = Encoder::new(&mut encoded, q);
            enc.set_sampling_factor(SamplingFactor::R_4_4_4);
            enc.encode(&rgb, w, h, ColorType::Rgb).unwrap();

            let jpeg = HeaderCache::default().build(w, h, q, scan_data(&encoded));
            let mut bgra = Vec::new();
            assert_eq!(decode_bgra(&jpeg, &mut bgra).unwrap(), (640, 360));
            assert_eq!(bgra.len(), 640 * 360 * 4);

            let mut reference = Vec::new();
            decode_bgra(&encoded, &mut reference).unwrap();
            assert_eq!(
                bgra, reference,
                "quality {q}: same pixels as the original file"
            );
        }
    }
}
