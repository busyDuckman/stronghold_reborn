// --------------------------------------------------------------------------------------------------------------------
// The generic interface every payload reader offers, and the names file_type_detector gives
// payloads.

use crate::error::Result;
use crate::formats::audio::AudioHeader;
use crate::formats::image::ImageHeader;

/// A reader for one file format: bytes in, a value out, or a reason why not.
pub trait FileFormat: Sized {
    /// How an error message names the format: "an image", "a palette".
    const DESCRIPTION: &'static str;

    /// Read the bytes, or say why they are not this format. Never guesses.
    fn read(bytes: &[u8]) -> Result<Self>;

    /// What the detector calls a payload this reader accepts.
    fn source_type(&self) -> SourceFileType;
}

/// What a payload is, as file_type_detector names it. Image and Audio carry their parsed
/// headers so a listing never decodes twice.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SourceFileType {
    Image(ImageHeader),
    Audio(AudioHeader),
    /// 768 bytes, 256 entries of 6-bit r, g, b.
    Palette,
    /// 144 bytes, 48 entries; see dat::palette_rules for where it lands.
    PalettePiece,
    /// An IFF FORM outside a kind 3 header.
    Xmi,
    /// Creative Voice File.
    Voc,
    /// Autodesk Animator FLI, 320x200.
    Fli,
    /// A DOS executable.
    Exe,
    /// DIGPAK sound driver.
    Digpak,
    /// MIDPAK driver.
    Midpak,
    /// AIL/Miles sound driver.
    Adv,
    /// The save/scenario blob.
    Save,
    /// Plain ASCII: help and advisor strings.
    Text,
    /// Nothing matched.
    Unknown,
}

impl SourceFileType {
    /// A short label for listings: image1, image4, pal, pcm, xmi, txt. The asset browser's vocabulary.
    pub fn label(&self) -> String {
        match self {
            SourceFileType::Image(h) => format!("image{}", h.image_type.type_byte()),
            SourceFileType::Audio(h) => h.audio_type.extension().to_string(),
            other => other.extension().to_string(),
        }
    }

    /// The extension an extracted payload is written with.
    pub fn extension(&self) -> &'static str {
        match self {
            SourceFileType::Image(_) => "img",
            SourceFileType::Audio(h) => h.audio_type.extension(),
            SourceFileType::Palette => "pal",
            SourceFileType::PalettePiece => "palpiece",
            SourceFileType::Xmi => "xmi",
            SourceFileType::Voc => "voc",
            SourceFileType::Fli => "fli",
            SourceFileType::Exe => "exe",
            SourceFileType::Digpak => "com",
            SourceFileType::Midpak => "com",
            SourceFileType::Adv => "adv",
            SourceFileType::Save => "sav",
            SourceFileType::Text => "txt",
            SourceFileType::Unknown => "bin",
        }
    }

    pub fn is_image(&self) -> bool {
        matches!(self, SourceFileType::Image(_))
    }

    pub fn is_palette(&self) -> bool {
        matches!(self, SourceFileType::Palette)
    }

    pub fn image_header(&self) -> Option<&ImageHeader> {
        match self {
            SourceFileType::Image(h) => Some(h),
            _ => None,
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_extensions() {
        assert_eq!(SourceFileType::Palette.label(), "pal");
        assert_eq!(SourceFileType::Text.extension(), "txt");
        assert!(!SourceFileType::Unknown.is_image());
    }
}
