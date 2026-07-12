//! Text extraction for uploaded document images, by shelling out to the
//! `tesseract` CLI (`tesseract-ocr` package — see `Dockerfile`'s runtime
//! stage) rather than an FFI binding crate: it keeps this project's only
//! system-level (non-Cargo) runtime dependency to "one binary on `PATH`",
//! with no libtesseract/libleptonica headers or linking to manage in the
//! build stage. Runs as detached background work (see
//! `web::handlers::documents::create`), per CLAUDE.md's OCR Engine Layer
//! rule — never inline in a request handler.
//!
//! PDFs (`extract_text_from_pdf`) follow the same "shell out, don't link"
//! precedent: `pdftoppm` (`poppler-utils` package — also in `Dockerfile`'s
//! runtime stage) rasterizes each page to a PNG, then each page image is
//! run through the same `tesseract` invocation used for direct image
//! uploads. Callers dispatch on content type via `extract`, so nothing
//! outside this module needs to know which content types need rasterizing
//! first.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
#[error("ocr error: {0}")]
pub struct OcrError(String);

/// PDF uploads are rasterized (`extract_text_from_pdf`) before OCR; every
/// other accepted content type goes straight through `extract_text`. Public
/// so `web::handlers::documents` can use the same constant for upload
/// validation and OCR-eligibility checks without duplicating the string.
pub const PDF_CONTENT_TYPE: &str = "application/pdf";

/// Extracts text from an uploaded document's bytes, dispatching on content
/// type: PDFs are rasterized page-by-page first (`extract_text_from_pdf`),
/// everything else is assumed to already be a raster image `tesseract` can
/// read directly (`extract_text`). The one place this project decides
/// "does this content type need rasterizing before OCR" — callers just
/// pass the bytes and the content type they were uploaded with.
pub async fn extract(content_type: &str, bytes: &[u8]) -> Result<String, OcrError> {
    if content_type == PDF_CONTENT_TYPE {
        extract_text_from_pdf(bytes).await
    } else {
        extract_text(bytes).await
    }
}

/// Deletes the temp file on drop so every fallible step in `run_tesseract`
/// (spawn, non-zero exit, non-UTF8 stdout) cleans up the same way without
/// needing a manual `remove_file` at each early-return point. Cleanup
/// itself is a brief synchronous `std::fs::remove_file` call — acceptable
/// for a single small image file in a background task, not worth a second
/// `spawn_blocking` just to avoid it.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Runs `tesseract` against an image file already on disk. Shared by
/// `extract_text` (which first writes the image bytes it's given to a temp
/// file) and `extract_text_from_pdf`'s per-page loop (which calls this
/// directly on the PNG `pdftoppm` already wrote to disk, rather than
/// reading those bytes back into memory just to have `extract_text` write
/// them straight back out to a second temp file).
async fn run_tesseract(path: &Path) -> Result<String, OcrError> {
    let output = tokio::process::Command::new("tesseract")
        .arg(path)
        .arg("stdout")
        .output()
        .await
        .map_err(|e| OcrError(format!("failed to spawn tesseract: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(OcrError(format!("tesseract exited with {}: {stderr}", output.status)));
    }

    String::from_utf8(output.stdout).map_err(|e| OcrError(format!("tesseract produced non-UTF-8 output: {e}")))
}

/// Extracts text from an image via the `tesseract` CLI. Writes to a real
/// temp file rather than piping over stdin: TIFF (one of the four accepted
/// upload types) has a history of trouble in Leptonica/tesseract's stdin
/// (`-`) path for non-seekable input, and a real file path sidesteps that
/// uniformly across every accepted type rather than needing per-format
/// branching.
async fn extract_text(image_bytes: &[u8]) -> Result<String, OcrError> {
    let path = std::env::temp_dir().join(format!("docuflow-ocr-{}", Uuid::new_v4()));
    tokio::fs::write(&path, image_bytes)
        .await
        .map_err(|e| OcrError(format!("failed to write temp file: {e}")))?;
    // Holds a tenant's private document bytes, briefly, on disk.
    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .await
        .map_err(|e| OcrError(format!("failed to set temp file permissions: {e}")))?;
    let temp_file = TempFile(path);

    run_tesseract(&temp_file.0).await
}

/// Same drop-cleanup rationale as `TempFile` above, but for the whole
/// per-PDF scratch directory `pdftoppm` writes its page PNGs into.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Extracts text from a PDF: rasterizes every page to a PNG via `pdftoppm`,
/// then runs `tesseract` directly against each page PNG already on disk (see
/// `run_tesseract`) in page order, joining the results with a page-separator
/// line. `pdftoppm` zero-pads its page-number filename suffix to the width
/// of the last page number (e.g. `page-01.png` .. `page-12.png`), so a plain
/// lexicographic sort of the produced filenames is already page order — no
/// numeric parsing needed.
async fn extract_text_from_pdf(pdf_bytes: &[u8]) -> Result<String, OcrError> {
    let dir = std::env::temp_dir().join(format!("docuflow-ocr-pdf-{}", Uuid::new_v4()));
    tokio::fs::create_dir(&dir)
        .await
        .map_err(|e| OcrError(format!("failed to create temp dir: {e}")))?;
    tokio::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
        .await
        .map_err(|e| OcrError(format!("failed to set temp dir permissions: {e}")))?;
    let temp_dir = TempDir(dir);

    let pdf_path = temp_dir.0.join("input.pdf");
    tokio::fs::write(&pdf_path, pdf_bytes)
        .await
        .map_err(|e| OcrError(format!("failed to write temp file: {e}")))?;
    // Holds a tenant's private document bytes, briefly, on disk.
    tokio::fs::set_permissions(&pdf_path, std::fs::Permissions::from_mode(0o600))
        .await
        .map_err(|e| OcrError(format!("failed to set temp file permissions: {e}")))?;

    let page_prefix = temp_dir.0.join("page");
    let output = tokio::process::Command::new("pdftoppm")
        .arg("-png")
        .arg(&pdf_path)
        .arg(&page_prefix)
        .output()
        .await
        .map_err(|e| OcrError(format!("failed to spawn pdftoppm: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(OcrError(format!("pdftoppm exited with {}: {stderr}", output.status)));
    }

    let mut page_paths = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&temp_dir.0)
        .await
        .map_err(|e| OcrError(format!("failed to list rasterized pages: {e}")))?;
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| OcrError(format!("failed to list rasterized pages: {e}")))?
    {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("png") {
            page_paths.push(path);
        }
    }
    if page_paths.is_empty() {
        return Err(OcrError("pdftoppm produced no pages".to_string()));
    }
    page_paths.sort();

    let mut pages = Vec::with_capacity(page_paths.len());
    for (index, page_path) in page_paths.iter().enumerate() {
        let text = run_tesseract(page_path).await?;
        pages.push(format!("--- page {} ---\n{text}", index + 1));
    }

    Ok(pages.join("\n\n"))
}
