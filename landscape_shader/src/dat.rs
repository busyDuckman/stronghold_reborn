// --------------------------------------------------------------------------------------------------------------------
// STRONG.DAT, the game's asset container: the index, the palettes and the image decoder.
//
// A port of what reversing/notes/about_assets.md establishes and reversing/scripts/strongdat.py
// checks against the real file:
//   - four bytes at offset 0 give the offset of the index table; rows are 21 bytes, of which
//     the payload offset and size are the only two that mean anything on disk
//   - an image is a kind 1 record; kind 2 wraps one (anchor x, y, then the image at +5) and
//     kind 4 wraps a kind 2 (next-frame pointer and a step byte, then the kind 2 at +6)
//   - the pixel stream is a PackBits variant where colour 0 is never written: that absence
//     is the transparency, so the decoder keeps a written mask beside the pixels
//   - a palette is a 768 byte payload of 6-bit VGA DAC values
// The first byte of a payload only nominates a layout; a payload is an image once its header
// and pixel stream account for its length exactly.

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

/// Where the CD build's container sits under the repo, relative to the repo root.
pub const STRONG_DAT_RELATIVE: &str = "achieve/archive.org/en_cdrom/unpacked/STRONG.DAT";
/// One index row.
const INDEX_ROW_LEN: usize = 21;
/// A palette payload: 256 entries of 6-bit r, g, b.
const PALETTE_LEN: usize = 768;

/// One decoded image: palette indices plus which pixels were written at all.
pub struct SpriteImage {
    pub width: u16,
    pub height: u16,
    /// Where the game puts the image relative to the point it draws at; kind 1 has none.
    pub anchor_x: i16,
    pub anchor_y: i16,
    /// The record kind the payload opened with: 1, 2 or 4.
    pub kind: u8,
    /// Row major palette indices; unwritten pixels hold 0.
    pub pixels: Vec<u8>,
    /// Row major; true where a pixel was written.
    pub mask: Vec<bool>,
}

impl std::fmt::Debug for SpriteImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SpriteImage kind {} {}x{} anchor ({}, {})", self.kind, self.width, self.height, self.anchor_x, self.anchor_y)
    }
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

/// A 6-bit VGA channel widened to 8 bits by replicating its top two bits.
fn widen(six: u8) -> u8 {
    let six = six & 0x3f;
    (six << 2) | (six >> 4)
}

impl Palette {
    /// A copy with a run of 6-bit colours stamped over the slots from first_slot on. The
    /// container keeps short palette pieces (144 bytes, 48 colours) after some images, meant
    /// to be laid over part of the main palette before the image is drawn.
    pub fn stamped(&self, first_slot: usize, six_bit: &[u8]) -> Result<Palette> {
        if six_bit.len() % 3 != 0 {
            bail!("a palette piece of {} bytes is not whole colours", six_bit.len());
        }
        let count = six_bit.len() / 3;
        if first_slot + count > 256 {
            bail!("{count} colours from slot {first_slot} run past the end of the palette");
        }
        let mut out = self.clone();
        for (i, rgb) in six_bit.chunks(3).enumerate() {
            out.colours[first_slot + i] = [widen(rgb[0]), widen(rgb[1]), widen(rgb[2])];
        }
        Ok(out)
    }
}

/// The container: the whole file in memory and its index.
pub struct StrongDat {
    bytes: Vec<u8>,
    entries: Vec<(usize, usize)>,
    path: PathBuf,
}

impl std::fmt::Debug for StrongDat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StrongDat {} entries from {}", self.entries.len(), self.path.display())
    }
}

impl StrongDat {
    /// Look for the container from the working directory upwards (the crate runs from either
    /// its own folder or the repo root), or None if it is not on this machine.
    pub fn find() -> Option<PathBuf> {
        let mut dir = std::env::current_dir().ok()?;
        for _ in 0..4 {
            let candidate = dir.join(STRONG_DAT_RELATIVE);
            if candidate.is_file() {
                return Some(candidate);
            }
            if !dir.pop() {
                break;
            }
        }
        None
    }

    /// Read and index the container, refusing a file whose index does not add up.
    pub fn open(path: &Path) -> Result<StrongDat> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        StrongDat::from_bytes(bytes, path.to_path_buf())
    }

    fn from_bytes(bytes: Vec<u8>, path: PathBuf) -> Result<StrongDat> {
        if bytes.len() < 4 {
            bail!("{} is too short to be a container", path.display());
        }
        let table = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        if table > bytes.len() || (bytes.len() - table) % INDEX_ROW_LEN != 0 {
            bail!("{}: index table offset {table} does not divide the file into {INDEX_ROW_LEN}-byte rows", path.display());
        }
        let mut entries = Vec::with_capacity((bytes.len() - table) / INDEX_ROW_LEN);
        let mut expected = 4usize;
        for row in bytes[table..].chunks(INDEX_ROW_LEN) {
            let offset = u32::from_le_bytes([row[1], row[2], row[3], row[4]]) as usize;
            let size = u32::from_le_bytes([row[5], row[6], row[7], row[8]]) as usize;
            // Payloads are contiguous from offset 4 to the table; a wrong stride cannot pass this.
            if offset != expected || offset + size > table {
                bail!("{}: entry {} at {offset} (+{size}) breaks the contiguous payload run", path.display(), entries.len());
            }
            expected = offset + size;
            entries.push((offset, size));
        }
        if expected != table {
            bail!("{}: the last payload ends at {expected}, the table starts at {table}", path.display());
        }
        Ok(StrongDat { bytes, entries, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// One payload's bytes.
    pub fn payload(&self, index: usize) -> Result<&[u8]> {
        let (offset, size) = *self.entries.get(index).ok_or_else(|| anyhow!("entry {index} is past the {} in the container", self.entries.len()))?;
        Ok(&self.bytes[offset..offset + size])
    }

    /// The indices of every palette payload, in file order.
    pub fn palette_indices(&self) -> Vec<usize> {
        (0..self.entries.len()).filter(|i| self.entries[*i].1 == PALETTE_LEN).collect()
    }

    /// A palette entry, widened from 6 to 8 bits by replicating the top two bits.
    pub fn palette(&self, index: usize) -> Result<Palette> {
        let payload = self.payload(index)?;
        if payload.len() != PALETTE_LEN {
            bail!("entry {index} is {} bytes, not a {PALETTE_LEN}-byte palette", payload.len());
        }
        let mut colours = [[0u8; 3]; 256];
        for (slot, rgb) in payload.chunks(3).enumerate() {
            for channel in 0..3 {
                colours[slot][channel] = widen(rgb[channel]);
            }
        }
        Ok(Palette { colours })
    }

    /// Decode an image entry, or say why the entry is not one.
    pub fn image(&self, index: usize) -> Result<SpriteImage> {
        parse_image(self.payload(index)?).with_context(|| format!("entry {index}"))
    }
}

// --------------------------------------------------------------------------------------------------------------------
// The image decoder

/// Decode an image payload. The kind byte nominates where the header sits; the header and
/// the pixel stream then have to account for the payload exactly.
pub fn parse_image(payload: &[u8]) -> Result<SpriteImage> {
    let kind = *payload.first().ok_or_else(|| anyhow!("empty payload"))?;
    let (prefix, anchor_at) = match kind {
        1 => (1usize, None),
        2 => (6, Some(1usize)),
        4 => (12, Some(7)),
        other => bail!("kind {other} is not an image kind"),
    };
    let header = prefix + 8;
    if payload.len() < header {
        bail!("payload of {} bytes is shorter than a {header}-byte header", payload.len());
    }
    let word = |at: usize| u16::from_le_bytes([payload[at], payload[at + 1]]);
    let (data_size, zero, width, height) = (word(prefix) as usize, word(prefix + 2), word(prefix + 4), word(prefix + 6));
    if header + data_size != payload.len() {
        bail!("declared data size {data_size} plus header {header} is not the payload size {}", payload.len());
    }
    if zero != 0 {
        bail!("the always-zero header word is {zero}");
    }
    if width == 0 || height == 0 {
        bail!("degenerate size {width}x{height}");
    }
    let (anchor_x, anchor_y) = match anchor_at {
        Some(at) => (word(at) as i16, word(at + 2) as i16),
        None => (0, 0),
    };
    let (pixels, mask, used) = decode_rle(&payload[header..], width as usize * height as usize)?;
    if used != data_size {
        bail!("decode consumed {used} bytes, the header declared {data_size}");
    }
    Ok(SpriteImage { width, height, anchor_x, anchor_y, kind, pixels, mask })
}

/// Expand the PackBits variant into exactly npix pixels. Returns the pixels, the written
/// mask and how many input bytes were consumed.
///   - 0x80 is filler and is skipped
///   - below 0x80 is a literal run of b+1 bytes
///   - above 0x80 is a run of 0x101-b copies of the next byte
/// Colour 0 is never written; the write position moves on and the mask stays false.
pub fn decode_rle(src: &[u8], npix: usize) -> Result<(Vec<u8>, Vec<bool>, usize)> {
    let mut out = vec![0u8; npix];
    let mut mask = vec![false; npix];
    let mut at = 0usize;
    let mut pos = 0usize;
    while pos < npix {
        let b = *src.get(at).ok_or_else(|| anyhow!("ran out of input at pixel {pos} of {npix}"))?;
        at += 1;
        if b == 0x80 {
            continue;
        }
        if b < 0x80 {
            let count = b as usize + 1;
            if at + count > src.len() {
                bail!("literal run of {count} overruns the input");
            }
            if pos + count > npix {
                bail!("literal run overruns the image at pixel {pos}");
            }
            for (k, value) in src[at..at + count].iter().enumerate() {
                if *value != 0 {
                    out[pos + k] = *value;
                    mask[pos + k] = true;
                }
            }
            at += count;
            pos += count;
        } else {
            let count = 0x101 - b as usize;
            let value = *src.get(at).ok_or_else(|| anyhow!("run length with no value byte"))?;
            at += 1;
            if pos + count > npix {
                bail!("run of {count} overruns the image at pixel {pos}");
            }
            if value != 0 {
                out[pos..pos + count].fill(value);
                mask[pos..pos + count].fill(true);
            }
            pos += count;
        }
    }
    Ok((out, mask, at))
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests. The real container is not in the repo; the tests that need it skip when it is absent.

#[cfg(test)]
mod tests {
    use super::*;

    /// A kind 1 image, 3 wide by 2 tall: row one is a literal 1,0,2, row two a run of three 5s.
    fn tiny_kind_1() -> Vec<u8> {
        let stream = [0x02u8, 1, 0, 2, 0xfe, 5];
        let mut payload = vec![1u8];
        payload.extend_from_slice(&(stream.len() as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&3u16.to_le_bytes());
        payload.extend_from_slice(&2u16.to_le_bytes());
        payload.extend_from_slice(&stream);
        payload
    }

    #[test]
    fn a_kind_1_image_decodes_and_zero_is_unwritten() {
        let image = parse_image(&tiny_kind_1()).expect("decodes");
        assert_eq!((image.width, image.height, image.kind), (3, 2, 1));
        assert_eq!(image.pixels, vec![1, 0, 2, 5, 5, 5]);
        assert_eq!(image.mask, vec![true, false, true, true, true, true]);
    }

    #[test]
    fn kind_2_and_kind_4_wrap_the_image_with_anchors() {
        let inner = tiny_kind_1();
        let mut kind_2 = vec![2u8];
        kind_2.extend_from_slice(&(-7i16).to_le_bytes());
        kind_2.extend_from_slice(&(12i16).to_le_bytes());
        kind_2.extend_from_slice(&inner);
        let image = parse_image(&kind_2).expect("decodes");
        assert_eq!((image.kind, image.anchor_x, image.anchor_y), (2, -7, 12));
        let mut kind_4 = vec![4u8, 0, 0, 0, 0, 3];
        kind_4.extend_from_slice(&kind_2);
        let image = parse_image(&kind_4).expect("decodes");
        assert_eq!((image.kind, image.anchor_x, image.anchor_y, image.width), (4, -7, 12, 3));
    }

    #[test]
    fn bad_payloads_are_refused() {
        let mut short = tiny_kind_1();
        short.pop();
        assert!(parse_image(&short).is_err());
        let mut wrong_kind = tiny_kind_1();
        wrong_kind[0] = 3;
        assert!(parse_image(&wrong_kind).is_err());
        assert!(decode_rle(&[0x00], 3).is_err());
        let (pixels, _, used) = decode_rle(&[0x80, 0x80, 0xff, 9], 2).expect("filler is skipped");
        assert_eq!((pixels, used), (vec![9, 9], 4));
    }

    #[test]
    fn a_synthetic_container_indexes() {
        let a = tiny_kind_1();
        let b = vec![0u8; 768];
        let mut bytes = vec![0u8; 4];
        bytes.extend_from_slice(&a);
        bytes.extend_from_slice(&b);
        let table = bytes.len() as u32;
        bytes[..4].copy_from_slice(&table.to_le_bytes());
        for (offset, size) in [(4usize, a.len()), (4 + a.len(), b.len())] {
            let mut row = vec![0u8; INDEX_ROW_LEN];
            row[1..5].copy_from_slice(&(offset as u32).to_le_bytes());
            row[5..9].copy_from_slice(&(size as u32).to_le_bytes());
            bytes.extend_from_slice(&row);
        }
        let dat = StrongDat::from_bytes(bytes, PathBuf::from("synthetic")).expect("indexes");
        assert_eq!(dat.len(), 2);
        assert_eq!(dat.palette_indices(), vec![1]);
        assert_eq!(dat.image(0).expect("image").width, 3);
        assert!(dat.image(1).is_err());
        assert_eq!(dat.palette(1).expect("palette").colours[0], [0, 0, 0]);
    }

    #[test]
    fn a_palette_piece_stamps_over_a_run_of_slots() {
        let base = Palette { colours: [[9; 3]; 256] };
        let piece = [0x3f, 0x20, 0x01, 0x00, 0x10, 0x3f];
        let out = base.stamped(160, &piece).expect("stamps");
        assert_eq!(out.colours[159], [9, 9, 9]);
        assert_eq!(out.colours[160], [255, 130, 4]);
        assert_eq!(out.colours[161], [0, 65, 255]);
        assert_eq!(out.colours[162], [9, 9, 9]);
        assert!(base.stamped(255, &piece).is_err(), "runs off the end");
        assert!(base.stamped(0, &piece[..4]).is_err(), "not whole colours");
    }

    #[test]
    fn the_real_container_decodes_every_image_it_claims() {
        let Some(path) = StrongDat::find() else {
            println!("STRONG.DAT not on this machine; skipping");
            return;
        };
        let dat = StrongDat::open(&path).expect("opens");
        assert_eq!(dat.len(), 3451);
        assert_eq!(dat.palette_indices().len(), 56);
        let mut images = 0;
        for index in 0..dat.len() {
            if dat.image(index).is_ok() {
                images += 1;
            }
        }
        assert_eq!(images, 3192);
        let tree = dat.image(300).expect("the first tree entry is an image");
        assert!(tree.width > 4 && tree.height > 4);
    }
}
