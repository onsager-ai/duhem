//! Content-type sniffing for artifact serving + static export, split
//! out of `reader.rs` so the reader stays within the per-file prod
//! token budget (`xtask check-file-budget`). Re-exported from `reader`,
//! so callers keep the `reader::sniff_content_type` path.

/// Cheap content sniff for artifact serving and export extensions.
/// Blobs carry no media type in the stream (the observation's
/// `output_name` is a label, not a MIME), so the bytes decide.
pub fn sniff_content_type(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        // EBML header — Matroska/WebM. Playwright records WebM (#215).
        "video/webm"
    } else if serde_json::from_slice::<serde_json::Value>(bytes).is_ok() {
        "application/json"
    } else if std::str::from_utf8(bytes).is_ok() {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

/// Export-side file extension for a sniffed content type.
pub fn extension_for(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "video/webm" => "webm",
        "application/json" => "json",
        m if m.starts_with("text/plain") => "txt",
        _ => "bin",
    }
}
