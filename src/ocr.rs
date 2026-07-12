//! Text extraction for uploaded document images, by shelling out to the
//! `tesseract` CLI (`tesseract-ocr` package — see `Dockerfile`'s runtime
//! stage) rather than an FFI binding crate: it keeps this project's only
//! system-level (non-Cargo) runtime dependency to "one binary on `PATH`",
//! with no libtesseract/libleptonica headers or linking to manage in the
//! build stage. Runs as detached background work (see
//! `web::handlers::documents::create`), per CLAUDE.md's OCR Engine Layer
//! rule — never inline in a request handler.

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
#[error("ocr error: {0}")]
pub struct OcrError(String);

/// Deletes the temp file on drop so every fallible step in `extract_text`
/// (write, spawn, non-zero exit, non-UTF8 stdout) cleans up the same way
/// without needing a manual `remove_file` at each early-return point.
/// Cleanup itself is a brief synchronous `std::fs::remove_file` call —
/// acceptable for a single small image file in a background task, not
/// worth a second `spawn_blocking` just to avoid it.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Extracts text from an image via the `tesseract` CLI. Writes to a real
/// temp file rather than piping over stdin: TIFF (one of the four accepted
/// upload types) has a history of trouble in Leptonica/tesseract's stdin
/// (`-`) path for non-seekable input, and a real file path sidesteps that
/// uniformly across every accepted type rather than needing per-format
/// branching.
pub async fn extract_text(image_bytes: &[u8]) -> Result<String, OcrError> {
    let path = std::env::temp_dir().join(format!("docuflow-ocr-{}", Uuid::new_v4()));
    tokio::fs::write(&path, image_bytes)
        .await
        .map_err(|e| OcrError(format!("failed to write temp file: {e}")))?;
    // Holds a tenant's private document bytes, briefly, on disk.
    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .await
        .map_err(|e| OcrError(format!("failed to set temp file permissions: {e}")))?;
    let temp_file = TempFile(path);

    let output = tokio::process::Command::new("tesseract")
        .arg(&temp_file.0)
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
