// --------------------------------------------------------------------------------------------------------------------
// Which palette an image is drawn with, from what has been established and nothing more.
//
// The archive ties almost no image to a palette. Each rule below cites where it came from,
// and the answer carries the rule, so a Default never gets mistaken for a fact. In order:
//   1. Adjacent: a palette stored beside the image. Portraits carry theirs after (3000:39fb
//      cuts 0,0 to 114,103 and stashes 768 bytes after it), credits before (6000:e846
//      stashes the palette, then cuts). All 44 portrait panels pair this way without exception.
//      A portrait is 115x104, which is what tells the two shapes apart when both neighbours
//      are palettes, and keeps the frame after a portrait's palette from claiming it. Where
//      the names file places both entries, they have to be from the same source file.
//   2. Backdrop: an image beside a 144-byte piece. dat_notes on 944: the main palette with the
//      piece stamped at slots 160..=207 renders every set cleanly, the images never use a slot
//      above 207. Sets are image, piece, image, so the first half's piece follows it and the
//      second half's precedes it.
//   3. SameSource: a palette stashed out of the same .lbm as the image (about_sprite_names.md:
//      talkwin9.lbm is 902 to 906, nwpanel1.lbm is 741 to 748). Needs a names file.
//   4. Inspected: the owner's readings in manual_inspection_of_palettes.md and dat_notes, for
//      the dialog and credits scenes where several files share one palette.
//   5. Default: the main game palette, 741 by the owner's inspection. Not a claim.
// Which palette a unit or building really uses comes out of the decompile and is still open.

use crate::dat::name_table::NameLookup;
use crate::error::{FormatError, Result};
use crate::file_format::SourceFileType;

/// The main game palette in the CD build: the one nwpanel1.lbm shipped with the main frame.
pub const MAIN_PALETTE_ENTRY: usize = 741;
/// Where a backdrop set's 48-colour piece lands in the main palette.
pub const BACKDROP_PIECE_SLOT: usize = 160;
/// Every character portrait is cut 0,0 to 114,103 and its palette stashed straight after.
pub const PORTRAIT_SIZE: (u16, u16) = (115, 104);

/// The owner's by-eye readings: (first, last, palette). Each traces to a note or the palette file.
const INSPECTED: &[(usize, usize, usize)] = &[
    (902, 905, 906),
    (918, 922, 917),
    (924, 925, 923),
    (927, 931, 926),
    (933, 937, 932),
    (939, 943, 938),
];

// --------------------------------------------------------------------------------------------------------------------
// What a choice looks like

/// What settled the choice. Ordered from evidence in the file down to an admitted default.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PaletteRule {
    /// The palette stored beside the image: portraits carry theirs after, credits before.
    Adjacent,
    /// A backdrop half: the main palette with the set's 48-colour piece stamped at slot 160.
    Backdrop,
    /// A palette stashed out of the same source .lbm as the image.
    SameSource,
    /// The owner's reading by eye, from manual_inspection_of_palettes.md and dat_notes.
    Inspected,
    /// Nothing ties the image to a palette; this is the main game palette and only a default.
    Default,
}

impl PaletteRule {
    /// A short account of the rule for a listing or a caption.
    pub fn describe(self) -> &'static str {
        match self {
            PaletteRule::Adjacent => "the palette stored beside this image in the archive",
            PaletteRule::Backdrop => "the main palette with this backdrop set's piece stamped at slot 160",
            PaletteRule::SameSource => "the palette stashed from the same source file as this image",
            PaletteRule::Inspected => "the owner's reading by eye, from the palette notes",
            PaletteRule::Default => "the main game palette by default; nothing ties this image to one",
        }
    }
}

/// A piece entry to lay over the base palette before drawing, and where.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PieceStamp {
    pub entry: usize,
    pub first_slot: usize,
}

/// Which palette to draw an image with, and how sure that is.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct PaletteChoice {
    /// The entry holding the 768-byte base palette.
    pub base: usize,
    pub stamp: Option<PieceStamp>,
    pub rule: PaletteRule,
}

impl PaletteChoice {
    /// A base palette used as it is, with nothing stamped over it.
    pub fn unstamped(base: usize, rule: PaletteRule) -> PaletteChoice {
        PaletteChoice { base, stamp: None, rule }
    }
}

/// Anything that can say which palette an entry is drawn with.
pub trait PaletteResolver {
    /// The choice for one image entry; an error for an entry that is not an image.
    fn palette_for(&self, index: usize) -> Result<PaletteChoice>;
}

// --------------------------------------------------------------------------------------------------------------------
// The rules

/// The rules above, applied once to one archive's types and names.
pub struct KnownPaletteRules {
    choices: Vec<Option<PaletteChoice>>,
    main_palette: usize,
}

impl std::fmt::Debug for KnownPaletteRules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let images = self.choices.iter().flatten().count();
        write!(f, "KnownPaletteRules for {images} images, main palette {}", self.main_palette)
    }
}

impl KnownPaletteRules {
    /// Work out every image's choice once, using the CD build's main palette.
    pub fn for_cd_build(types: &[SourceFileType], names: &dyn NameLookup) -> Result<KnownPaletteRules> {
        KnownPaletteRules::new(types, names, MAIN_PALETTE_ENTRY)
    }

    /// The same rules with a different default, for an archive that is not the CD build.
    pub fn new(types: &[SourceFileType], names: &dyn NameLookup, main_palette: usize) -> Result<KnownPaletteRules> {
        let is_palette = |i: usize| types.get(i).is_some_and(|t| t.is_palette());
        let is_piece = |i: usize| types.get(i).is_some_and(|t| *t == SourceFileType::PalettePiece);
        let is_portrait = |i: usize| types.get(i).and_then(|t| t.image_header()).is_some_and(|h| (h.width, h.height) == PORTRAIT_SIZE);
        // Adjacency is only evidence between entries the names file does not put in different files.
        let same_source = |a: usize, b: usize| match (names.origin_of(a), names.origin_of(b)) {
            (Some(x), Some(y)) => x.source == y.source,
            _ => true,
        };
        if !is_palette(main_palette) {
            return Err(FormatError::Palette(format!("entry {main_palette} is not a palette, so it cannot be the default")));
        }
        let mut choices = Vec::with_capacity(types.len());
        for (i, file_type) in types.iter().enumerate() {
            if !file_type.is_image() {
                choices.push(None);
                continue;
            }
            let before = i.checked_sub(1);
            let after = i + 1;
            let after_is_ours = is_portrait(i) && is_palette(after) && same_source(i, after);
            let before_is_ours = before.is_some_and(is_palette) && !i.checked_sub(2).is_some_and(is_portrait) && same_source(i, i - 1);
            let choice = if after_is_ours {
                PaletteChoice::unstamped(after, PaletteRule::Adjacent)
            } else if before_is_ours {
                PaletteChoice::unstamped(i - 1, PaletteRule::Adjacent)
            } else if before.is_some_and(is_piece) || is_piece(after) {
                // The second half of a set sits after its piece; the first half before it.
                let entry = if before.is_some_and(is_piece) { i - 1 } else { after };
                PaletteChoice { base: main_palette, stamp: Some(PieceStamp { entry, first_slot: BACKDROP_PIECE_SLOT }), rule: PaletteRule::Backdrop }
            } else if let Some(pal) = names.origin_of(i).and_then(|p| (p.source.first_entry..=p.source.last_entry).find(|j| is_palette(*j))) {
                PaletteChoice::unstamped(pal, PaletteRule::SameSource)
            } else if let Some(pal) = INSPECTED.iter().find(|(first, last, pal)| (first..=last).contains(&&i) && is_palette(*pal)).map(|t| t.2) {
                PaletteChoice::unstamped(pal, PaletteRule::Inspected)
            } else {
                PaletteChoice::unstamped(main_palette, PaletteRule::Default)
            };
            choices.push(Some(choice));
        }
        Ok(KnownPaletteRules { choices, main_palette })
    }

    /// The entry the Default rule answers with.
    pub fn main_palette(&self) -> usize {
        self.main_palette
    }
}

impl PaletteResolver for KnownPaletteRules {
    fn palette_for(&self, index: usize) -> Result<PaletteChoice> {
        match self.choices.get(index) {
            Some(Some(choice)) => Ok(*choice),
            Some(None) => Err(FormatError::WrongType { index, wanted: "an image", reason: "no palette applies to it".into() }),
            None => Err(FormatError::NoSuchEntry { index, count: self.choices.len() }),
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dat::name_table::NameTable;
    use crate::formats::image::{ImageHeader, ImageType};

    fn image() -> SourceFileType {
        SourceFileType::Image(ImageHeader { image_type: ImageType::Plain, width: 1, height: 1, anchor_x: 0, anchor_y: 0, advance: 0, data_size: 2, pixels_at: 9 })
    }

    fn portrait() -> SourceFileType {
        SourceFileType::Image(ImageHeader { image_type: ImageType::Placed, width: 115, height: 104, anchor_x: 0, anchor_y: 0, advance: 0, data_size: 2, pixels_at: 14 })
    }

    /// An archive laid out from a picture: i image, o portrait, P palette, p piece, . other.
    fn layout(picture: &str) -> Vec<SourceFileType> {
        picture.chars().map(|c| match c {
            'i' => image(),
            'o' => portrait(),
            'P' => SourceFileType::Palette,
            'p' => SourceFileType::PalettePiece,
            _ => SourceFileType::Unknown,
        }).collect()
    }

    #[test]
    fn each_rule_fires_in_order() {
        //                0123456789ab
        let t = layout("oPi.ipiipi.i");
        let names = NameTable::parse("scene.lbm\t3\t4\n").expect("names");
        let rules = KnownPaletteRules::new(&t, &names, 1).expect("builds");
        assert_eq!(rules.palette_for(0).expect("portrait, adjacent after"), PaletteChoice::unstamped(1, PaletteRule::Adjacent));
        assert_eq!(rules.palette_for(2).expect("the frame after a portrait's palette").rule, PaletteRule::Default);
        let first_half = rules.palette_for(4).expect("backdrop");
        assert_eq!((first_half.base, first_half.rule, first_half.stamp.map(|s| s.entry)), (1, PaletteRule::Backdrop, Some(5)));
        assert_eq!(rules.palette_for(6).expect("second half").stamp.map(|s| (s.entry, s.first_slot)), Some((5, 160)));
        assert_eq!(rules.palette_for(7).expect("first half of set 2").stamp.map(|s| s.entry), Some(8));
        assert_eq!(rules.palette_for(9).expect("second half of set 2").stamp.map(|s| s.entry), Some(8));
        assert_eq!(rules.palette_for(11).expect("default").rule, PaletteRule::Default);
        assert!(rules.palette_for(3).is_err(), "not an image");
        assert!(rules.palette_for(99).is_err());
        assert!(KnownPaletteRules::new(&t, &names, 0).is_err(), "the default must be a palette");
    }

    #[test]
    fn credits_alternate_palette_then_image_and_names_veto_a_neighbour() {
        //                01234567
        let t = layout("PiPiPi.i");
        let rules = KnownPaletteRules::new(&t, &NameTable::empty(), 0).expect("builds");
        assert_eq!(rules.palette_for(1).expect("between two").base, 0);
        assert_eq!(rules.palette_for(3).expect("between two").base, 2);
        assert_eq!(rules.palette_for(5).expect("last").base, 4);
        assert_eq!(rules.palette_for(7).expect("nothing beside it").rule, PaletteRule::Default);
        let names = NameTable::parse("a.lbm\t4\t4\nb.lbm\t5\t5\n").expect("names");
        let named = KnownPaletteRules::new(&t, &names, 0).expect("builds");
        assert_eq!(named.palette_for(5).expect("different file").rule, PaletteRule::Default);
        assert_eq!(named.palette_for(3).expect("unplaced, still adjacent").base, 2);
    }

    #[test]
    fn same_source_and_inspected_readings_apply() {
        let mut t = vec![SourceFileType::Unknown; 910];
        t[900..910].fill(image());
        t[906] = SourceFileType::Palette;
        t[741] = SourceFileType::Palette;
        let names = NameTable::parse("talkwin9.lbm\t902\t906\n").expect("names");
        let rules = KnownPaletteRules::for_cd_build(&t, &names).expect("builds");
        assert_eq!(rules.palette_for(902).expect("same file"), PaletteChoice::unstamped(906, PaletteRule::SameSource));
        assert_eq!(rules.palette_for(905).expect("beside it, but not a portrait"), PaletteChoice::unstamped(906, PaletteRule::SameSource));
        assert_eq!(rules.palette_for(900).expect("nothing known").rule, PaletteRule::Default);
        let unnamed = KnownPaletteRules::for_cd_build(&t, &NameTable::empty()).expect("builds");
        assert_eq!(unnamed.palette_for(903).expect("owner's reading"), PaletteChoice::unstamped(906, PaletteRule::Inspected));
    }
}
