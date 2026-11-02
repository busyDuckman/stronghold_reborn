// --------------------------------------------------------------------------------------------------------------------
// Which format a payload is.
//
// There is no type field in the archive (about_assets.md, "The trap"). A payload is a given
// format once that format's reader accepts it, and the readers are tried in an order that
// mirrors detect_file_type() in reversing/scripts/strongdat.py so the two agree entry for entry. The
// files the packer took as they were rather than converting (sound drivers, help text, the
// FLI sequences, a save blob) are not decoded; each still carries its own signature and the
// predicates below name them by it.

use crate::file_format::{FileFormat, SourceFileType};
use crate::formats::audio::AudioHeader;
use crate::formats::image::IndexedImage;
use crate::formats::palette::{Palette, PalettePiece};

/// Name one payload. The game's own layouts first, then the foreign signatures, then text.
pub fn detect_file_type(bytes: &[u8]) -> SourceFileType {
    if let Ok(audio) = AudioHeader::read(bytes) {
        return audio.source_type();
    }
    // Decoding the pixels is the test: a header that adds up over a stream that does not is not an image.
    if let Ok(image) = IndexedImage::read(bytes) {
        return image.source_type();
    }
    if is_xmi(bytes) {
        return SourceFileType::Xmi;
    }
    if is_voc(bytes) {
        return SourceFileType::Voc;
    }
    if Palette::read(bytes).is_ok() {
        return SourceFileType::Palette;
    }
    if PalettePiece::read(bytes).is_ok() {
        return SourceFileType::PalettePiece;
    }
    if is_fli(bytes) {
        return SourceFileType::Fli;
    }
    if is_exe(bytes) {
        return SourceFileType::Exe;
    }
    if is_digpak(bytes) {
        return SourceFileType::Digpak;
    }
    if is_midpak(bytes) {
        return SourceFileType::Midpak;
    }
    if is_adv(bytes) {
        return SourceFileType::Adv;
    }
    if is_save(bytes) {
        return SourceFileType::Save;
    }
    if is_text(bytes) {
        return SourceFileType::Text;
    }
    SourceFileType::Unknown
}

// --------------------------------------------------------------------------------------------------------------------
// Signature predicates for the files the packer took whole

/// An IFF FORM outside a kind 3 header: XMI music.
fn is_xmi(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes.starts_with(b"FORM")
}

fn is_voc(bytes: &[u8]) -> bool {
    bytes.starts_with(b"Creative Voice File")
}

/// Autodesk Animator: 0xAF11 at offset 4, then 320 by 200.
fn is_fli(bytes: &[u8]) -> bool {
    let word = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
    bytes.len() >= 12 && word(4) == 0xaf11 && word(8) == 320 && word(10) == 200
}

fn is_exe(bytes: &[u8]) -> bool {
    bytes.starts_with(b"MZ") || bytes.starts_with(b"ZM")
}

/// AIL sound drivers open with a near jump over an inline banner string.
fn is_digpak(bytes: &[u8]) -> bool {
    bytes.first() == Some(&0xe9) && contains(head(bytes), b"DIGPAK")
}

fn is_midpak(bytes: &[u8]) -> bool {
    bytes.first() == Some(&0xe9) && contains(head(bytes), b"MIDPAK")
}

/// AIL/Miles drivers open with a word and then their own copyright banner.
fn is_adv(bytes: &[u8]) -> bool {
    contains(head(bytes), b"Miles Design")
}

/// A version number, then the words.
fn is_save(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(32)];
    let start = sample.iter().position(|b| !b"0123456789. ".contains(b)).unwrap_or(sample.len());
    sample[start..].starts_with(b"Stronghold Save")
}

/// More than 95 percent printable ASCII in the first 256 bytes.
fn is_text(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(256)];
    let printable = sample.iter().filter(|b| (32..127).contains(*b) || matches!(**b, 9 | 10 | 13)).count();
    !sample.is_empty() && printable as f64 / sample.len() as f64 > 0.95
}

fn head(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.len().min(64)]
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_are_named() {
        assert_eq!(detect_file_type(b"FORMXDIRxxxxxxxx"), SourceFileType::Xmi);
        assert_eq!(detect_file_type(b"Creative Voice File\x1a"), SourceFileType::Voc);
        assert_eq!(detect_file_type(&[0x3f; 768]), SourceFileType::Palette);
        assert_eq!(detect_file_type(&[0x10; 144]), SourceFileType::PalettePiece);
        assert_eq!(detect_file_type(b"MZ\x00\x00"), SourceFileType::Exe);
        assert_eq!(detect_file_type(b"\xe9\x10\x00 DIGPAK driver"), SourceFileType::Digpak);
        assert_eq!(detect_file_type(b"\xe9\x10\x00 MIDPAK driver"), SourceFileType::Midpak);
        assert_eq!(detect_file_type(b"\x01\x00Copyright Miles Design"), SourceFileType::Adv);
        assert_eq!(detect_file_type(b"1.03 Stronghold Save blob"), SourceFileType::Save);
        assert_eq!(detect_file_type(b"Welcome, my lord. The advisor speaks.\r\n"), SourceFileType::Text);
        assert_eq!(detect_file_type(&[0xfe, 0xed, 0xfa, 0xce, 0, 0, 0]), SourceFileType::Unknown);
        let mut fli = vec![0u8; 16];
        fli[4..6].copy_from_slice(&0xaf11u16.to_le_bytes());
        fli[8..10].copy_from_slice(&320u16.to_le_bytes());
        fli[10..12].copy_from_slice(&200u16.to_le_bytes());
        assert_eq!(detect_file_type(&fli), SourceFileType::Fli);
    }

    #[test]
    fn each_predicate_needs_its_signature() {
        assert!(!is_xmi(b"FORM"), "too short to be a file");
        assert!(!is_save(b"A Stronghold Save"), "only digits and dots may lead");
        assert!(!is_text(&[0xfe, 0xed, 0xfa, 0xce, 0, 0, 0]));
        assert!(!is_text(b""));
        assert!(!is_digpak(b"\x00\x00DIGPAK"), "no jump in front");
        assert!(!is_fli(&[0; 8]));
    }

    #[test]
    fn a_palette_opening_with_an_image_type_byte_is_still_a_palette() {
        // Entries 1845, 2468, 3296 open with 02 and 3440 with 01: the image header never adds up.
        let mut pal = vec![0x02u8; 768];
        pal[1] = 0x3f;
        assert_eq!(detect_file_type(&pal), SourceFileType::Palette);
    }
}
