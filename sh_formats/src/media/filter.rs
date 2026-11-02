// --------------------------------------------------------------------------------------------------------------------
// Reprocessing: something done to a picture or a sound before it is used, giving a new one.
//
// A filter is a value so it can carry its settings, be kept in a list and be named in a
// menu. Filters never touch the original.

use crate::media::audio::AudioClip;
use crate::media::image::{Image, Rgba};
use crate::media::xbr;

/// Turns one picture into another.
pub trait ImageFilter {
    /// For a menu or a filename: "xbr2x".
    fn name(&self) -> &'static str;

    fn apply(&self, image: &Image) -> Image;
}

/// Turns one sound into another.
pub trait AudioFilter {
    fn name(&self) -> &'static str;

    fn apply(&self, clip: &AudioClip) -> AudioClip;
}

/// Each source pixel becomes a factor by factor block. Pixels stay pixels.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct NearestScale {
    pub factor: usize,
}

impl ImageFilter for NearestScale {
    fn name(&self) -> &'static str {
        "nearest"
    }

    fn apply(&self, image: &Image) -> Image {
        let factor = self.factor.max(1);
        let (w, h) = (image.width() * factor, image.height() * factor);
        let mut pixels: Vec<Rgba> = Vec::with_capacity(w * h);
        for y in 0..h {
            for x in 0..w {
                pixels.push(image.pixel(x / factor, y / factor).unwrap_or([0, 0, 0, 0]));
            }
        }
        Image::from_pixels(w, h, pixels).unwrap_or_else(|_| Image::blank(w, h))
    }
}

/// xBR at 2x: edges are found and smoothed into the doubled picture. See media::xbr.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Xbr2x {
    /// Blend along edges (the usual look) rather than picking source colours only.
    pub blend_colours: bool,
    /// Let the filter work across a sprite's transparent surround. On for sprites.
    pub scale_alpha: bool,
}

impl Default for Xbr2x {
    /// How the asset browser calls it.
    fn default() -> Self {
        Xbr2x { blend_colours: true, scale_alpha: true }
    }
}

impl ImageFilter for Xbr2x {
    fn name(&self) -> &'static str {
        "xbr2x"
    }

    fn apply(&self, image: &Image) -> Image {
        let src: Vec<u32> = image.pixels().iter().map(|p| u32::from_le_bytes(*p)).collect();
        let out = xbr::xbr2x(&src, image.width(), image.height(), self.blend_colours, self.scale_alpha);
        let pixels = out.iter().map(|p| p.to_le_bytes()).collect();
        let (w, h) = (image.width() * 2, image.height() * 2);
        Image::from_pixels(w, h, pixels).unwrap_or_else(|_| Image::blank(w, h))
    }
}

/// Change the sample rate by linear interpolation. The game's rates are DOS timer divisors
/// (9009, 11111, 21739 Hz); a modern mixer wants one it knows.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Resample {
    pub sample_rate: u32,
}

impl AudioFilter for Resample {
    fn name(&self) -> &'static str {
        "resample"
    }

    fn apply(&self, clip: &AudioClip) -> AudioClip {
        let src = clip.samples();
        if src.is_empty() || clip.sample_rate() == 0 || self.sample_rate == 0 {
            return AudioClip::new(self.sample_rate, Vec::new());
        }
        let step = clip.sample_rate() as f64 / self.sample_rate as f64;
        let count = (src.len() as f64 / step).round().max(1.0) as usize;
        let samples = (0..count).map(|n| {
            let at = n as f64 * step;
            let i = (at as usize).min(src.len() - 1);
            let frac = at - i as f64;
            let next = src[(i + 1).min(src.len() - 1)] as f64;
            (src[i] as f64 * (1.0 - frac) + next * frac).round() as u8
        }).collect();
        AudioClip::new(self.sample_rate, samples)
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
    fn nearest_scale_makes_blocks() {
        let image = Image::from_pixels(2, 1, vec![[1, 1, 1, 255], [2, 2, 2, 255]]).expect("fits");
        let big = NearestScale { factor: 2 }.apply(&image);
        assert_eq!((big.width(), big.height()), (4, 2));
        assert_eq!(big.pixel(1, 1), Some([1, 1, 1, 255]));
        assert_eq!(big.pixel(2, 0), Some([2, 2, 2, 255]));
        assert_eq!(NearestScale { factor: 0 }.apply(&image), image, "a factor below one is one");
    }

    #[test]
    fn xbr_doubles_and_keeps_the_pixel_layout() {
        let image = Image::from_pixels(2, 2, vec![[255, 0, 0, 255], [0, 0, 0, 0], [0, 0, 0, 0], [0, 0, 255, 255]]).expect("fits");
        let big = Xbr2x::default().apply(&image);
        assert_eq!((big.width(), big.height()), (4, 4));
        assert_eq!(big.pixel(0, 0), Some([255, 0, 0, 255]), "a corner keeps its colour");
        assert_eq!(Xbr2x::default().name(), "xbr2x");
    }

    #[test]
    fn resampling_keeps_the_duration() {
        let clip = AudioClip::new(8000, (0..8000).map(|n| (n % 256) as u8).collect());
        let out = Resample { sample_rate: 44100 }.apply(&clip);
        assert_eq!(out.sample_rate(), 44100);
        assert_eq!(out.len(), 44100);
        assert!((out.duration_seconds() - 1.0).abs() < 1e-9);
        assert_eq!(out.samples()[0], 0);
        assert!(Resample { sample_rate: 22050 }.apply(&AudioClip::new(8000, vec![])).is_empty());
    }
}
