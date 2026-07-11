//! Content sniffing for artifact serving (#215 adds WebM). Blobs carry
//! no media type in the stream, so the bytes decide the `Content-Type`
//! the artifact route serves and the extension a static export writes.

use duhem_dashboard::reader::{extension_for, sniff_content_type};

#[test]
fn webm_is_sniffed_from_its_ebml_magic() {
    // Playwright records WebM; the EBML header `1A 45 DF A3` leads it.
    let mut clip = vec![0x1A, 0x45, 0xDF, 0xA3];
    clip.extend_from_slice(&[0x93, 0x42, 0x82, 0x88]);
    assert_eq!(sniff_content_type(&clip), "video/webm");
    assert_eq!(extension_for("video/webm"), "webm");
}

#[test]
fn other_kinds_are_unaffected_by_the_webm_branch() {
    assert_eq!(sniff_content_type(&[0x89, b'P', b'N', b'G']), "image/png");
    assert_eq!(sniff_content_type(br#"{"a":1}"#), "application/json");
    assert_eq!(sniff_content_type(b"hello"), "text/plain; charset=utf-8");
}
