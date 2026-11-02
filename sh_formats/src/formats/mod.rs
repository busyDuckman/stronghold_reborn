// --------------------------------------------------------------------------------------------------------------------
// One handler per payload format. Scaffolding only: module declarations and re-exports.

pub mod audio;
pub mod image;
pub mod music;
pub mod palette;

pub use audio::{AudioHeader, AudioType};
pub use image::{decode_rle, ImageHeader, ImageType, IndexedImage};
pub use music::{Timbre, XmiEvent, XmiScore, XmiSequence, XMI_TICKS_PER_SECOND};
pub use palette::{widen_vga_channel, Palette, PalettePiece};
