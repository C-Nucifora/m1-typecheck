//! Tolerant text decoding for MoTeC's XML files (`.m1prj` / `.m1cfg` / `.m1dbc`).
//!
//! MoTeC writes these declared `<?xml version="1.0"?>` — no `encoding` attribute,
//! which per the XML spec means UTF-8 — but emits **Windows-1252** bytes for
//! non-ASCII characters. A CAN signal whose unit is a yaw rate, for example,
//! stores `°/s` with the degree sign as the single byte `0xB0`, which is not
//! valid UTF-8. A strict `read_to_string` of such a file errors with "stream did
//! not contain valid UTF-8", and that failure used to abort the entire
//! project-model build (so every channel/parameter became an unresolved
//! built-in with no type).
//!
//! We therefore read the raw bytes and decode tolerantly: try UTF-8 first (the
//! common case, and what the declaration claims), and on failure fall back to
//! Windows-1252 — a superset of Latin-1 — which never fails and recovers `°`,
//! `±`, `µ`, etc. exactly.

use std::io;
use std::path::Path;

/// Read a MoTeC XML file as text, decoding UTF-8 with a Windows-1252 fallback.
pub fn read_motec_xml(path: &Path) -> io::Result<String> {
    Ok(decode(std::fs::read(path)?))
}

/// Decode bytes as UTF-8, falling back to Windows-1252 if they are not valid
/// UTF-8. Pure, so it is unit-testable without touching the filesystem.
pub fn decode(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(s) => s,
        // `into_bytes` reuses the original buffer (no UTF-8 was produced).
        Err(e) => cp1252_to_string(&e.into_bytes()),
    }
}

fn cp1252_to_string(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| cp1252_char(b)).collect()
}

/// Map a single Windows-1252 byte to its Unicode scalar. `0x00–0x7F` and
/// `0xA0–0xFF` are identical to Latin-1 (the first 256 Unicode code points), so
/// only the `0x80–0x9F` "C1" range needs an explicit table. The five bytes
/// undefined in CP1252 map to U+FFFD.
fn cp1252_char(b: u8) -> char {
    match b {
        0x80 => '\u{20AC}', // €
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}', // ™
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        // Undefined in Windows-1252.
        0x81 | 0x8D | 0x8F | 0x90 | 0x9D => '\u{FFFD}',
        // 0x00–0x7F (ASCII) and 0xA0–0xFF (Latin-1) are 1:1 with Unicode.
        _ => b as char,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_passes_through_unchanged() {
        assert_eq!(decode("Qty=\"°/s\"".as_bytes().to_vec()), "Qty=\"°/s\"");
    }

    #[test]
    fn windows_1252_degree_sign_is_recovered() {
        // MoTeC stores `°` as the single CP1252 byte 0xB0, which is invalid UTF-8.
        let mut bytes = b"Qty=\"".to_vec();
        bytes.push(0xB0); // °
        bytes.extend_from_slice(b"/s\"");
        assert_eq!(decode(bytes), "Qty=\"°/s\"");
    }

    #[test]
    fn windows_1252_c1_range_and_latin1() {
        // ± (0xB1) and µ (0xB5) are Latin-1; € (0x80) and ™ (0x99) are CP1252 C1.
        let bytes = vec![0xB1, 0xB5, 0x80, 0x99];
        assert_eq!(decode(bytes), "±µ€™");
    }
}
