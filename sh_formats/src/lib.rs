// --------------------------------------------------------------------------------------------------------------------
// sh_formats: readers for the game's own file formats. Scaffolding only: module declarations and re-exports.
//
// file_format.rs is the generic interface, file_type_detector.rs says which format a payload is,
// formats/ holds one handler per payload format, dat/ is the archive those payloads live in, and
// media/ is what a modern consumer holds once they are out: pictures, sprites, sounds, and
// grab/ is the packer's side, STRONG.DAT back into the source files it was built from.

pub mod dat;
pub mod error;
pub mod file_format;
pub mod file_type_detector;
pub mod formats;
pub mod grab;
pub mod media;
pub mod prelude;
