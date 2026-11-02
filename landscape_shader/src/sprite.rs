// --------------------------------------------------------------------------------------------------------------------
// A sprite: one unit on the map, shaped like the original's unit records.
//
// The game keeps its units in one table of 600 records of 0x37 bytes. Only the fields this
// test bed draws are modelled: where it is, where it is heading, what it is and whose it is.
// Positions are in cells, kept as floats so a later pass can move a unit smoothly between
// cells without changing the type.

use anyhow::{bail, Result};

/// The original's unit table holds at most this many records.
pub const GROUP_TABLE_CAP: usize = 600;
/// One record of the original's unit table.
pub const GROUP_RECORD_LEN: usize = 0x37;
/// How many unit kinds the original has.
pub const SPRITE_KIND_COUNT: u8 = 12;
/// How many sides the original has.
pub const SPRITE_SIDE_COUNT: u8 = 24;
/// Flag: the record is skipped by every search the game makes over the table.
pub const SPRITE_FLAG_SKIPPED: u8 = 0x02;
/// Flag: the record has been given an order this turn.
pub const SPRITE_FLAG_ORDERED: u8 = 0x08;

/// One unit.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Sprite {
    pub x: f32,
    pub y: f32,
    pub dest_x: f32,
    pub dest_y: f32,
    pub kind: u8,
    pub side: u8,
    pub flags: u8,
}

impl Sprite {
    /// A unit standing still at a cell.
    pub fn standing(x: f32, y: f32, kind: u8, side: u8) -> Sprite {
        Sprite { x, y, dest_x: x, dest_y: y, kind, side, flags: 0 }
    }

    /// From one 0x37-byte record of the original's unit table.
    ///   - position at +0 and +2, destination at +8 and +10, all signed sixteen bit
    ///   - kind at +0x0d, side at +0x1a, flags at +0x33
    pub fn from_record(record: &[u8]) -> Result<Sprite> {
        if record.len() != GROUP_RECORD_LEN {
            bail!("a unit record is {GROUP_RECORD_LEN} bytes, got {}", record.len());
        }
        let word = |at: usize| i16::from_le_bytes([record[at], record[at + 1]]) as f32;
        Ok(Sprite {
            x: word(0),
            y: word(2),
            dest_x: word(8),
            dest_y: word(10),
            kind: record[0x0d],
            side: record[0x1a],
            flags: record[0x33],
        })
    }

    /// Every record in a run of them, back to back.
    pub fn from_records(bytes: &[u8]) -> Result<Vec<Sprite>> {
        if bytes.len() % GROUP_RECORD_LEN != 0 {
            bail!("{} bytes is not a whole number of {GROUP_RECORD_LEN}-byte records", bytes.len());
        }
        bytes.chunks(GROUP_RECORD_LEN).map(Sprite::from_record).collect()
    }

    pub fn is_moving(&self) -> bool {
        self.x != self.dest_x || self.y != self.dest_y
    }

    pub fn is_skipped(&self) -> bool {
        self.flags & SPRITE_FLAG_SKIPPED != 0
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    /// The 46 records of the captured seed-100 world after the game's spawner populated it.
    const WORLD_100_GROUPS: &[u8] = include_bytes!("../test_data/world_100_groups.bin");

    #[test]
    fn captured_records_parse() {
        let sprites = Sprite::from_records(WORLD_100_GROUPS).expect("fixture parses");
        assert_eq!(sprites.len(), 46);
        let first = sprites[0];
        assert_eq!((first.x, first.y), (30.0, 0.0));
        assert_eq!((first.dest_x, first.dest_y), (137.0, 1.0));
        assert_eq!(first.kind, 0);
        assert_eq!(first.side, 0);
        assert!(first.is_skipped());
        assert!(first.is_moving());
        assert!(sprites.iter().all(|s| s.x >= 0.0 && s.x < 148.0 && s.y >= 0.0 && s.y < 60.0));
        assert!(sprites.iter().all(|s| s.side < SPRITE_SIDE_COUNT));
    }

    #[test]
    fn short_records_are_refused() {
        assert!(Sprite::from_record(&[0; 10]).is_err());
        assert!(Sprite::from_records(&[0; GROUP_RECORD_LEN + 1]).is_err());
    }
}
