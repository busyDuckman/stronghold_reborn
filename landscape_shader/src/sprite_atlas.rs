// --------------------------------------------------------------------------------------------------------------------
// The sprite atlas: the game art this test bed draws, decoded under one palette into one image.
//
// Which container entry stands for which map thing is a stand-in table, not the game's own
// binding (that comes out of the decompile and is not settled). The entry runs themselves
// are real: they are the source-file runs reversing/work/names/segmentation.tsv recovers
// from the packer's own code, and dat_sprites.yaml confirms the building sheets are cut as
// three upgrades of three views, view fastest.

use crate::dat::{Palette, SpriteImage, StrongDat};
use anyhow::{bail, Context, Result};
use std::collections::HashMap;

/// What a billboard shows.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum SpriteKey {
    /// A tree by its decor word (0x3d..=0x42 or 0x67..=0x6f).
    Tree { kind: u8 },
    /// A building by its decor word (0x92..=0x96), upgrade 1..=3 and view 0..=2.
    Building { kind: u8, upgrade: u8, view: u8 },
    /// A unit by its kind.
    Unit { kind: u8 },
}

/// The first entry of each tree sheet, build\tree01.lbm to tree06.lbm, twelve entries apiece.
const TREE_RUNS: [usize; 6] = [300, 312, 324, 336, 348, 360];
/// Entries per tree sheet.
const TREE_RUN_LEN: usize = 12;
/// The first entry of five confirmed building sheets: guest house, magres, cletem, thfgld, elfres.
const BUILDING_RUNS: [usize; 5] = [36, 54, 72, 90, 126];
/// The first frame of twelve unit animation runs: fighter, mage, cleric, thief, elf, dwarf,
/// halfling, orc, red dragon, bats, wights, griffon.
const UNIT_RUNS: [usize; 12] = [1170, 1264, 1458, 1676, 2011, 2239, 2359, 1979, 1846, 2540, 3081, 3154];

impl SpriteKey {
    /// The container entry that stands for this key. A stand-in mapping, documented here:
    ///   - the fifteen tree kinds cycle through the six sheets, taking slots 0, 4 and 8 of a
    ///     sheet in turn (what the twelve slots of a sheet mean is not established)
    ///   - the five building kinds take the first five confirmed building sheets, entry
    ///     base + 3 * (upgrade - 1) + view
    ///   - unit kinds take the first frame of a run, cycling past twelve
    pub fn entry(self) -> Result<usize> {
        match self {
            SpriteKey::Tree { kind } => {
                let index = match kind {
                    0x3d..=0x42 => (kind - 0x3d) as usize,
                    0x67..=0x6f => (kind - 0x67) as usize + 6,
                    other => bail!("{other:#x} is not a tree kind"),
                };
                let slot = (index / TREE_RUNS.len()) * 4;
                Ok(TREE_RUNS[index % TREE_RUNS.len()] + slot.min(TREE_RUN_LEN - 1))
            }
            SpriteKey::Building { kind, upgrade, view } => {
                if !(0x92..=0x96).contains(&kind) {
                    bail!("{kind:#x} is not a building kind");
                }
                if !(1..=3).contains(&upgrade) || view > 2 {
                    bail!("building upgrade {upgrade} view {view} is outside 1..=3 and 0..=2");
                }
                Ok(BUILDING_RUNS[(kind - 0x92) as usize] + 3 * (upgrade as usize - 1) + view as usize)
            }
            SpriteKey::Unit { kind } => Ok(UNIT_RUNS[kind as usize % UNIT_RUNS.len()]),
        }
    }

    /// Every key the atlas holds.
    pub fn all() -> Vec<SpriteKey> {
        let mut keys = Vec::new();
        for kind in (0x3d..=0x42).chain(0x67..=0x6f) {
            keys.push(SpriteKey::Tree { kind });
        }
        for kind in 0x92..=0x96 {
            for upgrade in 1..=3 {
                for view in 0..=2 {
                    keys.push(SpriteKey::Building { kind, upgrade, view });
                }
            }
        }
        for kind in 0..UNIT_RUNS.len() as u8 {
            keys.push(SpriteKey::Unit { kind });
        }
        keys
    }
}

/// Where one sprite sits in the atlas, and how it hangs off the point it is drawn at.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct AtlasSprite {
    /// u0, v0, u1, v1 in 0..=1.
    pub uv: [f32; 4],
    pub width: u16,
    pub height: u16,
    pub anchor_x: i16,
    pub anchor_y: i16,
    /// The record kind the entry opened with: 4 means the anchor is a unit's foot.
    pub kind: u8,
}

/// The atlas image and the table into it.
pub struct SpriteAtlas {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub palette_entry: usize,
    sprites: HashMap<SpriteKey, AtlasSprite>,
}

impl std::fmt::Debug for SpriteAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SpriteAtlas {}x{} with {} sprites under palette {}", self.width, self.height, self.sprites.len(), self.palette_entry)
    }
}

/// The atlas is this wide; sprites are shelved into rows below one another.
const ATLAS_WIDTH: u32 = 1024;
/// A one pixel gutter round every sprite so linear sampling never bleeds a neighbour.
const GUTTER: u32 = 1;

impl SpriteAtlas {
    /// Decode every key under one palette entry and pack the results.
    pub fn build(dat: &StrongDat, palette_entry: usize) -> Result<SpriteAtlas> {
        let palette = dat.palette(palette_entry).context("the atlas palette")?;
        let mut images: Vec<(SpriteKey, SpriteImage)> = Vec::new();
        for key in SpriteKey::all() {
            let entry = key.entry()?;
            let image = dat.image(entry).with_context(|| format!("{key:?} at entry {entry}"))?;
            images.push((key, image));
        }
        SpriteAtlas::pack(images, &palette, palette_entry)
    }

    fn pack(images: Vec<(SpriteKey, SpriteImage)>, palette: &Palette, palette_entry: usize) -> Result<SpriteAtlas> {
        // Shelf packing: fill a row left to right, start a new row when one does not fit.
        let mut placements = Vec::with_capacity(images.len());
        let (mut x, mut y, mut row_h) = (GUTTER, GUTTER, 0u32);
        for (_, image) in &images {
            let (w, h) = (image.width as u32, image.height as u32);
            if w + 2 * GUTTER > ATLAS_WIDTH {
                bail!("a {w}-wide sprite does not fit a {ATLAS_WIDTH}-wide atlas");
            }
            if x + w + GUTTER > ATLAS_WIDTH {
                x = GUTTER;
                y += row_h + GUTTER;
                row_h = 0;
            }
            placements.push((x, y));
            x += w + GUTTER;
            row_h = row_h.max(h);
        }
        let height = (y + row_h + GUTTER).next_power_of_two();
        let mut rgba = vec![0u8; (ATLAS_WIDTH * height * 4) as usize];
        let mut sprites = HashMap::with_capacity(images.len());
        for ((key, image), (px, py)) in images.into_iter().zip(placements) {
            for row in 0..image.height as u32 {
                for col in 0..image.width as u32 {
                    let i = (row * image.width as u32 + col) as usize;
                    if !image.mask[i] {
                        continue;
                    }
                    let [r, g, b] = palette.colours[image.pixels[i] as usize];
                    let at = (((py + row) * ATLAS_WIDTH + px + col) * 4) as usize;
                    rgba[at..at + 4].copy_from_slice(&[r, g, b, 255]);
                }
            }
            let (aw, ah) = (ATLAS_WIDTH as f32, height as f32);
            sprites.insert(
                key,
                AtlasSprite {
                    uv: [px as f32 / aw, py as f32 / ah, (px + image.width as u32) as f32 / aw, (py + image.height as u32) as f32 / ah],
                    width: image.width,
                    height: image.height,
                    anchor_x: image.anchor_x,
                    anchor_y: image.anchor_y,
                    kind: image.kind,
                },
            );
        }
        Ok(SpriteAtlas { width: ATLAS_WIDTH, height, rgba, palette_entry, sprites })
    }

    pub fn lookup(&self, key: SpriteKey) -> Option<&AtlasSprite> {
        self.sprites.get(&key)
    }

    pub fn len(&self) -> usize {
        self.sprites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sprites.is_empty()
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
    fn keys_map_to_entries_inside_their_runs() {
        assert_eq!(SpriteKey::Tree { kind: 0x3d }.entry().expect("tree"), 300);
        assert_eq!(SpriteKey::Tree { kind: 0x42 }.entry().expect("tree"), 360);
        assert_eq!(SpriteKey::Tree { kind: 0x67 }.entry().expect("tree"), 304);
        assert_eq!(SpriteKey::Tree { kind: 0x6f }.entry().expect("tree"), 332);
        assert_eq!(SpriteKey::Building { kind: 0x92, upgrade: 1, view: 0 }.entry().expect("building"), 36);
        assert_eq!(SpriteKey::Building { kind: 0x96, upgrade: 3, view: 2 }.entry().expect("building"), 134);
        assert_eq!(SpriteKey::Unit { kind: 0 }.entry().expect("unit"), 1170);
        assert_eq!(SpriteKey::Unit { kind: 12 }.entry().expect("unit"), 1170);
        assert!(SpriteKey::Tree { kind: 0x50 }.entry().is_err());
        assert!(SpriteKey::Building { kind: 0x92, upgrade: 4, view: 0 }.entry().is_err());
        assert_eq!(SpriteKey::all().len(), 15 + 45 + 12);
    }

    #[test]
    fn packing_places_every_sprite_without_overlap() {
        let mut images = Vec::new();
        for (i, key) in SpriteKey::all().into_iter().enumerate() {
            let (w, h) = (10 + (i % 7) as u16 * 30, 8 + (i % 5) as u16 * 20);
            let n = w as usize * h as usize;
            images.push((key, SpriteImage { width: w, height: h, anchor_x: 0, anchor_y: 0, kind: 1, pixels: vec![1; n], mask: vec![true; n] }));
        }
        let mut palette = Palette { colours: [[0; 3]; 256] };
        palette.colours[1] = [255, 0, 0];
        let atlas = SpriteAtlas::pack(images, &palette, 0).expect("packs");
        assert_eq!(atlas.len(), 72);
        let painted = atlas.rgba.chunks(4).filter(|p| p[3] == 255).count();
        let expected: usize = (0..72).map(|i| (10 + (i % 7) * 30) * (8 + (i % 5) * 20)).sum();
        assert_eq!(painted, expected, "gutters kept every sprite apart");
        let tree = atlas.lookup(SpriteKey::Tree { kind: 0x3d }).expect("present");
        assert!(tree.uv[2] > tree.uv[0] && tree.uv[3] > tree.uv[1]);
    }

    #[test]
    fn the_real_atlas_builds_when_the_container_is_here() {
        let Some(path) = StrongDat::find() else {
            println!("STRONG.DAT not on this machine; skipping");
            return;
        };
        let dat = StrongDat::open(&path).expect("opens");
        let first_palette = dat.palette_indices()[0];
        let atlas = SpriteAtlas::build(&dat, first_palette).expect("builds");
        assert_eq!(atlas.len(), 72);
        let tree = atlas.lookup(SpriteKey::Tree { kind: 0x3d }).expect("tree");
        assert_eq!((tree.width, tree.height), (48, 55));
        let unit = atlas.lookup(SpriteKey::Unit { kind: 0 }).expect("unit");
        assert_eq!(unit.anchor_y, 23);
    }
}
