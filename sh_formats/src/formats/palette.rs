// --------------------------------------------------------------------------------------------------------------------
// VGA palettes and the short pieces that get stamped over them.
//
// A palette payload is 768 bytes of 6-bit DAC values (about_assets.md). Widening to 8 bits
// replicates the top two bits rather than shifting, so white stays 255 instead of 252. A piece
// is 144 bytes, 48 colours, laid over part of the main palette before an image is drawn; where
// it lands is the archive's business (dat::palette_rules), not the format's.

use crate::error::{FormatError, Result};
use crate::file_format::{FileFormat, SourceFileType};

/// A full palette payload: 256 entries of 6-bit r, g, b.
pub const PALETTE_LEN: usize = 768;
/// A piece: 48 entries.
pub const PALETTE_PIECE_LEN: usize = 144;

/// A 6-bit VGA channel widened to 8 bits by replicating its top two bits.
pub fn widen_vga_channel(six: u8) -> u8 {
    let six = six & 0x3f;
    (six << 2) | (six >> 4)
}

/// Every byte a 6-bit value, which is what tells a palette from an image that happens to be 768 bytes.
fn all_six_bit(bytes: &[u8]) -> bool {
    bytes.iter().all(|b| *b < 64)
}

/// A palette widened to 8 bits per channel.
#[derive(Clone, PartialEq, Eq)]
pub struct Palette {
    pub colours: [[u8; 3]; 256],
}

impl std::fmt::Debug for Palette {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Palette")
    }
}

impl Palette {
    /// A copy with a piece stamped over the slots from first_slot on.
    pub fn stamped(&self, first_slot: usize, piece: &PalettePiece) -> Result<Palette> {
        if first_slot + piece.colours.len() > 256 {
            return Err(FormatError::malformed(format!("{} colours from slot {first_slot} run past the end of the palette", piece.colours.len())));
        }
        let mut out = self.clone();
        out.colours[first_slot..first_slot + piece.colours.len()].copy_from_slice(&piece.colours);
        Ok(out)
    }
}

impl FileFormat for Palette {
    const DESCRIPTION: &'static str = "a palette";

    fn read(bytes: &[u8]) -> Result<Palette> {
        if bytes.len() != PALETTE_LEN {
            return Err(FormatError::malformed(format!("{} bytes is not a {PALETTE_LEN}-byte palette", bytes.len())));
        }
        if !all_six_bit(bytes) {
            return Err(FormatError::malformed("a byte is over 63, so these are not VGA DAC values"));
        }
        let mut colours = [[0u8; 3]; 256];
        for (slot, rgb) in bytes.chunks(3).enumerate() {
            colours[slot] = [widen_vga_channel(rgb[0]), widen_vga_channel(rgb[1]), widen_vga_channel(rgb[2])];
        }
        Ok(Palette { colours })
    }

    fn source_type(&self) -> SourceFileType {
        SourceFileType::Palette
    }
}

/// A partial palette: 48 colours, widened, meant to be stamped over a full one.
#[derive(Clone, PartialEq, Eq)]
pub struct PalettePiece {
    pub colours: [[u8; 3]; 48],
}

impl std::fmt::Debug for PalettePiece {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PalettePiece")
    }
}

impl FileFormat for PalettePiece {
    const DESCRIPTION: &'static str = "a palette piece";

    fn read(bytes: &[u8]) -> Result<PalettePiece> {
        if bytes.len() != PALETTE_PIECE_LEN {
            return Err(FormatError::malformed(format!("{} bytes is not a {PALETTE_PIECE_LEN}-byte palette piece", bytes.len())));
        }
        if !all_six_bit(bytes) {
            return Err(FormatError::malformed("a byte is over 63, so these are not VGA DAC values"));
        }
        let mut colours = [[0u8; 3]; 48];
        for (slot, rgb) in bytes.chunks(3).enumerate() {
            colours[slot] = [widen_vga_channel(rgb[0]), widen_vga_channel(rgb[1]), widen_vga_channel(rgb[2])];
        }
        Ok(PalettePiece { colours })
    }

    fn source_type(&self) -> SourceFileType {
        SourceFileType::PalettePiece
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
    fn widening_keeps_the_ends() {
        assert_eq!(widen_vga_channel(0), 0);
        assert_eq!(widen_vga_channel(63), 255);
        assert_eq!(widen_vga_channel(0x20), 130);
    }

    #[test]
    fn a_palette_is_768_six_bit_bytes() {
        let mut raw = vec![0x3fu8; PALETTE_LEN];
        assert_eq!(Palette::read(&raw).expect("parses").colours[255], [255, 255, 255]);
        raw[7] = 64;
        assert!(Palette::read(&raw).is_err());
        assert!(Palette::read(&raw[..100]).is_err());
    }

    #[test]
    fn a_piece_stamps_over_a_run_of_slots() {
        let base = Palette { colours: [[9; 3]; 256] };
        let mut raw = vec![0u8; PALETTE_PIECE_LEN];
        raw[..6].copy_from_slice(&[0x3f, 0x20, 0x01, 0x00, 0x10, 0x3f]);
        let piece = PalettePiece::read(&raw).expect("parses");
        assert_eq!(piece.source_type(), SourceFileType::PalettePiece);
        let out = base.stamped(160, &piece).expect("stamps");
        assert_eq!(out.colours[159], [9, 9, 9]);
        assert_eq!(out.colours[160], [255, 130, 4]);
        assert_eq!(out.colours[161], [0, 65, 255]);
        assert_eq!(out.colours[162], [0, 0, 0]);
        assert_eq!(out.colours[207], [0, 0, 0]);
        assert_eq!(out.colours[208], [9, 9, 9]);
        assert!(base.stamped(209, &piece).is_err(), "runs off the end");
        assert!(PalettePiece::read(&raw[..100]).is_err());
    }
}
