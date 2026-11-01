//! PNG helpers. A thumbnail is made once, when an image is copied, and stored beside it, so opening the popup only
//! decodes tiny files and never a full screenshot.
use std::io::Cursor;

use png::{BitDepth, ColorType, Decoder, Encoder, Transformations};
use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Largest image (in pixels) decoded for a thumbnail: 20 Mpix is about 80 MB of RGBA while it is being decoded.
/// Bigger images are still stored and pasted, they just show no thumbnail.
const MAX_THUMB_SOURCE_PIXELS: u64 = 20_000_000;
/// Largest JPEG (in pixels) converted to PNG.
const MAX_JPEG_PIXELS: u64 = 8_000_000;
/// The thumbnail is fitted into this box. The list shows it at half size, so it stays sharp on HiDPI screens.
const THUMB_BOX: (u32, u32) = (192, 96);

/// What the clipboard gave us as PNG: PNG bytes stay as they are, a JPEG is decoded and re-encoded (so the history, the
/// thumbnails, the paste and the OCR only ever deal with PNG). `None` for anything else or a JPEG too big to decode.
/// To support another format, add a magic-number arm here and its type to `x11.rs`'s image targets.
pub fn to_png(data: Vec<u8>) -> Option<Vec<u8>> {
    use zune_jpeg::{
        JpegDecoder,
        zune_core::{bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions},
    };
    if data.starts_with(b"\x89PNG") {
        return Some(data);
    }
    if !data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return None;
    }
    let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(&data), options);
    decoder.decode_headers().ok()?;
    let info = decoder.info()?;
    // As PNG a photo takes 2-3 bytes per pixel, so above this it would exceed the 8 MB cap after all the work of
    // decoding and encoding it.
    if u64::from(info.width) * u64::from(info.height) > MAX_JPEG_PIXELS {
        return None;
    }
    let rgba = decoder.decode().ok()?;
    encode_rgba(u32::from(info.width), u32::from(info.height), &rgba)
}

/// (width, height) from the PNG header; `None` when the bytes are not a PNG.
pub fn size(png: &[u8]) -> Option<(u32, u32)> {
    let reader = Decoder::new(Cursor::new(png)).read_info().ok()?;
    let info = reader.info();
    Some((info.width, info.height))
}

/// The whole image as RGBA, 8 bits per channel.
pub fn decode_rgba(png: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut decoder = Decoder::new(Cursor::new(png));
    // Palettes and 16-bit samples become plain 8-bit gray/RGB(A).
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buf).ok()?;
    let len = frame.line_size * frame.height as usize;
    let data = &buf[..len];
    let rgba = match frame.color_type {
        ColorType::Rgba => {
            // The usual case (screenshots): reuse the buffer instead of a second copy of up to 80 MB.
            buf.truncate(len);
            buf
        }
        ColorType::Rgb => data
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        ColorType::GrayscaleAlpha => data
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        ColorType::Indexed => return None, // EXPAND already turned it into RGB(A)
    };
    Some((frame.width, frame.height, rgba))
}

/// `(w, h)` scaled down to fit `THUMB_BOX`, keeping the aspect ratio and never enlarging.
fn fit(w: u32, h: u32) -> (u32, u32) {
    let scale = (f64::from(THUMB_BOX.0) / f64::from(w))
        .min(f64::from(THUMB_BOX.1) / f64::from(h))
        .min(1.0);
    (
        ((f64::from(w) * scale).round() as u32).max(1),
        ((f64::from(h) * scale).round() as u32).max(1),
    )
}

/// Area average: every output pixel is the mean of the source pixels it covers (no skipped pixels like nearest-neighbor).
fn downscale(src: &[u8], (sw, sh): (u32, u32), (dw, dh): (u32, u32)) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    let mut out = Vec::with_capacity(dw * dh * 4);
    for dy in 0..dh {
        let (y0, y1) = (dy * sh / dh, ((dy + 1) * sh / dh).max(dy * sh / dh + 1));
        for dx in 0..dw {
            let (x0, x1) = (dx * sw / dw, ((dx + 1) * sw / dw).max(dx * sw / dw + 1));
            let mut sum = [0u32; 4];
            for y in y0..y1 {
                for pixel in src[(y * sw + x0) * 4..(y * sw + x1) * 4].as_chunks::<4>().0 {
                    for (s, &c) in sum.iter_mut().zip(pixel) {
                        *s += u32::from(c);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            out.extend(sum.map(|s| (s / n) as u8));
        }
    }
    out
}

fn encode_rgba(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, w, h);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(rgba).ok()?;
    writer.finish().ok()?;
    Some(out)
}

/// A small PNG of `png` for the list; `None` when it is too big to decode cheaply or cannot be decoded.
pub fn thumbnail(png: &[u8]) -> Option<Vec<u8>> {
    let (w, h) = size(png)?;
    if u64::from(w) * u64::from(h) > MAX_THUMB_SOURCE_PIXELS {
        return None;
    }
    let (w, h, rgba) = decode_rgba(png)?;
    let (tw, th) = fit(w, h);
    encode_rgba(tw, th, &downscale(&rgba, (w, h), (tw, th)))
}

/// A stored thumbnail as an image the UI can show.
pub fn load_thumbnail(id: &str) -> Option<slint::Image> {
    let png = std::fs::read(fastpaste_core::history::thumb_path(id)).ok()?;
    let (w, h, rgba) = decode_rgba(&png)?;
    Some(slint::Image::from_rgba8(
        SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&rgba, w, h),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A w x h RGBA PNG: red grows to the right, green downwards.
    fn gradient(w: u32, h: u32) -> Vec<u8> {
        let rgba: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| [(x * 255 / w) as u8, (y * 255 / h) as u8, 0, 255]))
            .collect();
        encode_rgba(w, h, &rgba).unwrap()
    }

    #[test]
    fn thumbnail_fits_the_box_and_keeps_the_aspect() {
        let big = gradient(400, 100);
        assert_eq!(size(&big), Some((400, 100)));
        let thumb = thumbnail(&big).unwrap();
        assert_eq!(size(&thumb), Some((192, 48)));
        let (_, _, px) = decode_rgba(&thumb).unwrap();
        assert!(
            px[0] < px[(191 * 4) as usize],
            "red grows to the right: averaged, not garbled"
        );
        // A small image is not enlarged.
        assert_eq!(size(&thumbnail(&gradient(10, 10)).unwrap()), Some((10, 10)));
    }

    #[test]
    fn jpeg_becomes_png_and_png_is_kept() {
        let png = super::to_png(include_bytes!("../testdata/tiny.jpg").to_vec()).unwrap();
        assert_eq!(size(&png), Some((16, 8)));
        assert_eq!(super::to_png(png.clone()), Some(png));
        assert_eq!(super::to_png(b"GIF89a....".to_vec()), None);
        assert_eq!(super::to_png(vec![0xFF, 0xD8, 0xFF, 0, 1, 2]), None, "truncated JPEG");
    }

    #[test]
    fn garbage_and_other_color_types() {
        assert_eq!(size(b"not a png"), None);
        assert!(thumbnail(b"not a png").is_none());
        let mut gray = Vec::new();
        let mut enc = Encoder::new(&mut gray, 2, 1);
        enc.set_color(ColorType::Grayscale);
        enc.set_depth(BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[10, 200]).unwrap();
        w.finish().unwrap();
        assert_eq!(decode_rgba(&gray).unwrap().2, [10, 10, 10, 255, 200, 200, 200, 255]);
    }
}
