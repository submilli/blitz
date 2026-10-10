//! Decoding image bodies within the decode bounds and the decoded image
//! budget: rasters first, then SVG documents.
use crate::DecodedImageBudget;
use crate::node::{ImageData, RasterImageData};
use blitz_traits::net::Bytes;
use std::io::Cursor;
use std::sync::Arc;

/// Decode a raster image, or else parse an SVG document, charged to `budget`.
/// The error describes why neither worked; it names no page content.
// `svg_fonts` is only read to parse SVG documents.
#[cfg_attr(not(feature = "svg"), allow(unused_variables))]
pub(crate) fn decode_image(
    bytes: &Bytes,
    origin_clean: bool,
    svg_fonts: &crate::util::SvgFonts,
    budget: &DecodedImageBudget,
) -> Result<ImageData, String> {
    const BUDGET_EXHAUSTED: &str = "Decoded image budget exhausted";
    let image_err = match decode_charged(bytes, budget) {
        Ok(mut raster) => {
            raster.origin_clean = origin_clean;
            return Ok(ImageData::Raster(raster));
        }
        Err(DecodeRefusal::Budget) => return Err(BUDGET_EXHAUSTED.into()),
        Err(DecodeRefusal::Image(e)) => e.to_string(),
    };

    #[cfg(feature = "svg")]
    let svg_err = match crate::util::parse_svg_image(bytes, svg_fonts) {
        Ok(mut svg) => {
            if !budget.admit_svg(&svg) {
                return Err(BUDGET_EXHAUSTED.into());
            }
            svg.origin_clean = origin_clean;
            return Ok(ImageData::Svg(svg));
        }
        Err(e) => e.to_string(),
    };
    #[cfg(not(feature = "svg"))]
    let svg_err = "svg feature disabled";

    Err(format!(
        "Could not parse image ({} bytes): image-crate error: {image_err}; svg fallback error: {svg_err}",
        bytes.len()
    ))
}

/// A few bytes of compressed image can declare a huge bitmap. Decoding
/// stops past these bounds, and the image is broken. The decoder's output
/// (up to 16 bits per channel) and the RGBA8 copy every image ends as are
/// bounded separately, before either is allocated: a grayscale image's
/// decoded buffer is a quarter of its RGBA one. Together they peak at
/// 192 MiB, well inside a 512 MiB guest.
const MAX_DECODED_SIDE: u32 = 16_384;
const MAX_DECODE_ALLOCATION: u64 = 128 * 1024 * 1024;
const MAX_RGBA_BYTES: u64 = 64 * 1024 * 1024;

/// The RGBA bytes an encoded raster image decodes to, read from its header,
/// when it is within the decode bounds; `None` when it is not or is unreadable.
pub(crate) fn bounded_rgba_bytes(bytes: &[u8]) -> Option<u64> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.limits(decode_limits());
    let (width, height) = reader.into_dimensions().ok()?;
    let rgba = u64::from(width) * u64::from(height) * 4;
    (width <= MAX_DECODED_SIDE && height <= MAX_DECODED_SIDE && rgba <= MAX_RGBA_BYTES)
        .then_some(rgba)
}

/// Decode `bytes`, refusing images whose decoded or RGBA8 form exceeds
/// its bound before any pixel buffer is allocated. `image::Limits` caps
/// the side for every format and the PNG and GIF decoders' working memory;
/// JPEG and WebP decoders ignore `max_alloc`, but their working memory is
/// a few bytes per pixel, which the RGBA bound keeps near the same peak.
/// The output buffer is checked here, as `ImageReader::decode` would.
pub(crate) fn decode_bounded(bytes: &Bytes) -> image::ImageResult<image::DynamicImage> {
    use image::ImageDecoder;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .expect("IO errors impossible with Cursor");
    reader.limits(decode_limits());
    let decoder = reader.into_decoder()?;
    let (width, height) = decoder.dimensions();
    if decoder.total_bytes() > MAX_DECODE_ALLOCATION
        || u64::from(width) * u64::from(height) * 4 > MAX_RGBA_BYTES
    {
        return Err(image::ImageError::Limits(
            image::error::LimitError::from_kind(image::error::LimitErrorKind::InsufficientMemory),
        ));
    }
    image::DynamicImage::from_decoder(decoder)
}

/// Why [`decode_charged`] produced no image.
enum DecodeRefusal {
    /// The image is readable but its pixels would pass the decoded image budget.
    Budget,
    /// The bytes are not a raster image within the decode bounds.
    Image(image::ImageError),
}

/// [`decode_bounded`] within `budget`: the RGBA bytes are reserved from the
/// header before any pixel buffer is allocated, and charged to the image.
fn decode_charged(
    bytes: &Bytes,
    budget: &DecodedImageBudget,
) -> Result<RasterImageData, DecodeRefusal> {
    let rgba = bounded_rgba_bytes(bytes).and_then(|rgba| usize::try_from(rgba).ok());
    let reservation = match rgba {
        Some(rgba) => Some(budget.reserve(rgba).ok_or(DecodeRefusal::Budget)?),
        // Unreadable or out of bounds, so decoding fails before allocating.
        // Should a decoder still read an image, it is charged once decoded.
        None => None,
    };
    let image = decode_bounded(bytes).map_err(DecodeRefusal::Image)?;
    let (width, height) = (image.width(), image.height());
    let raster = RasterImageData::new(width, height, Arc::new(image.into_rgba8().into_raw()));
    let reservation = match reservation {
        Some(reservation) => reservation,
        None => budget
            .reserve(raster.data.len())
            .ok_or(DecodeRefusal::Budget)?,
    };
    reservation.track_raster(&raster);
    Ok(raster)
}

fn decode_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODED_SIDE);
    limits.max_image_height = Some(MAX_DECODED_SIDE);
    limits.max_alloc = Some(MAX_DECODE_ALLOCATION);
    limits
}

#[cfg(test)]
/// A PNG declaring `side`×`side` RGBA pixels, with no image data.
pub(crate) fn declared_png(side: u32) -> Bytes {
    declared_png_of(side, 6)
}

#[cfg(test)]
/// A PNG declaring `side`×`side` pixels of PNG colour type `colour`.
pub(crate) fn declared_png_of(side: u32, colour: u8) -> Bytes {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &byte in bytes {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    let mut chunk = b"IHDR".to_vec();
    chunk.extend(side.to_be_bytes());
    chunk.extend(side.to_be_bytes());
    chunk.extend([8, colour, 0, 0, 0]);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(13u32.to_be_bytes());
    png.extend(&chunk);
    png.extend(crc32(&chunk).to_be_bytes());
    for name in [b"IDAT", b"IEND"] {
        png.extend(0u32.to_be_bytes());
        png.extend(name);
        png.extend(crc32(name).to_be_bytes());
    }
    Bytes::from(png)
}

#[cfg(test)]
mod decode_limit_tests {
    use super::*;

    fn decode_error(bytes: Bytes) -> image::ImageResult<image::DynamicImage> {
        decode_bounded(&bytes)
    }

    #[test]
    fn declared_bitmaps_past_the_bounds_stop_before_decoding() {
        // 5000² RGBA is 100 MB: under the decoder's allocation bound, over
        // the RGBA bound.
        let rgba = decode_error(declared_png(5000));
        assert!(
            matches!(rgba, Err(image::ImageError::Limits(_))),
            "{rgba:?}"
        );
        // 8000² grayscale decodes into 64 MB but would become 256 MB RGBA.
        let gray = decode_error(declared_png_of(8000, 0));
        assert!(
            matches!(gray, Err(image::ImageError::Limits(_))),
            "{gray:?}"
        );
        let wide = decode_error(declared_png(MAX_DECODED_SIDE + 1));
        assert!(
            matches!(wide, Err(image::ImageError::Limits(_))),
            "{wide:?}"
        );
        // Within the bounds, the empty data is a decoding error instead.
        let small = decode_error(declared_png(64));
        assert!(
            matches!(small, Err(ref e) if !matches!(e, image::ImageError::Limits(_))),
            "{small:?}"
        );
    }
}
