// --------------------------------------------------------------------------------------------------------------------
// Deluxe Paint's chunky IFF picture, FORM PBM, as STRONG.EXE's reader 2000:3801 accepts it (about_grab.md):
//   - FORM, a size, PBM, then BMHD (exactly 20 bytes, before BODY), an optional CMAP, and BODY, which ends the read.
//   - every chunk under 65,536 bytes: the reader's byte swap 2000:00eb sizes anything larger as megabytes.
//   - BODY stored or ByteRun1, 8 planes chunky; each CMAP byte is shifted right by two into the VGA palette.
// The writer keeps to that and refuses an odd width, which the game's reader reads one byte short per row.

use crate::error::{FormatError, Result};

/// The reader's limit on any one chunk.
pub const CHUNK_LIMIT: usize = 0x10000;
/// 256 colours of r, g, b.
pub const CMAP_LEN: usize = 768;
/// The BMHD the game's reader insists on, to the byte.
const BMHD_LEN: usize = 20;

/// One picture: chunky 8-bit pixels and a 768-byte CMAP.
#[derive(Clone, PartialEq, Eq)]
pub struct Lbm {
    pub width: usize,
    pub height: usize,
    /// Row major, width * height.
    pub pixels: Vec<u8>,
    /// 8-bit channels as IFF stores them; the game keeps the top six bits.
    pub cmap: [u8; CMAP_LEN],
}

impl std::fmt::Debug for Lbm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Lbm {}x{}", self.width, self.height)
    }
}

impl Lbm {
    /// The file bytes: FORM PBM with BMHD, CMAP and BODY, stored or ByteRun1.
    pub fn to_bytes(&self, compress: bool) -> Result<Vec<u8>> {
        if self.width == 0 || self.height == 0 || self.width > u16::MAX as usize || self.height > u16::MAX as usize {
            return Err(FormatError::malformed(format!("a {}x{} picture cannot be a BMHD", self.width, self.height)));
        }
        if self.width % 2 != 0 {
            return Err(FormatError::malformed(format!("width {} is odd, and 2000:3801 reads an odd width one byte short per row", self.width)));
        }
        if self.pixels.len() != self.width * self.height {
            return Err(FormatError::malformed(format!("{} pixels for a {}x{} picture", self.pixels.len(), self.width, self.height)));
        }
        let (w, h) = (self.width as u16, self.height as u16);
        let mut bmhd = Vec::with_capacity(BMHD_LEN);
        bmhd.extend_from_slice(&w.to_be_bytes());
        bmhd.extend_from_slice(&h.to_be_bytes());
        bmhd.extend_from_slice(&[0, 0, 0, 0]); // x, y origin
        bmhd.extend_from_slice(&[8, 0, u8::from(compress), 0]); // planes, masking, compression, pad
        bmhd.extend_from_slice(&[0, 0, 1, 1]); // transparent colour 0, aspect 1:1
        bmhd.extend_from_slice(&w.to_be_bytes());
        bmhd.extend_from_slice(&h.to_be_bytes());
        let body = if compress {
            let mut out = Vec::new();
            for row in self.pixels.chunks(self.width) {
                byterun1_row(row, &mut out);
            }
            out
        } else {
            self.pixels.clone()
        };
        let mut form = b"PBM ".to_vec();
        push_chunk(&mut form, b"BMHD", &bmhd)?;
        push_chunk(&mut form, b"CMAP", &self.cmap)?;
        push_chunk(&mut form, b"BODY", &body)?;
        let mut out = b"FORM".to_vec();
        out.extend_from_slice(&(form.len() as u32).to_be_bytes());
        out.extend_from_slice(&form);
        Ok(out)
    }

    /// Read a PBM back the way the game's reader would: chunks in any order, unknown ones
    /// skipped, BODY last. Planar ILBM is refused rather than guessed at.
    pub fn read(bytes: &[u8]) -> Result<Lbm> {
        let fail = |why: String| FormatError::malformed(format!("LBM: {why}"));
        if bytes.len() < 12 || &bytes[..4] != b"FORM" {
            return Err(fail("no FORM header".into()));
        }
        if &bytes[8..12] != b"PBM " {
            return Err(fail(format!("form type {:?} is not PBM", String::from_utf8_lossy(&bytes[8..12]))));
        }
        let mut at = 12usize;
        let mut header: Option<(usize, usize, u8, u8)> = None;
        let mut cmap = [0u8; CMAP_LEN];
        while at + 8 <= bytes.len() {
            let tag = &bytes[at..at + 4];
            let size = u32::from_be_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]) as usize;
            let data = bytes.get(at + 8..at + 8 + size).ok_or_else(|| fail(format!("chunk {:?} of {size} bytes runs past the file", String::from_utf8_lossy(tag))))?;
            if size >= CHUNK_LIMIT {
                return Err(fail(format!("chunk {:?} is {size} bytes, and the game's reader cannot size 64K or more", String::from_utf8_lossy(tag))));
            }
            match tag {
                b"BMHD" => {
                    if size != BMHD_LEN {
                        return Err(fail(format!("BMHD of {size} bytes, the game's reader wants exactly {BMHD_LEN}")));
                    }
                    let w = u16::from_be_bytes([data[0], data[1]]) as usize;
                    let h = u16::from_be_bytes([data[2], data[3]]) as usize;
                    header = Some((w, h, data[8], data[10]));
                }
                b"CMAP" => {
                    let n = size.min(CMAP_LEN);
                    cmap[..n].copy_from_slice(&data[..n]);
                }
                b"BODY" => {
                    let (w, h, planes, compression) = header.ok_or_else(|| fail("BODY before BMHD".into()))?;
                    if planes != 8 {
                        return Err(fail(format!("{planes} planes; the ungrab writes and reads 8")));
                    }
                    let stride = w + (w & 1);
                    let raw = if compression == 0 { data.get(..stride * h).map(|d| d.to_vec()).ok_or_else(|| fail("stored BODY is short".into()))? } else { unpack_byterun1(data, stride, h)? };
                    let mut pixels = Vec::with_capacity(w * h);
                    for row in raw.chunks(stride) {
                        pixels.extend_from_slice(&row[..w]);
                    }
                    return Ok(Lbm { width: w, height: h, pixels, cmap });
                }
                _ => {}
            }
            at += 8 + size + (size & 1);
        }
        Err(fail("no BODY".into()))
    }
}

/// One chunk: tag, big-endian size, data, a pad byte to keep the next chunk even.
fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) -> Result<()> {
    if data.len() >= CHUNK_LIMIT {
        return Err(FormatError::malformed(format!("{} chunk of {} bytes; 2000:3801 cannot size a chunk of 64K or more", String::from_utf8_lossy(tag), data.len())));
    }
    out.extend_from_slice(tag);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    Ok(())
}

/// ByteRun1 for one row: literals of up to 128 bytes, repeats of 3 to 128.
pub fn byterun1_row(row: &[u8], out: &mut Vec<u8>) {
    let mut literal: Vec<u8> = Vec::new();
    let flush = |literal: &mut Vec<u8>, out: &mut Vec<u8>| {
        for chunk in literal.chunks(128) {
            out.push((chunk.len() - 1) as u8);
            out.extend_from_slice(chunk);
        }
        literal.clear();
    };
    let mut i = 0usize;
    while i < row.len() {
        let mut j = i;
        while j < row.len() && j - i < 128 && row[j] == row[i] {
            j += 1;
        }
        if j - i >= 3 {
            flush(&mut literal, out);
            out.push((0x101 - (j - i)) as u8);
            out.push(row[i]);
            i = j;
        } else {
            literal.push(row[i]);
            i += 1;
        }
    }
    flush(&mut literal, out);
}

/// Unpack rows of ByteRun1, each exactly stride bytes; 0x80 is a no-op.
fn unpack_byterun1(src: &[u8], stride: usize, rows: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(stride * rows);
    let mut at = 0usize;
    let short = || FormatError::malformed("LBM: ByteRun1 BODY ends early");
    for _ in 0..rows {
        let end = out.len() + stride;
        while out.len() < end {
            let b = *src.get(at).ok_or_else(short)?;
            at += 1;
            if b < 0x80 {
                let count = b as usize + 1;
                out.extend_from_slice(src.get(at..at + count).ok_or_else(short)?);
                at += count;
            } else if b > 0x80 {
                let value = *src.get(at).ok_or_else(short)?;
                at += 1;
                out.extend(std::iter::repeat_n(value, 0x101 - b as usize));
            }
            if out.len() > end {
                return Err(FormatError::malformed("LBM: a ByteRun1 run crosses a row end"));
            }
        }
    }
    Ok(out)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: usize, height: usize) -> Lbm {
        let pixels = (0..width * height).map(|i| if (i / 7) % 3 == 0 { 9 } else { (i % 251) as u8 }).collect();
        let mut cmap = [0u8; CMAP_LEN];
        for (i, b) in cmap.iter_mut().enumerate() {
            *b = (i % 64) as u8 * 4;
        }
        Lbm { width, height, pixels, cmap }
    }

    #[test]
    fn stored_and_compressed_pictures_read_back() {
        let lbm = picture(320, 200);
        for compress in [false, true] {
            let bytes = lbm.to_bytes(compress).expect("writes");
            assert_eq!(&bytes[..4], b"FORM");
            assert_eq!(&bytes[8..12], b"PBM ");
            assert_eq!(u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize, bytes.len() - 8);
            assert_eq!(bytes[12 + 8 + 10], u8::from(compress), "the compression byte");
            assert_eq!(Lbm::read(&bytes).expect("reads"), lbm);
        }
    }

    #[test]
    fn byterun1_matches_the_python_rebuild() {
        // grab_trace.packbits_row on the same row: a 3-run, a literal, a 130-run split at 128.
        let mut row = vec![5u8, 5, 5, 1, 2];
        row.extend(std::iter::repeat_n(7u8, 130));
        let mut out = Vec::new();
        byterun1_row(&row, &mut out);
        assert_eq!(out, vec![0xfe, 5, 0x01, 1, 2, 0x81, 7, 0x01, 7, 7]);
        assert_eq!(unpack_byterun1(&out, row.len(), 1).expect("unpacks"), row);
    }

    #[test]
    fn what_the_game_cannot_read_is_refused() {
        let lbm = picture(321, 2);
        assert!(lbm.to_bytes(false).is_err(), "odd width");
        let wide = Lbm { pixels: (0..640 * 200).map(|i| (i / 40) as u8).collect(), ..picture(640, 200) };
        assert!(wide.to_bytes(false).is_err(), "a 128,000-byte stored BODY is over the chunk limit");
        assert!(wide.to_bytes(true).is_ok(), "compressed it fits");
        let mut bytes = picture(320, 2).to_bytes(false).expect("writes");
        bytes[8..12].copy_from_slice(b"ILBM");
        assert!(Lbm::read(&bytes).is_err(), "planar is not read");
        assert!(Lbm::read(b"FORM").is_err());
    }

    #[test]
    fn unknown_chunks_are_skipped_and_body_ends_the_read() {
        let lbm = picture(320, 4);
        let bytes = lbm.to_bytes(false).expect("writes");
        let mut form = bytes[8..12].to_vec();
        push_chunk(&mut form, b"DPPS", &[1, 2, 3]).expect("chunk");
        form.extend_from_slice(&bytes[12..]);
        let mut out = b"FORM".to_vec();
        out.extend_from_slice(&(form.len() as u32).to_be_bytes());
        out.extend_from_slice(&form);
        out.extend_from_slice(b"junk after the body");
        assert_eq!(Lbm::read(&out).expect("reads"), lbm);
    }
}
