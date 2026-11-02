// --------------------------------------------------------------------------------------------------------------------
// What a modern consumer holds: pictures, sprites, sounds, and what can be done to them.
// Nothing here knows about STRONG.DAT; dat::DatFile builds these out of what it reads.
// Scaffolding only: module declarations and re-exports.

pub mod audio;
pub mod export;
pub mod filter;
pub mod image;
pub mod music;
pub mod sprite;
pub mod xbr;

pub use audio::{AudioClip, Wav};
pub use export::{export_to_file, Exporter};
pub use filter::{AudioFilter, ImageFilter, NearestScale, Resample, Xbr2x};
pub use image::{Image, Png, Rgba};
pub use music::{Message, Midi, Score, TimedMessage, TICKS_PER_QUARTER};
pub use sprite::{Animation, BuildingState, Frame, Sprite, SpriteSet, SpriteState, UnitState};
