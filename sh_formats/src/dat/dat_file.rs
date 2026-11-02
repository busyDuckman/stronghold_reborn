// --------------------------------------------------------------------------------------------------------------------
// STRONG.DAT: the entry table, every entry's type, the names for them and the palette each
// image is drawn with. One object, opened once.
//
// From about_assets.md and about_loader.md (which call it the index table): four bytes at
// offset 0 give the offset of the entry table, which sits at the very end; rows are 21 bytes
// of which only the payload offset and size mean anything on disk, the other nine being the
// loader's runtime scratch. Payloads run from offset 4 to the table with no padding, and that
// contiguity is asserted on open because a wrong row stride cannot survive it.
//
// The archive records no type and no name for anything. Types come from the detector over
// every payload at open, names from a .tsv beside the file, and the palette rules from both.

use crate::dat::name_table::{NameLookup, NameTable, EntryOrigin};
use crate::dat::palette_rules::{KnownPaletteRules, PaletteChoice, PaletteResolver};
use crate::error::{FormatError, Result};
use crate::file_format::{FileFormat, SourceFileType};
use crate::file_type_detector::detect_file_type;
use crate::formats::audio::AudioHeader;
use crate::formats::image::{ImageType, IndexedImage};
use crate::formats::music::XmiScore;
use crate::formats::palette::{Palette, PalettePiece};
use crate::media::audio::AudioClip;
use crate::media::image::Image;
use crate::media::music::Score;
use crate::media::sprite::{Animation, Frame, Sprite};
use std::path::{Path, PathBuf};

/// One row of the entry table.
pub const ENTRY_ROW_LEN: usize = 21;
/// The most rows the game's loader allocates for; the CD build has 3451.
pub const MAX_ENTRIES: usize = 3475;
/// open() looks for the names file beside the .dat under the same stem with this extension.
pub const NAMES_EXTENSION: &str = "tsv";

/// Where one payload sits in the file.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub index: usize,
    pub offset: usize,
    pub size: usize,
}

/// The archive, resident, with its entry table checked and every entry classified.
pub struct DatFile {
    bytes: Vec<u8>,
    entries: Vec<Entry>,
    types: Vec<SourceFileType>,
    names: NameTable,
    palettes: KnownPaletteRules,
    path: PathBuf,
}

impl std::fmt::Debug for DatFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DatFile {} entries, {} bytes, {} names, from {}", self.entries.len(), self.bytes.len(), self.names.len(), self.path.display())
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Opening

impl DatFile {
    /// Open the archive and the names file beside it (STRONG.DAT next to STRONG.tsv). A
    /// missing names file is logged and tolerated, because the port ships without one; every
    /// lookup by name then misses and every entry is still reachable by index.
    pub fn open(path: &Path) -> Result<DatFile> {
        let names_path = DatFile::default_names_path(path);
        let names = if names_path.is_file() {
            NameTable::load(&names_path)?
        } else {
            log::warn!("no names file at {}; entries are reachable by index only", names_path.display());
            NameTable::empty()
        };
        DatFile::open_with_names(path, names)
    }

    /// Open the archive with names from wherever the caller got them.
    pub fn open_with_names(path: &Path, names: NameTable) -> Result<DatFile> {
        let bytes = std::fs::read(path).map_err(|source| FormatError::Io { path: path.to_path_buf(), source })?;
        DatFile::from_bytes(bytes, path.to_path_buf(), names)
    }

    /// Open an archive that is already in memory, refusing one whose entry table does not add
    /// up. The path only names the archive in messages.
    pub fn from_bytes(bytes: Vec<u8>, path: PathBuf, names: NameTable) -> Result<DatFile> {
        let entries = DatFile::parse_entry_table(&bytes).map_err(|e| FormatError::Archive { path: path.clone(), reason: e.to_string() })?;
        let types: Vec<SourceFileType> = entries.iter().map(|e| detect_file_type(&bytes[e.offset..e.offset + e.size])).collect();
        let palettes = KnownPaletteRules::for_cd_build(&types, &names)?;
        log::info!("{}: {} entries, {} images, {} palettes, {} names", path.display(), entries.len(), types.iter().filter(|t| t.is_image()).count(), types.iter().filter(|t| t.is_palette()).count(), names.len());
        Ok(DatFile { bytes, entries, types, names, palettes, path })
    }

    /// The table of entries at the end of the file.
    /// NOTE: Err if not 4 byte entry count followed by 21-bytes per entry.
    fn parse_entry_table(bytes: &[u8]) -> Result<Vec<Entry>> {
        let fail = FormatError::malformed;
        if bytes.len() < 4 {
            return Err(fail("too short to be an archive".into()));
        }
        let table = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        if table < 4 || table > bytes.len() || !(bytes.len() - table).is_multiple_of(ENTRY_ROW_LEN) {
            return Err(fail(format!("entry table offset {table} does not divide the file into {ENTRY_ROW_LEN}-byte rows")));
        }
        let count = (bytes.len() - table) / ENTRY_ROW_LEN;
        if count > MAX_ENTRIES {
            return Err(fail(format!("{count} rows is more than the {MAX_ENTRIES} the game's loader allows")));
        }
        let mut entries = Vec::with_capacity(count);
        let mut expected = 4usize;
        for (index, row) in bytes[table..].chunks(ENTRY_ROW_LEN).enumerate() {
            let offset = u32::from_le_bytes([row[1], row[2], row[3], row[4]]) as usize;
            let size = u32::from_le_bytes([row[5], row[6], row[7], row[8]]) as usize;
            // Payloads are contiguous from offset 4 to the table; a wrong stride cannot pass this.
            if offset != expected || offset + size > table {
                return Err(fail(format!("entry {index} at {offset} (+{size}) breaks the contiguous payload run")));
            }
            expected = offset + size;
            entries.push(Entry { index, offset, size });
        }
        if expected != table {
            return Err(fail(format!("the last payload ends at {expected}, the table starts at {table}")));
        }
        Ok(entries)
    }

    /// Where open() looks for the names file: STRONG.DAT gives STRONG.tsv beside it.
    pub fn default_names_path(dat_path: &Path) -> PathBuf {
        dat_path.with_extension(NAMES_EXTENSION)
    }

    /// Write payloads into a new archive image: header, payloads back to back, entry table.
    /// The inverse of parse_entry_table; the tests use it and a writer would.
    pub fn pack(payloads: &[&[u8]]) -> Vec<u8> {
        let mut bytes = vec![0u8; 4];
        let mut rows = Vec::with_capacity(payloads.len() * ENTRY_ROW_LEN);
        for payload in payloads {
            let mut row = [0u8; ENTRY_ROW_LEN];
            row[1..5].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
            row[5..9].copy_from_slice(&(payload.len() as u32).to_le_bytes());
            rows.extend_from_slice(&row);
            bytes.extend_from_slice(payload);
        }
        let table = bytes.len() as u32;
        bytes[..4].copy_from_slice(&table.to_le_bytes());
        bytes.extend_from_slice(&rows);
        bytes
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Entries, by index and by name

impl DatFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Where one entry's payload sits in the file.
    pub fn entry(&self, index: usize) -> Result<Entry> {
        self.entries.get(index).copied().ok_or(FormatError::NoSuchEntry { index, count: self.entries.len() })
    }

    /// Every entry in file order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// One payload's bytes, as stored.
    pub fn payload_bytes(&self, index: usize) -> Result<&[u8]> {
        let entry = self.entry(index)?;
        Ok(&self.bytes[entry.offset..entry.offset + entry.size])
    }

    /// What one entry is, as the detector named it at open.
    pub fn source_type(&self, index: usize) -> Result<&SourceFileType> {
        self.types.get(index).ok_or(FormatError::NoSuchEntry { index, count: self.types.len() })
    }

    /// Every entry's type, in file order.
    pub fn source_types(&self) -> &[SourceFileType] {
        &self.types
    }

    pub fn names(&self) -> &NameTable {
        &self.names
    }

    /// Which art file an entry was cut from, and which cut it was, if the names file knows.
    pub fn origin_of(&self, index: usize) -> Option<EntryOrigin<'_>> {
        self.names.origin_of(index)
    }

    /// Every entry cut from the named art file, in order. A name that occurs twice gives both spans.
    pub fn entries_named(&self, name: &str) -> Result<Vec<usize>> {
        let found = self.names.source_files_named(name);
        if found.is_empty() {
            return Err(FormatError::UnknownName(name.to_string()));
        }
        Ok(found.iter().flat_map(|s| s.first_entry..=s.last_entry).collect())
    }

    /// The archive index of the nth sprite (from 0) cut from the named art file.
    pub fn entry_named(&self, name: &str, nth: usize) -> Result<usize> {
        self.names.source_file_named(name)?.entry(nth)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Decoding an entry as a format

impl DatFile {
    /// Decode an entry as format F, or say which entry is not one and why.
    pub fn read_as<F: FileFormat>(&self, index: usize) -> Result<F> {
        F::read(self.payload_bytes(index)?).map_err(|e| FormatError::WrongType { index, wanted: F::DESCRIPTION, reason: e.to_string() })
    }

    /// Decode an image entry with its written mask.
    pub fn read_image(&self, index: usize) -> Result<IndexedImage> {
        self.read_as(index)
    }

    /// Decode a palette entry, widened to 8 bits per channel.
    pub fn read_palette(&self, index: usize) -> Result<Palette> {
        self.read_as(index)
    }

    /// The indices of every palette entry, in file order.
    pub fn palette_indices(&self) -> Vec<usize> {
        (0..self.types.len()).filter(|i| self.types[*i].is_palette()).collect()
    }

    /// The palette a choice describes: the base entry with any piece stamped over it.
    pub fn build_palette(&self, choice: &PaletteChoice) -> Result<Palette> {
        let base = self.read_palette(choice.base)?;
        match choice.stamp {
            None => Ok(base),
            Some(stamp) => {
                let piece: PalettePiece = self.read_as(stamp.entry)?;
                base.stamped(stamp.first_slot, &piece).map_err(|e| FormatError::WrongType { index: stamp.entry, wanted: "a palette piece", reason: e.to_string() })
            }
        }
    }

    /// The palette an image is drawn with, and the rule that chose it.
    pub fn palette_for_image(&self, index: usize) -> Result<(Palette, PaletteChoice)> {
        let choice = self.palette_for(index)?;
        Ok((self.build_palette(&choice)?, choice))
    }

    pub fn palette_rules(&self) -> &KnownPaletteRules {
        &self.palettes
    }
}

impl PaletteResolver for DatFile {
    fn palette_for(&self, index: usize) -> Result<PaletteChoice> {
        self.palettes.palette_for(index)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Entries as media: what a modern consumer takes away

impl DatFile {
    /// An image entry painted under the palette the rules choose for it.
    pub fn render_image(&self, index: usize) -> Result<Image> {
        let (palette, _) = self.palette_for_image(index)?;
        Ok(Image::from_indexed(&self.read_image(index)?, &palette))
    }

    /// An image entry as a sprite: painted, with the anchor its record carries. A plain
    /// image has no anchor and is cornered.
    pub fn sprite(&self, index: usize) -> Result<Sprite> {
        let indexed = self.read_image(index)?;
        let (palette, _) = self.palette_for_image(index)?;
        let image = Image::from_indexed(&indexed, &palette);
        let h = indexed.header;
        Ok(Sprite { image, anchor_x: h.anchor_x as i32, anchor_y: h.anchor_y as i32 })
    }

    /// A run of consecutive entries as one animation, each frame carrying its advance.
    /// Frames are only Frame records; a Placed or plain image in the run is refused, since
    /// a building view is not a frame of anything.
    pub fn animation(&self, first: usize, count: usize) -> Result<Animation> {
        let mut frames = Vec::with_capacity(count);
        for index in first..first + count {
            let header = *self.source_type(index)?.image_header().ok_or_else(|| FormatError::WrongType { index, wanted: "a unit frame", reason: "not an image".into() })?;
            if header.image_type != ImageType::Frame {
                return Err(FormatError::WrongType { index, wanted: "a unit frame", reason: format!("a {:?} image", header.image_type) });
            }
            frames.push(Frame { sprite: self.sprite(index)?, advance: header.advance as u32 });
        }
        Ok(Animation::new(frames))
    }

    /// A sampled sound entry as a clip.
    pub fn audio_clip(&self, index: usize) -> Result<AudioClip> {
        let payload = self.payload_bytes(index)?;
        let header: AudioHeader = self.read_as(index)?;
        AudioClip::from_dat(&header, payload).map_err(|e| FormatError::WrongType { index, wanted: "a sampled sound", reason: e.to_string() })
    }

    /// A music entry as a score, timed so it plays for as long as the game played it. An XMI
    /// is a catalogue, and no entry in this archive holds a second song, so the first is it.
    pub fn score(&self, index: usize) -> Result<Score> {
        let xmi: XmiScore = self.read_as(index)?;
        if xmi.sequences.len() > 1 {
            log::warn!("entry {index} holds {} sequences; scoring the first", xmi.sequences.len());
        }
        let first = xmi.sequences.first().ok_or_else(|| FormatError::WrongType { index, wanted: "music", reason: "the catalogue is empty".into() })?;
        Score::from_xmi(first)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests. The real archive is not in the repo; the tests that need it skip when it is absent.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dat::palette_rules::PaletteRule;
    use crate::formats::audio::AudioType;
    use crate::formats::image::ImageType;
    use crate::media::export::Exporter;
    use crate::media::music::{Message, Midi};

    /// Where the CD build's archive sits under the repo, relative to the repo root.
    const STRONG_DAT_RELATIVE: &str = "achieve/archive.org/en_cdrom/unpacked/STRONG.DAT";

    /// The real archive if this machine has it, looking from the crate up to the repo root.
    fn real_archive() -> Option<PathBuf> {
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

    /// A plain image, 3 wide by 2 tall.
    fn tiny_image() -> Vec<u8> {
        let stream = [0x02u8, 1, 0, 2, 0xfe, 5];
        let mut payload = vec![1u8];
        payload.extend_from_slice(&(stream.len() as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&3u16.to_le_bytes());
        payload.extend_from_slice(&2u16.to_le_bytes());
        payload.extend_from_slice(&stream);
        payload
    }

    fn open_synthetic(payloads: &[&[u8]], names: &str) -> Result<DatFile> {
        DatFile::from_bytes(DatFile::pack(payloads), PathBuf::from("synthetic"), NameTable::parse(names).expect("names"))
    }

    /// Shaped like the CD build's palette neighbourhood: the default palette has to sit at
    /// entry 741, so everything before it is filler.
    fn synthetic() -> DatFile {
        let image = tiny_image();
        let palette = vec![0x3fu8; 768];
        let piece = [0x10u8; 144];
        let filler = [0xffu8; 1];
        let mut payloads: Vec<&[u8]> = vec![&filler; 741];
        payloads.push(&palette);
        payloads.extend([&image[..], &image[..], &piece[..], &image[..], &image[..], &palette[..]]);
        open_synthetic(&payloads, "frame.lbm\t741\t742\n3438\tstrong.fli\n").expect("opens")
    }

    #[test]
    fn a_packed_archive_lists_and_hands_back_its_payloads() {
        let bytes = DatFile::pack(&[b"abc", b"", b"defgh"]);
        let palette = vec![0x3fu8; 768];
        let mut payloads: Vec<&[u8]> = vec![b"abc", b"", b"defgh"];
        payloads.resize(741, b"x");
        payloads.push(&palette);
        let dat = open_synthetic(&payloads, "").expect("indexes");
        assert_eq!(dat.entry_count(), 742);
        assert_eq!(dat.entry(1).expect("entry"), Entry { index: 1, offset: 7, size: 0 });
        assert_eq!(dat.payload_bytes(2).expect("payload"), b"defgh");
        assert!(matches!(dat.entry(742), Err(FormatError::NoSuchEntry { index: 742, count: 742 })));
        assert_eq!(dat.entries().len(), 742);
        assert!(DatFile::from_bytes(bytes, PathBuf::from("no default palette"), NameTable::empty()).is_err(), "741 must be a palette");
    }

    #[test]
    fn a_broken_entry_table_is_refused() {
        let mut bytes = DatFile::pack(&[b"abc", b"de"]);
        let table = bytes.len() - 2 * ENTRY_ROW_LEN;
        bytes[table + 1] = 5;
        assert!(DatFile::parse_entry_table(&bytes).is_err(), "first payload not at 4");
        let mut bytes = DatFile::pack(&[b"abc", b"de"]);
        bytes.push(0);
        assert!(DatFile::parse_entry_table(&bytes).is_err(), "rows no longer 21 bytes");
        assert!(DatFile::parse_entry_table(&[1, 2]).is_err());
    }

    #[test]
    fn entries_are_typed_named_and_given_palettes() {
        let dat = synthetic();
        assert_eq!(dat.entry_count(), 748);
        assert_eq!(dat.source_type(0).expect("type").label(), "bin");
        assert!(dat.source_type(741).expect("type").is_palette());
        assert_eq!(dat.palette_indices(), vec![741, 747]);
        assert_eq!(dat.entries_named("frame.lbm").expect("run"), vec![741, 742]);
        assert_eq!(dat.entry_named("FRAME.LBM", 1).expect("nth"), 742);
        assert!(dat.entry_named("frame.lbm", 2).is_err());
        assert!(dat.entries_named("nope.lbm").is_err());
        assert_eq!(dat.origin_of(742).expect("named").nth, 1);
        let (palette, choice) = dat.palette_for_image(742).expect("adjacent");
        assert_eq!((choice.base, choice.rule), (741, PaletteRule::Adjacent));
        assert_eq!(palette.colours[0], [255, 255, 255]);
        let choice = dat.palette_for(743).expect("backdrop first half");
        assert_eq!((choice.rule, choice.stamp.map(|s| s.entry)), (PaletteRule::Backdrop, Some(744)));
        let stamped = dat.build_palette(&choice).expect("stamped");
        assert_eq!((stamped.colours[159], stamped.colours[160]), ([255; 3], [65; 3]));
        assert_eq!(dat.palette_for(745).expect("second half").stamp.map(|s| s.entry), Some(744));
        assert_eq!(dat.palette_for(746).expect("a palette after a non-portrait is not evidence").rule, PaletteRule::Default);
        assert!(dat.read_image(741).is_err(), "a palette is not an image");
        assert!(dat.read_palette(742).is_err(), "an image is not a palette");
        assert_eq!(dat.read_image(742).expect("image").pixels, vec![1, 0, 2, 5, 5, 5]);
        let image = dat.render_image(742).expect("renders");
        assert_eq!(image.pixel(0, 0), Some([255, 255, 255, 255]));
        assert_eq!(image.pixel(1, 0), Some([0, 0, 0, 0]), "unwritten is transparent");
        assert_eq!(dat.sprite(742).expect("sprite").anchor_x, 0, "a plain image is cornered");
        assert!(dat.animation(742, 1).is_err(), "a plain image is not a frame");
        assert!(dat.audio_clip(742).is_err());
    }

    #[test]
    fn the_real_archive_matches_the_notes() {
        let Some(path) = real_archive() else {
            eprintln!("STRONG.DAT not on this machine; skipping");
            return;
        };
        let dat = DatFile::open_with_names(&path, NameTable::empty()).expect("opens");
        assert_eq!(dat.entry_count(), 3451);
        let count = |f: &dyn Fn(&SourceFileType) -> bool| dat.source_types().iter().filter(|t| f(t)).count();
        assert_eq!(count(&|t| t.is_image()), 3192);
        assert_eq!(count(&|t| t.is_palette()), 56);
        assert_eq!(count(&|t| *t == SourceFileType::PalettePiece), 9);
        assert_eq!(count(&|t| *t == SourceFileType::Fli), 2);
        assert_eq!(count(&|t| *t == SourceFileType::Text), 34);
        assert_eq!(count(&|t| matches!(t, SourceFileType::Audio(_))), 138);
        assert_eq!(dat.palette_for(3200).expect("portrait").rule, PaletteRule::Adjacent);
        assert_eq!(dat.palette_for(3200).expect("portrait").base, 3201);
        assert_eq!(dat.palette_for(944).expect("backdrop").stamp.map(|s| s.entry), Some(945));
        assert_eq!(dat.palette_for(970).expect("last backdrop").stamp.map(|s| s.entry), Some(969));
        assert_eq!(dat.palette_for(903).expect("dialog").base, 906);
        assert_eq!(dat.palette_for(300).expect("a tree").rule, PaletteRule::Default);
        assert_eq!(dat.read_image(300).expect("tree").header.image_type, ImageType::Placed);
        assert_eq!(dat.read_image(1170).expect("spearman").header.image_type, ImageType::Frame);
        let walk = dat.animation(1170, 30).expect("the spearman's 30 frames");
        assert_eq!(walk.len(), 30);
        assert!(walk.advance_per_loop() > 0, "a walk covers ground");
        let strip = walk.strip();
        assert!(strip.width() >= 30 * 11 && strip.height() >= 20);
        let clip = dat.audio_clip(1025).expect("a sampled sound");
        assert!(clip.sample_rate() > 0 && !clip.is_empty());
    }

    #[test]
    fn every_score_in_the_real_archive_converts() {
        let Some(path) = real_archive() else {
            eprintln!("STRONG.DAT not on this machine; skipping");
            return;
        };
        let dat = DatFile::open_with_names(&path, NameTable::empty()).expect("opens");
        let music: Vec<usize> = dat.source_types().iter().enumerate()
            .filter(|(_, t)| matches!(t, SourceFileType::Audio(h) if h.audio_type == AudioType::Xmi))
            .map(|(index, _)| index)
            .collect();
        assert_eq!(music.len(), 43, "about_assets.md counts 43 XMI entries");
        let (mut ticks, mut ons, mut offs) = (0u32, 0usize, 0usize);
        let mut longest = (0usize, 0.0f64);
        for index in &music {
            let xmi: XmiScore = dat.read_as(*index).expect("reads as XMI");
            assert_eq!(xmi.sequences.len(), 1, "entry {index} holds one song");
            let sequence = &xmi.sequences[0];
            ticks += sequence.tick_count();
            let score = dat.score(*index).expect("converts");
            ons += score.messages().iter().filter(|m| matches!(m.message, Message::NoteOn { .. })).count();
            offs += score.messages().iter().filter(|m| matches!(m.message, Message::NoteOff { .. })).count();
            // Whatever the tempo event says, the score plays for as long as the game played it.
            assert!((score.duration_seconds() - sequence.duration_seconds()).abs() < 0.01, "entry {index} changed length");
            if score.duration_seconds() > longest.1 {
                longest = (*index, score.duration_seconds());
            }
            let bytes = Midi.to_bytes(&score).expect("exports");
            assert_eq!(&bytes[..4], b"MThd", "entry {index} writes a MIDI file");
            assert_eq!(&bytes[bytes.len() - 3..], &[0xff, 0x2f, 0x00]);
        }
        assert_eq!(ticks, 50656, "422 seconds of music, at 120 intervals a second");
        assert_eq!((ons, offs), (4592, 4592), "every note that starts is stopped");
        assert_eq!(longest.0, 1017);
        assert!((longest.1 - 110.6).abs() < 0.1, "the long one is {} seconds", longest.1);
    }
}
