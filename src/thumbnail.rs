//! Generates a small preview JPEG for a document's dashboard/detail-page
//! thumbnail (feature 025) — decodes whatever raster bytes `run_ocr`
//! already has in hand (the original bytes for a direct image upload, or
//! `ocr::extract`'s already-rasterized PDF page 1) and re-encodes a
//! resized copy, mirroring `pdf_assemble.rs`'s decode-then-re-encode-as-
//! JPEG pattern rather than introducing a second image-handling approach.

const THUMBNAIL_MAX_DIMENSION: u32 = 200;
const THUMBNAIL_JPEG_QUALITY: u8 = 80;

#[derive(Debug, thiserror::Error)]
#[error("thumbnail error: {0}")]
pub struct ThumbnailError(String);

/// Decodes `bytes` (format auto-detected from its signature — the source
/// may be a JPEG/PNG/WebP/TIFF upload or a PDF-rasterized PNG page) and
/// returns a resized JPEG no larger than `THUMBNAIL_MAX_DIMENSION` on its
/// longest side, preserving aspect ratio. Never panics — a corrupt or
/// unrecognized image is just an `Err`, same "guess didn't pan out, not a
/// crash" spirit as `date_extract`/`doc_type_extract`.
pub fn generate(bytes: &[u8]) -> Result<Vec<u8>, ThumbnailError> {
    let decoded = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ThumbnailError(format!("failed to guess image format: {e}")))?
        .decode()
        .map_err(|e| ThumbnailError(format!("failed to decode image: {e}")))?;

    let resized = decoded.thumbnail(THUMBNAIL_MAX_DIMENSION, THUMBNAIL_MAX_DIMENSION);
    let rgb = image::DynamicImage::ImageRgb8(resized.into_rgb8());

    let mut jpeg_bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, THUMBNAIL_JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|e| ThumbnailError(format!("failed to encode thumbnail JPEG: {e}")))?;

    Ok(jpeg_bytes)
}

#[cfg(test)]
mod tests {
    use image::GenericImageView;

    use super::*;

    fn decoded_dimensions(jpeg_bytes: &[u8]) -> (u32, u32) {
        image::ImageReader::new(std::io::Cursor::new(jpeg_bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
            .dimensions()
    }

    #[test]
    fn generates_a_thumbnail_from_a_png() {
        let bytes = std::fs::read("tests/fixtures/english_sample.png").unwrap();
        let thumbnail = generate(&bytes).unwrap();
        let (width, height) = decoded_dimensions(&thumbnail);
        assert!(
            width <= 200 && height <= 200,
            "expected both dimensions within the cap, got {width}x{height}"
        );
    }

    #[test]
    fn generates_a_thumbnail_from_a_jpeg() {
        let bytes = std::fs::read("tests/fixtures/exif_dated_sample.jpg").unwrap();
        let thumbnail = generate(&bytes).unwrap();
        let (width, height) = decoded_dimensions(&thumbnail);
        assert!(
            width <= 200 && height <= 200,
            "expected both dimensions within the cap, got {width}x{height}"
        );
    }

    #[test]
    fn preserves_aspect_ratio() {
        let bytes = std::fs::read("tests/fixtures/english_sample.png").unwrap();
        let original = image::ImageReader::new(std::io::Cursor::new(&bytes))
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap();
        let (orig_w, orig_h) = original.dimensions();

        let thumbnail = generate(&bytes).unwrap();
        let (thumb_w, thumb_h) = decoded_dimensions(&thumbnail);

        let orig_ratio = orig_w as f64 / orig_h as f64;
        let thumb_ratio = thumb_w as f64 / thumb_h as f64;
        // A generous tolerance: `thumbnail()` rounds to whole pixels, so a
        // small thumbnail (e.g. 200x44) can't hit the original ratio
        // exactly — this only guards against a real distortion bug (a
        // stretched, not just rounded, resize).
        assert!(
            (orig_ratio - thumb_ratio).abs() < 0.1,
            "expected aspect ratio to be roughly preserved: original {orig_ratio}, thumbnail {thumb_ratio}"
        );
    }

    #[test]
    fn returns_an_error_instead_of_panicking_on_garbage_bytes() {
        assert!(generate(b"not an image").is_err());
    }
}
