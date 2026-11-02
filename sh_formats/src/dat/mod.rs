// --------------------------------------------------------------------------------------------------------------------
// STRONG.DAT, the archive the payloads live in. Scaffolding only: module declarations and re-exports.

pub mod dat_file;
pub mod name_table;
pub mod palette_rules;

pub use dat_file::{DatFile, Entry};
pub use name_table::{Confidence, NameLookup, NameTable, SourceFile, EntryOrigin};
pub use palette_rules::{KnownPaletteRules, PaletteChoice, PaletteResolver, PaletteRule, PieceStamp, MAIN_PALETTE_ENTRY};
