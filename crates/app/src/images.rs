//! PNG helpers. A thumbnail is made once, when an image is copied, and stored beside it, so opening the popup only
//! decodes tiny files and never a full screenshot.
use std::io::Cursor;

use png::{BitDepth, ColorType, Decoder, Encoder, Transformations};
use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Largest image (in pixels) read for a thumbnail. The decode is streamed, so this limits time, not memory: a PNG of
/// at most 8 MB can describe an enormous flat image. 40 Mpix covers several 4K screens. Bigger images are still
/// stored and pasted, they just show no thumbnail.
const MAX_THUMB_SOURCE_PIXELS: u64 = 40_000_000;
/// The thumbnail is fitted into this box. The list shows it at half size, so it stays sharp on HiDPI screens.
const THUMB_BOX: (u32, u32) = (192, 96);

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

/// For each of `src` source positions, the output cell (of `dst`, `dst <= src`) it falls in. The cells split the
/// source evenly, so every source position belongs to exactly one.
fn cell_map(src: u32, dst: u32) -> Vec<usize> {
    let (s, d) = (u64::from(src), u64::from(dst));
    // The last cell whose first source position (cell * src / dst) is at or before `i`.
    (0..s).map(|i| (((i + 1) * d).div_ceil(s) - 1) as usize).collect()
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

/// What `thumbnail` made of a PNG.
#[derive(Debug, PartialEq)]
pub enum Thumb {
    Made(Vec<u8>),
    /// Valid but not decoded: too big to do in reasonable time, or interlaced. The row shows only its label.
    Skipped,
    /// The data does not decode (truncated, corrupt): the image is not worth storing.
    Broken,
}

/// A small PNG of `png` for the list. Decoded row by row and averaged straight into the thumbnail (every output pixel
/// is the mean of the source pixels it covers), so memory stays at one row however big the screenshot is: decoding the
/// whole image took 80-130 MB for a 4K/5K one, and glibc then kept part of it.
pub fn thumbnail(png: &[u8]) -> Thumb {
    let mut decoder = Decoder::new(Cursor::new(png));
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let Ok(mut reader) = decoder.read_info() else {
        return Thumb::Broken;
    };
    let (w, h, interlaced) = {
        let info = reader.info();
        (info.width, info.height, info.interlaced)
    };
    if w == 0 || h == 0 {
        return Thumb::Broken;
    }
    // `next_row` of an interlaced image yields the passes of Adam7, not whole rows; those get the text label.
    if interlaced || u64::from(w) * u64::from(h) > MAX_THUMB_SOURCE_PIXELS {
        return Thumb::Skipped;
    }
    let channels = reader.output_color_type().0.samples();
    let (tw, th) = fit(w, h);
    let (cols, rows) = (cell_map(w, tw), cell_map(h, th));
    let tw = tw as usize;
    let mut sums = vec![0u64; tw * th as usize * 4];
    let mut counts = vec![0u64; tw * th as usize];
    for &cell_row in &rows {
        let Ok(Some(row)) = reader.next_row() else {
            return Thumb::Broken;
        };
        for (px, &cell) in row.data().chunks_exact(channels).zip(&cols) {
            let rgba = match *px {
                [g] => [g, g, g, 255],
                [g, a] => [g, g, g, a],
                [r, g, b] => [r, g, b, 255],
                [r, g, b, a] => [r, g, b, a],
                _ => return Thumb::Broken,
            };
            let i = cell_row * tw + cell;
            counts[i] += 1;
            for (s, c) in sums[i * 4..i * 4 + 4].iter_mut().zip(rgba) {
                *s += u64::from(c);
            }
        }
    }
    let out: Vec<u8> = sums
        .as_chunks::<4>()
        .0
        .iter()
        .zip(&counts)
        .flat_map(|(s, &n)| s.iter().map(move |&v| (v / n.max(1)) as u8))
        .collect();
    encode_rgba(tw as u32, th, &out).map_or(Thumb::Broken, Thumb::Made)
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

    fn made(png: &[u8]) -> Vec<u8> {
        match thumbnail(png) {
            Thumb::Made(t) => t,
            other => panic!("no thumbnail: {other:?}"),
        }
    }

    #[test]
    fn thumbnail_fits_the_box_and_keeps_the_aspect() {
        let big = gradient(400, 100);
        assert_eq!(size(&big), Some((400, 100)));
        let thumb = made(&big);
        assert_eq!(size(&thumb), Some((192, 48)));
        let (_, _, px) = decode_rgba(&thumb).unwrap();
        assert!(
            px[0] < px[(191 * 4) as usize],
            "red grows to the right: averaged, not garbled"
        );
        // A small image is not enlarged.
        assert_eq!(size(&made(&gradient(10, 10))), Some((10, 10)));
    }

    /// The thumbnail the old way: the whole image as RGBA, then the mean of the source pixels each output pixel covers.
    fn reference_thumbnail(png: &[u8]) -> (u32, u32, Vec<u8>) {
        let (sw, sh, src) = decode_rgba(png).unwrap();
        let (dw, dh) = fit(sw, sh);
        let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
        let mut out = Vec::new();
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
        (dw as u32, dh as u32, out)
    }

    #[test]
    fn streamed_thumbnail_equals_the_whole_image_average_for_every_color_type() {
        // 500x130 does not divide evenly into the 192x50 thumbnail: the cell borders are the risky part.
        let (w, h) = (500u32, 130u32);
        for (color, palette) in [
            (ColorType::Grayscale, false),
            (ColorType::GrayscaleAlpha, false),
            (ColorType::Rgb, false),
            (ColorType::Rgba, false),
            (ColorType::Indexed, true),
        ] {
            let channels = if palette { 1 } else { color.samples() };
            let data: Vec<u8> = (0..w * h * channels as u32)
                .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
                .collect();
            let mut png = Vec::new();
            let mut enc = Encoder::new(&mut png, w, h);
            enc.set_color(color);
            enc.set_depth(BitDepth::Eight);
            if palette {
                enc.set_palette((0..=255u8).flat_map(|i| [i, 255 - i, i / 2]).collect::<Vec<_>>());
            }
            let mut writer = enc.write_header().unwrap();
            writer.write_image_data(&data).unwrap();
            writer.finish().unwrap();

            let thumb = made(&png);
            let (tw, th, pixels) = decode_rgba(&thumb).unwrap();
            let (rw, rh, expected) = reference_thumbnail(&png);
            assert_eq!((tw, th), (rw, rh), "{color:?}: size");
            assert_eq!(pixels, expected, "{color:?}: pixels");
        }
    }

    #[test]
    fn thumbnail_gives_up_on_too_many_pixels_and_cell_borders_cover_everything() {
        let huge = {
            let mut png = Vec::new();
            let mut enc = Encoder::new(&mut png, 7000, 7000); // 49 Mpix of a flat color: a few KB
            enc.set_color(ColorType::Grayscale);
            enc.set_depth(BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&vec![0u8; 7000 * 7000]).unwrap();
            w.finish().unwrap();
            png
        };
        assert_eq!(thumbnail(&huge), Thumb::Skipped);
        // Every source position maps to a cell, in order, and every cell gets at least one.
        let map = cell_map(500, 192);
        assert!(map.windows(2).all(|p| p[1] == p[0] || p[1] == p[0] + 1));
        assert_eq!((map[0], *map.last().unwrap()), (0, 191));
    }

    #[test]
    fn garbage_and_other_color_types() {
        assert_eq!(size(b"not a png"), None);
        assert_eq!(thumbnail(b"not a png"), Thumb::Broken);
        // Valid header, data cut short (an app that died mid-copy): broken, not "too big".
        let whole = gradient(50, 50);
        assert_eq!(thumbnail(&whole[..whole.len() / 2]), Thumb::Broken);
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
