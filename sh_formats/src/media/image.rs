// --------------------------------------------------------------------------------------------------------------------
// A picture: an RGBA raster with nothing of the game's encoding left in it.

use crate::error::{FormatError, Result};
use crate::formats::image::IndexedImage;
use crate::formats::palette::Palette;
use crate::media::export::Exporter;
use std::io::Write;

/// One pixel, 8 bits a channel, straight (not premultiplied) alpha.
pub type Rgba = [u8; 4];

pub const TRANSPARENT: Rgba = [0, 0, 0, 0];

/// A width by height raster of RGBA pixels, row major, top row first.
#[derive(Clone, PartialEq, Eq)]
pub struct Image {
    width: usize,
    height: usize,
    pixels: Vec<Rgba>,
}

impl std::fmt::Debug for Image {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Image {}x{}", self.width, self.height)
    }
}

impl Image {
    /// A blank, fully transparent image.
    pub fn blank(width: usize, height: usize) -> Image {
        Image { width, height, pixels: vec![TRANSPARENT; width * height] }
    }

    /// From pixels already laid out row major; refused if the count does not match.
    pub fn from_pixels(width: usize, height: usize, pixels: Vec<Rgba>) -> Result<Image> {
        if pixels.len() != width * height {
            return Err(FormatError::malformed(format!("{} pixels is not {width}x{height}", pixels.len())));
        }
        Ok(Image { width, height, pixels })
    }

    /// Paint the game's palette indices with a palette. Unwritten pixels are transparent,
    /// which is the transparency the format spells by not writing them.
    pub fn from_indexed(indexed: &IndexedImage, palette: &Palette) -> Image {
        let pixels = indexed.pixels.iter().zip(&indexed.mask).map(|(index, written)| {
            if *written {
                let [r, g, b] = palette.colours[*index as usize];
                [r, g, b, 255]
            } else {
                TRANSPARENT
            }
        }).collect();
        Image { width: indexed.width(), height: indexed.height(), pixels }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn pixels(&self) -> &[Rgba] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [Rgba] {
        &mut self.pixels
    }

    /// The pixel at (x, y); None outside the image.
    pub fn pixel(&self, x: usize, y: usize) -> Option<Rgba> {
        if x < self.width && y < self.height {
            Some(self.pixels[y * self.width + x])
        } else {
            None
        }
    }

    pub fn set_pixel(&mut self, x: usize, y: usize, rgba: Rgba) {
        if x < self.width && y < self.height {
            self.pixels[y * self.width + x] = rgba;
        }
    }

    /// The pixels as one byte run, RGBA in order, which is what encoders and GPUs take.
    pub fn as_bytes(&self) -> Vec<u8> {
        self.pixels.iter().flatten().copied().collect()
    }

    /// The same picture facing the other way.
    pub fn flipped_horizontal(&self) -> Image {
        let mut pixels = Vec::with_capacity(self.pixels.len());
        for row in self.pixels.chunks(self.width.max(1)) {
            pixels.extend(row.iter().rev());
        }
        Image { width: self.width, height: self.height, pixels }
    }

    /// Paint another image onto this one at (x, y), top left corners aligned, pixels with
    /// any alpha replacing what was there. A placement partly outside is clipped.
    pub fn blit(&mut self, other: &Image, x: i64, y: i64) {
        for sy in 0..other.height {
            for sx in 0..other.width {
                let rgba = other.pixels[sy * other.width + sx];
                if rgba[3] == 0 {
                    continue;
                }
                let (dx, dy) = (x + sx as i64, y + sy as i64);
                if dx >= 0 && dy >= 0 {
                    self.set_pixel(dx as usize, dy as usize, rgba);
                }
            }
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Out as a file

/// PNG, 8 bits a channel RGBA, so transparency survives. The png crate is on the Cargo.toml menu.
pub struct Png;

impl Exporter<Image> for Png {
    const EXTENSION: &'static str = "png";

    fn write(&self, image: &Image, out: &mut dyn Write) -> Result<()> {
        let fail = |e: png::EncodingError| FormatError::Export(format!("png: {e}"));
        let mut encoder = png::Encoder::new(out, image.width() as u32, image.height() as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(fail)?;
        writer.write_image_data(&image.as_bytes()).map_err(fail)?;
        writer.finish().map_err(fail)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_format::FileFormat;

    /// A plain image, 3 wide by 2 tall: row one is a literal 1,0,2, row two a run of three 5s.
    fn tiny_indexed() -> IndexedImage {
        let stream = [0x02u8, 1, 0, 2, 0xfe, 5];
        let mut payload = vec![1u8];
        payload.extend_from_slice(&(stream.len() as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&3u16.to_le_bytes());
        payload.extend_from_slice(&2u16.to_le_bytes());
        payload.extend_from_slice(&stream);
        IndexedImage::read(&payload).expect("decodes")
    }

    #[test]
    fn indices_become_colours_and_unwritten_pixels_become_transparent() {
        let mut palette = Palette { colours: [[0; 3]; 256] };
        palette.colours[1] = [10, 20, 30];
        palette.colours[2] = [40, 50, 60];
        palette.colours[5] = [70, 80, 90];
        let image = Image::from_indexed(&tiny_indexed(), &palette);
        assert_eq!((image.width(), image.height()), (3, 2));
        assert_eq!(image.pixel(0, 0), Some([10, 20, 30, 255]));
        assert_eq!(image.pixel(1, 0), Some(TRANSPARENT));
        assert_eq!(image.pixel(2, 1), Some([70, 80, 90, 255]));
        assert_eq!(image.pixel(3, 0), None);
        assert_eq!(image.as_bytes().len(), 24);
    }

    #[test]
    fn flipping_and_blitting() {
        let image = Image::from_pixels(2, 1, vec![[1, 1, 1, 255], [2, 2, 2, 255]]).expect("fits");
        assert_eq!(image.flipped_horizontal().pixels(), &[[2, 2, 2, 255], [1, 1, 1, 255]]);
        assert!(Image::from_pixels(2, 2, vec![TRANSPARENT]).is_err());
        let mut canvas = Image::blank(3, 2);
        canvas.blit(&image, -1, 1);
        assert_eq!(canvas.pixel(0, 1), Some([2, 2, 2, 255]), "the left pixel was clipped away");
        assert_eq!(canvas.pixel(0, 0), Some(TRANSPARENT));
    }

    #[test]
    fn png_round_trips_through_the_decoder() {
        let image = Image::from_pixels(2, 1, vec![[1, 2, 3, 255], [0, 0, 0, 0]]).expect("fits");
        let bytes = Png.to_bytes(&image).expect("encodes");
        assert_eq!(&bytes[1..4], b"PNG");
        let mut reader = png::Decoder::new(&bytes[..]).read_info().expect("decodes");
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).expect("frame");
        assert_eq!((info.width, info.height, info.color_type), (2, 1, png::ColorType::Rgba));
        assert_eq!(&buf[..8], &[1, 2, 3, 255, 0, 0, 0, 0]);
        assert_eq!(<Png as Exporter<Image>>::EXTENSION, "png");
    }
}
