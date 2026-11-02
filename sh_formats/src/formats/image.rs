// --------------------------------------------------------------------------------------------------------------------
// The game's one image format and the two records that wrap it.
//
// From reversing/notes/about_assets.md, checked against every image in the CD build:
//   - kind 1 is the image: u16 data_size, u16 zero, u16 width, u16 height, then the pixels
//   - kind 2 wraps a kind 1 at +5 behind an i16 anchor x, y; the game draws it at (x - ax, y - ay)
//   - kind 4 wraps a kind 2 at +6 behind a far pointer to the next frame and a u8 walk advance
//   - each row is its own PackBits stream and ends exactly on the width
//   - colour 0 is never written: the absence of a write is the transparency, so the decoder
//     keeps a written mask beside the pixels
// The leading byte only nominates a layout. A payload is an image once its header and pixel
// stream account for its length exactly, which is what read() insists on.

use crate::error::{FormatError, Result};
use crate::file_format::{FileFormat, SourceFileType};

/// Which record the payload opened with. The byte is what the game's code calls the kind.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ImageType {
    /// A bare image, byte 1.
    Plain,
    /// Byte 2: an image with an anchor, mostly buildings and panels.
    Placed,
    /// Byte 4: one frame of a unit animation, wrapping a Placed record.
    Frame,
}

impl ImageType {
    /// The leading byte as it appears in the file.
    pub fn type_byte(self) -> u8 {
        match self {
            ImageType::Plain => 1,
            ImageType::Placed => 2,
            ImageType::Frame => 4,
        }
    }

    fn from_byte(byte: u8) -> Option<ImageType> {
        match byte {
            1 => Some(ImageType::Plain),
            2 => Some(ImageType::Placed),
            4 => Some(ImageType::Frame),
            _ => None,
        }
    }

    /// Byte offset of the plain header, and of the anchor if this record has one.
    fn header_layout(self) -> (usize, Option<usize>) {
        match self {
            ImageType::Plain => (1, None),
            ImageType::Placed => (6, Some(1)),
            ImageType::Frame => (12, Some(7)),
        }
    }
}

/// Everything an image says about itself before the pixels.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ImageHeader {
    pub image_type: ImageType,
    pub width: u16,
    pub height: u16,
    /// Where the game puts the image relative to the point it draws at; Plain has none.
    pub anchor_x: i16,
    pub anchor_y: i16,
    /// Frame only: how far the unit moves on this frame. Zero otherwise.
    pub advance: u8,
    /// Bytes of pixel stream the header declares.
    pub data_size: usize,
    /// Where the pixel stream starts in the payload.
    pub pixels_at: usize,
}

impl ImageHeader {
    /// Read the header and check it against the payload length. Does not touch the pixels.
    pub fn read(bytes: &[u8]) -> Result<ImageHeader> {
        let byte = *bytes.first().ok_or_else(|| FormatError::malformed("empty payload"))?;
        let image_type = ImageType::from_byte(byte).ok_or_else(|| FormatError::malformed(format!("leading byte {byte} is not an image type")))?;
        let (prefix, anchor_at) = image_type.header_layout();
        let pixels_at = prefix + 8;
        if bytes.len() < pixels_at {
            return Err(FormatError::malformed(format!("payload of {} bytes is shorter than a {pixels_at}-byte header", bytes.len())));
        }
        let word = |at: usize| u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let (data_size, zero, width, height) = (word(prefix) as usize, word(prefix + 2), word(prefix + 4), word(prefix + 6));
        if pixels_at + data_size != bytes.len() {
            return Err(FormatError::malformed(format!("declared data size {data_size} plus header {pixels_at} is not the payload size {}", bytes.len())));
        }
        if zero != 0 {
            return Err(FormatError::malformed(format!("the always-zero header word is {zero}")));
        }
        if width == 0 || height == 0 {
            return Err(FormatError::malformed(format!("degenerate size {width}x{height}")));
        }
        let (anchor_x, anchor_y) = match anchor_at {
            Some(at) => (word(at) as i16, word(at + 2) as i16),
            None => (0, 0),
        };
        let advance = if image_type == ImageType::Frame { bytes[5] } else { 0 };
        Ok(ImageHeader { image_type, width, height, anchor_x, anchor_y, advance, data_size, pixels_at })
    }
}

/// One decoded image as the game holds it: palette indices plus which pixels were written at all.
/// Not a picture yet; media::Image::from_indexed turns it into one under a palette.
pub struct IndexedImage {
    pub header: ImageHeader,
    /// Row major palette indices; unwritten pixels hold 0.
    pub pixels: Vec<u8>,
    /// Row major; true where a pixel was written.
    pub mask: Vec<bool>,
}

impl std::fmt::Debug for IndexedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IndexedImage {:?}", self.header)
    }
}

impl IndexedImage {
    pub fn width(&self) -> usize {
        self.header.width as usize
    }

    pub fn height(&self) -> usize {
        self.header.height as usize
    }
}

impl FileFormat for IndexedImage {
    const DESCRIPTION: &'static str = "an image";

    fn read(bytes: &[u8]) -> Result<IndexedImage> {
        let header = ImageHeader::read(bytes)?;
        let npix = header.width as usize * header.height as usize;
        let (pixels, mask, used) = decode_rle(&bytes[header.pixels_at..], npix)?;
        if used != header.data_size {
            return Err(FormatError::malformed(format!("decode consumed {used} bytes, the header declared {}", header.data_size)));
        }
        Ok(IndexedImage { header, pixels, mask })
    }

    fn source_type(&self) -> SourceFileType {
        SourceFileType::Image(self.header)
    }
}

/// Expand the PackBits variant into exactly npix pixels. Returns the pixels, the written
/// mask and how many input bytes were consumed.
///   - 0x80 is filler and is skipped
///   - below 0x80 is a literal run of b+1 bytes
///   - above 0x80 is a run of 0x101-b copies of the next byte
///   - colour 0 is never written; the write position moves on and the mask stays false
///
/// Note: the rows abut in every shipped image, so one stream over the whole image gives the
/// same pixels as the game's row-at-a-time loop. An encoder must not rely on that.
pub fn decode_rle(src: &[u8], npix: usize) -> Result<(Vec<u8>, Vec<bool>, usize)> {
    let mut out = vec![0u8; npix];
    let mut mask = vec![false; npix];
    let mut at = 0usize;
    let mut pos = 0usize;
    while pos < npix {
        let b = *src.get(at).ok_or_else(|| FormatError::malformed(format!("ran out of input at pixel {pos} of {npix}")))?;
        at += 1;
        if b == 0x80 {
            continue;
        }
        if b < 0x80 {
            let count = b as usize + 1;
            if at + count > src.len() {
                return Err(FormatError::malformed(format!("literal run of {count} overruns the input")));
            }
            if pos + count > npix {
                return Err(FormatError::malformed(format!("literal run overruns the image at pixel {pos}")));
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
            let value = *src.get(at).ok_or_else(|| FormatError::malformed("run length with no value byte"))?;
            at += 1;
            if pos + count > npix {
                return Err(FormatError::malformed(format!("run of {count} overruns the image at pixel {pos}")));
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
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain image, 3 wide by 2 tall: row one is a literal 1,0,2, row two a run of three 5s.
    pub fn tiny_plain() -> Vec<u8> {
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
    fn a_plain_image_decodes_and_zero_is_unwritten() {
        let image = IndexedImage::read(&tiny_plain()).expect("decodes");
        assert_eq!((image.width(), image.height(), image.header.image_type), (3, 2, ImageType::Plain));
        assert_eq!(image.pixels, vec![1, 0, 2, 5, 5, 5]);
        assert_eq!(image.mask, vec![true, false, true, true, true, true]);
        assert_eq!(image.source_type(), SourceFileType::Image(image.header));
    }

    #[test]
    fn placed_and_frame_records_wrap_the_image_with_anchors() {
        let inner = tiny_plain();
        let mut placed = vec![2u8];
        placed.extend_from_slice(&(-7i16).to_le_bytes());
        placed.extend_from_slice(&(12i16).to_le_bytes());
        placed.extend_from_slice(&inner);
        let h = IndexedImage::read(&placed).expect("decodes").header;
        assert_eq!((h.image_type, h.anchor_x, h.anchor_y, h.advance), (ImageType::Placed, -7, 12, 0));
        let mut frame = vec![4u8, 0, 0, 0, 0, 3];
        frame.extend_from_slice(&placed);
        let h = IndexedImage::read(&frame).expect("decodes").header;
        assert_eq!((h.image_type, h.anchor_x, h.anchor_y, h.width, h.advance), (ImageType::Frame, -7, 12, 3, 3));
    }

    #[test]
    fn bad_payloads_are_refused() {
        let mut short = tiny_plain();
        short.pop();
        assert!(IndexedImage::read(&short).is_err());
        let mut wrong_type = tiny_plain();
        wrong_type[0] = 3;
        assert!(IndexedImage::read(&wrong_type).is_err());
        let mut bad_stream = tiny_plain();
        bad_stream[9] = 0x05;
        assert!(ImageHeader::read(&bad_stream).is_ok(), "the header alone still adds up");
        assert!(IndexedImage::read(&bad_stream).is_err(), "the pixels do not");
        assert!(decode_rle(&[0x00], 3).is_err());
        let (pixels, _, used) = decode_rle(&[0x80, 0x80, 0xff, 9], 2).expect("filler is skipped");
        assert_eq!((pixels, used), (vec![9, 9], 4));
    }
}
