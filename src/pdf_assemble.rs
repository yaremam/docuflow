//! Assembles captured phone-scan pages (JPEG/PNG) into a single PDF
//! (feature 022). Pure-Rust via `lopdf` — no system dependency, matching
//! the `qrcode` precedent (TDR 009 Alternative H) rather than the
//! tesseract/pdftoppm CLI one; see TDR 022 §2 Alternatives D/E.
//!
//! JPEG pages are embedded byte-for-byte as `DCTDecode` streams (no
//! recompression — the `image` crate only reads their header for
//! dimensions/color type). PNG pages, and JPEGs in exotic color layouts,
//! are decoded and re-encoded as RGB JPEG instead of hand-rolling PNG
//! predictor embedding.

use image::ImageDecoder;
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream};

/// One captured page, as stored by `submit_scan`.
pub struct PageImage {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PdfAssembleError {
    #[error("cannot assemble a PDF from zero pages")]
    Empty,
    #[error("page {page}: {message}")]
    Page { page: usize, message: String },
    #[error("pdf serialization: {0}")]
    Serialize(String),
}

/// Every page is placed at `pixels × 72/150` points: `ocr::extract_text_from_pdf`
/// rasterizes with `pdftoppm` at its default 150 DPI, so this factor makes
/// the OCR pass reproduce the original photo's pixel dimensions almost
/// exactly — nothing lost to a small page box, no memory wasted inflating
/// a large one (TDR 022 §2).
const POINTS_PER_PIXEL: f32 = 72.0 / 150.0;

/// JPEG quality for pages that have to be re-encoded (PNG input, or a JPEG
/// whose color layout can't pass through as-is).
const REENCODE_JPEG_QUALITY: u8 = 90;

struct PreparedPage {
    jpeg_bytes: Vec<u8>,
    width: u32,
    height: u32,
    /// PDF color space name matching the embedded JPEG's components.
    color_space: &'static str,
}

fn page_err(page: usize, message: impl std::fmt::Display) -> PdfAssembleError {
    PdfAssembleError::Page {
        page,
        message: message.to_string(),
    }
}

/// Decodes fully and re-encodes as an RGB JPEG — the fallback path for
/// anything that isn't already a directly embeddable JPEG.
fn reencode_as_rgb_jpeg(
    page_number: usize,
    bytes: &[u8],
    format: image::ImageFormat,
) -> Result<PreparedPage, PdfAssembleError> {
    let decoded = image::ImageReader::with_format(std::io::Cursor::new(bytes), format)
        .decode()
        .map_err(|e| page_err(page_number, format!("failed to decode image: {e}")))?;
    let rgb = image::DynamicImage::ImageRgb8(decoded.to_rgb8());

    let mut jpeg_bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_bytes, REENCODE_JPEG_QUALITY)
        .encode_image(&rgb)
        .map_err(|e| page_err(page_number, format!("failed to re-encode as JPEG: {e}")))?;

    Ok(PreparedPage {
        jpeg_bytes,
        width: rgb.width(),
        height: rgb.height(),
        color_space: "DeviceRGB",
    })
}

fn prepare_page(page_number: usize, page: &PageImage) -> Result<PreparedPage, PdfAssembleError> {
    match page.content_type.as_str() {
        "image/jpeg" => {
            // Header-only read: dimensions + color type, no pixel decode.
            let decoder = image::ImageReader::with_format(
                std::io::Cursor::new(&page.bytes),
                image::ImageFormat::Jpeg,
            )
            .into_decoder()
            .map_err(|e| page_err(page_number, format!("failed to read JPEG header: {e}")))?;
            let (width, height) = decoder.dimensions();
            let color_space = match decoder.color_type() {
                image::ColorType::L8 => Some("DeviceGray"),
                image::ColorType::Rgb8 => Some("DeviceRGB"),
                // CMYK or anything else a phone won't produce: fall through
                // to the decode-and-re-encode path below.
                _ => None,
            };
            match color_space {
                Some(color_space) => Ok(PreparedPage {
                    jpeg_bytes: page.bytes.clone(),
                    width,
                    height,
                    color_space,
                }),
                None => reencode_as_rgb_jpeg(page_number, &page.bytes, image::ImageFormat::Jpeg),
            }
        }
        "image/png" => reencode_as_rgb_jpeg(page_number, &page.bytes, image::ImageFormat::Png),
        other => Err(page_err(
            page_number,
            format!("unsupported page content type: {other}"),
        )),
    }
}

/// Builds a PDF with one page per input image, in input order. `pages` is
/// skipped so the byte-carrying page images never appear in tracing fields
/// (TDR 022 §4).
#[tracing::instrument(skip(pages))]
pub fn images_to_pdf(pages: &[PageImage]) -> Result<Vec<u8>, PdfAssembleError> {
    if pages.is_empty() {
        return Err(PdfAssembleError::Empty);
    }

    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut kids: Vec<Object> = Vec::with_capacity(pages.len());

    for (index, page) in pages.iter().enumerate() {
        let page_number = index + 1;
        let prepared = prepare_page(page_number, page)?;
        let width_pt = prepared.width as f32 * POINTS_PER_PIXEL;
        let height_pt = prepared.height as f32 * POINTS_PER_PIXEL;

        let image_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => prepared.width as i64,
                "Height" => prepared.height as i64,
                "ColorSpace" => prepared.color_space,
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            },
            prepared.jpeg_bytes,
        ));

        let content = Content {
            operations: vec![
                Operation::new("q", vec![]),
                Operation::new(
                    "cm",
                    vec![
                        width_pt.into(),
                        0f32.into(),
                        0f32.into(),
                        height_pt.into(),
                        0f32.into(),
                        0f32.into(),
                    ],
                ),
                Operation::new("Do", vec![Object::Name(b"Im0".to_vec())]),
                Operation::new("Q", vec![]),
            ],
        };
        let content_bytes = content.encode().map_err(|e| {
            page_err(
                page_number,
                format!("failed to encode page content stream: {e}"),
            )
        })?;
        let content_id = doc.add_object(Stream::new(dictionary! {}, content_bytes));

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), width_pt.into(), height_pt.into()],
            "Contents" => content_id,
            "Resources" => dictionary! {
                "XObject" => dictionary! { "Im0" => image_id },
            },
        });
        kids.push(page_id.into());
    }

    let page_count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => page_count,
        }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);

    let mut out = Vec::new();
    doc.save_to(&mut out)
        .map_err(|e| PdfAssembleError::Serialize(e.to_string()))?;
    Ok(out)
}
