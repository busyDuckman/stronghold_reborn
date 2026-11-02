// --------------------------------------------------------------------------------------------------------------------
// The original game's map generator, ported from the recovered C in reversing/src/world_gen.c.
//
// Given one of the game's seeds this produces the game's own 148 by 60 map: the same heights,
// the same water and the same rivers, byte for byte against a world captured from the real
// executable (see the test at the bottom). Where trees and buildings land is close but not
// identical, for a reason explained under "Stand-ins" below.
//
// The grids are kept as three 64K segments rather than tight arrays because the original
// builds every address in sixteen bits: a cell one step off the left edge reads the top of
// the segment rather than the byte before the block. Keeping the segment keeps that
// behaviour without a single bounds check, and the tests show it matters at the edges.
//
// Quirks of the original are kept and marked, not fixed. Names are in English; the C keeps
// the addresses for anyone who needs to go back to the executable.

use crate::game_map::{Decor, GameMap, STRONGHOLD_COLS, STRONGHOLD_ROWS};
use crate::rng::GameRng;
use anyhow::{Context, Result};

const COLS: i32 = STRONGHOLD_COLS as i32;
const ROWS: i32 = STRONGHOLD_ROWS as i32;
/// One byte per corner, row major.
const HEIGHT_ROW_STRIDE: u16 = 148;
/// One word per cell, row major.
const TERRAIN_ROW_STRIDE: u16 = 0x128;
/// One word per cell, column major (the original indexes this block the other way round).
const DECOR_COL_STRIDE: u16 = 0x78;
/// How much of each word grid the original clears before rolling.
const WORD_GRID_BYTES: usize = 17_760;
/// A 64K segment.
const SEGMENT_LEN: usize = 0x1_0000;

// The terrain word's low bits: two water triangles and the diagonal, as game_map.rs has them.
const WATER_A: u16 = 0x0001;
const WATER_MASK: u16 = 0x0003;
const DIAGONAL: u16 = 0x0004;

/// The x step for each direction: up, right, down, left.
const X_STEP: [i32; 4] = [0, 1, 0, -1];
/// The y step for each direction: up, right, down, left.
const Y_STEP: [i32; 4] = [-1, 0, 1, 0];

/// What a river writes into a cell's terrain word for a turn from one direction to the next,
/// indexed by (from * 4 + to). Zero means the walk ends; the four zeros all sit on reversals,
/// which the walk refuses anyway. Read out of the captured seed-100 world's data group.
const RIVER_TURN_TABLE: [u8; 16] = [3, 6, 0, 1, 5, 3, 1, 0, 0, 2, 3, 5, 2, 0, 6, 3];

/// The two corners of a cell a river crosses when it leaves in a direction, two per direction,
/// each corner coded as (index & 1, index >> 1). Directions 1 and 3 name a diagonal pair, not
/// an edge; that is what the table holds. Read out of the captured seed-100 world.
const RIVER_CORNER_TABLE: [u8; 8] = [0, 1, 1, 2, 2, 3, 3, 0];

// --------------------------------------------------------------------------------------------------------------------
// Stand-ins
//
// The generator asks three routines the reversing pass has not recovered yet. The port needs
// an answer from each, so these are the answers, chosen to keep the map populated:
//   - may the cell take a tree or building: always yes
//   - what shape is the ground at a dip between two rises (a 0..4 answer, and buildings only
//     go where it is 2): always 2
//   - does the cell show a visible edge on a side (four bits in the terrain word): never
// Consequence: heights, water and rivers match the game exactly because they are rolled
// before any of these are asked. Trees and buildings drift from the game's placement from
// the first refusal it would have made, because a refusal also skips random draws.

/// Whether a cell may take a tree or building. See "Stand-ins".
fn may_place_object(_x: i32, _y: i32) -> bool {
    true
}

/// The ground shape at a dip, 0..=4. See "Stand-ins".
fn dip_shape(_x: i32, _y: i32) -> u8 {
    2
}

/// Whether a cell shows a visible edge on one side. See "Stand-ins".
fn cell_has_edge(_x: i32, _y: i32, _dir: u8) -> bool {
    false
}

/// Whether a kind of thing standing two columns to the left blocks a building here. The
/// game reads one byte of a static record per kind; for every kind the generator can write
/// the byte is 1 except the building family's, which is 2, and 2 or more blocks.
fn kind_blocks_neighbour(kind: u16) -> bool {
    matches!(kind, 0x92..=0x96)
}

// --------------------------------------------------------------------------------------------------------------------
// Sixteen bit arithmetic

/// Absolute value on a sixteen bit word, the way the original's cwd/xor/sub idiom does it.
fn abs16(value: i32) -> i32 {
    let sign: u16 = if value < 0 { 0xffff } else { 0 };
    (((value as u16) ^ sign).wrapping_sub(sign)) as i16 as i32
}

/// Arithmetic shift right by one on a sixteen bit word.
fn sar16(value: i32) -> i32 {
    let bits = value as u16;
    ((bits >> 1) | (bits & 0x8000)) as i16 as i32
}

fn height_ofs(x: i32, y: i32) -> u16 {
    (y as u16).wrapping_mul(HEIGHT_ROW_STRIDE).wrapping_add(x as u16)
}

fn terrain_ofs(x: i32, y: i32) -> u16 {
    (y as u16).wrapping_mul(TERRAIN_ROW_STRIDE).wrapping_add((x as u16).wrapping_mul(2))
}

fn decor_ofs(x: i32, y: i32) -> u16 {
    (x as u16).wrapping_mul(DECOR_COL_STRIDE).wrapping_add((y as u16).wrapping_mul(2))
}

// --------------------------------------------------------------------------------------------------------------------
// A 64K segment

/// One of the original's far blocks, kept as the whole 64K segment it lived in. A u16 offset
/// can never miss, which is what makes every access below panic-free without a check.
struct Segment {
    bytes: Vec<u8>,
}

impl Segment {
    fn zeroed() -> Segment {
        Segment { bytes: vec![0; SEGMENT_LEN] }
    }

    fn byte(&self, ofs: u16) -> u8 {
        self.bytes[ofs as usize]
    }

    fn signed(&self, ofs: u16) -> i32 {
        self.bytes[ofs as usize] as i8 as i32
    }

    fn set_byte(&mut self, ofs: u16, value: u8) {
        self.bytes[ofs as usize] = value;
    }

    fn word(&self, ofs: u16) -> u16 {
        u16::from_le_bytes([self.bytes[ofs as usize], self.bytes[ofs.wrapping_add(1) as usize]])
    }

    fn set_word(&mut self, ofs: u16, value: u16) {
        let [low, high] = value.to_le_bytes();
        self.bytes[ofs as usize] = low;
        self.bytes[ofs.wrapping_add(1) as usize] = high;
    }

    fn or_word(&mut self, ofs: u16, bits: u16) {
        let word = self.word(ofs);
        self.set_word(ofs, word | bits);
    }
}

// --------------------------------------------------------------------------------------------------------------------
// The roll

/// The generator's working state: the generator and the three grids it writes.
pub struct StrongholdRoll {
    rng: GameRng,
    heights: Segment,
    terrain: Segment,
    decor: Segment,
    rivers: u8,
}

impl std::fmt::Debug for StrongholdRoll {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "StrongholdRoll(rng {:?}, {} rivers)", self.rng, self.rivers)
    }
}

impl StrongholdRoll {
    /// Roll the game's map for a seed, drawing from the generator in the game's own order.
    pub fn roll(seed: u32) -> StrongholdRoll {
        let mut roll = StrongholdRoll {
            rng: GameRng::seeded(seed),
            heights: Segment::zeroed(),
            terrain: Segment::zeroed(),
            decor: Segment::zeroed(),
            rivers: 0,
        };
        // The game's first draw after seeding sets a per-world byte whose meaning is not
        // known here. The draw has to happen so the stream stays aligned with the game's.
        let _unknown_per_world_byte = roll.rng.below(3) * 3;
        roll.rivers = roll.rng.below(0x14) as u8;
        roll.roll_terrain();
        roll
    }

    /// How many river walks this seed made.
    pub fn river_count(&self) -> u8 {
        self.rivers
    }

    /// The generator's state after the roll, for checking against a captured world.
    pub fn rng_state_hex(&self) -> String {
        self.rng.state_hex()
    }

    /// The finished map, lifted out of the segments into the game map's row major layers.
    pub fn into_game_map(self) -> Result<GameMap> {
        let count = STRONGHOLD_COLS * STRONGHOLD_ROWS;
        let mut height = Vec::with_capacity(count);
        let mut terrain = Vec::with_capacity(count);
        let mut decor = Vec::with_capacity(count);
        for y in 0..ROWS {
            for x in 0..COLS {
                height.push(self.heights.byte(height_ofs(x, y)));
                terrain.push(self.terrain.word(terrain_ofs(x, y)));
                let word = self.decor.word(decor_ofs(x, y));
                decor.push(Decor::from_word(word).with_context(|| format!("decor at ({x}, {y})"))?);
            }
        }
        GameMap::from_layers(STRONGHOLD_COLS, STRONGHOLD_ROWS, height, terrain, decor)
    }

    // ---- the grid readers the original bounds-checks ----

    /// The corner height at (x, y), or zero outside the grid.
    fn height_in_grid(&self, x: i32, y: i32) -> u8 {
        if x >= COLS || x < 0 || y >= ROWS || y < 0 {
            return 0;
        }
        self.heights.byte(height_ofs(x, y))
    }

    /// Store a corner height, doing nothing outside the grid.
    fn set_height_in_grid(&mut self, x: i32, y: i32, value: u8) {
        if x >= COLS || x < 0 || y >= ROWS || y < 0 {
            return;
        }
        self.heights.set_byte(height_ofs(x, y), value);
    }

    /// The two water bits at (x, y), or zero outside the grid.
    fn water_bits_in_grid(&self, x: i32, y: i32) -> u8 {
        if x >= COLS || x < 0 || y >= ROWS || y < 0 {
            return 0;
        }
        (self.terrain.word(terrain_ofs(x, y)) & WATER_MASK) as u8
    }

    /// The four corner heights of a cell added up, each widened unsigned.
    fn quad_height_sum(&self, x: i32, y: i32) -> u16 {
        (self.height_in_grid(x, y) as u16)
            .wrapping_add(self.height_in_grid(x + 1, y) as u16)
            .wrapping_add(self.height_in_grid(x, y + 1) as u16)
            .wrapping_add(self.height_in_grid(x + 1, y + 1) as u16)
    }

    /// True when none of the three neighbours other than the way back carries water.
    fn neighbours_dry(&self, x: i32, y: i32, dir: u8) -> bool {
        for i in 0..4usize {
            if i as u16 == (dir as u16 ^ 2) {
                continue;
            }
            if self.water_bits_in_grid(x + X_STEP[i], y + Y_STEP[i]) != 0 {
                return false;
            }
        }
        true
    }

    // ---- water ----

    /// Set a cell's water bits from its four corners: a triangle whose three corners are all
    /// at sea level is water. Which corners make each triangle depends on the diagonal bit.
    /// Neither read is bounded; a cell off the grid reads and writes wrapped offsets.
    fn mark_water_triangles(&mut self, x: i32, y: i32) {
        let cell = terrain_ofs(x, y);
        let at = height_ofs(x, y);
        let h = |ofs: u16| self.heights.byte(at.wrapping_add(ofs));
        let (down, right, here, far) = (h(HEIGHT_ROW_STRIDE), h(1), h(0), h(HEIGHT_ROW_STRIDE + 1));
        let (tri_a, tri_b) = if self.terrain.word(cell) & DIAGONAL != 0 {
            ([down, right, here], [down, right, far])
        } else {
            ([here, far, down], [here, far, right])
        };
        if tri_a == [0, 0, 0] {
            self.terrain.or_word(cell, WATER_A);
        }
        if tri_b == [0, 0, 0] {
            self.terrain.or_word(cell, WATER_MASK & !WATER_A);
        }
    }

    /// Water over the whole grid, one short on both axes because a cell reads x+1 and y+1.
    fn mark_water_everywhere(&mut self) {
        for y in 0..ROWS - 1 {
            for x in 0..COLS - 1 {
                self.mark_water_triangles(x, y);
            }
        }
    }

    /// The five cells a flattening touches: the cell and its four neighbours.
    fn redo_water_around(&mut self, x: i32, y: i32) {
        self.mark_water_triangles(x, y);
        self.mark_water_triangles(x - 1, y);
        self.mark_water_triangles(x + 1, y);
        self.mark_water_triangles(x, y - 1);
        self.mark_water_triangles(x, y + 1);
    }

    // ---- rivers ----

    /// Which way a river goes next. Sweeps the four directions from a random start and keeps
    /// three candidates: the lowest neighbouring cell, the lowest whose own neighbours are
    /// dry, and the last direction looked at. It prefers them in that order, so a river turns
    /// towards low ground and open ground. The turn and run bytes are the walk's memory of how many
    /// times in a row it has made the same turn; a third is refused, which stops circling.
    fn pick_river_direction(&mut self, x: i32, y: i32, dir: u8, turn: &mut u8, run: &mut u8) -> u8 {
        let mut best_any = self.quad_height_sum(x, y).wrapping_add(0x14);
        let mut best_clear = best_any;
        let mut pick_clear = 0xffu8;
        let mut pick_open = 0xffu8;
        let mut pick_any = 0xffu8;
        let mut last_seen = 0xffu8;
        let start = self.rng.below(4) as u8;
        let mut cur = start;
        loop {
            let delta = cur.wrapping_sub(dir) & 3;
            let refused = delta == 2 || (*run >= 2 && *turn == delta);
            if !refused {
                last_seen = cur;
                let (nx, ny) = (x + X_STEP[(cur & 3) as usize], y + Y_STEP[(cur & 3) as usize]);
                let score = self.quad_height_sum(nx, ny);
                if score <= best_any {
                    pick_any = cur;
                    best_any = score;
                }
                if self.neighbours_dry(nx, ny, cur) {
                    pick_open = cur;
                    if score <= best_clear {
                        pick_clear = cur;
                        best_clear = score;
                    }
                }
            }
            cur = cur.wrapping_add(1) & 3;
            if cur == start {
                break;
            }
        }
        let chosen = if pick_clear != 0xff {
            pick_clear
        } else if pick_open != 0xff {
            pick_open
        } else if pick_any != 0xff {
            pick_any
        } else {
            last_seen
        };
        let delta = chosen.wrapping_sub(dir) & 3;
        if delta != 0 {
            if *turn == delta {
                *run = run.wrapping_add(1);
            } else {
                *run = 0;
                *turn = delta;
            }
        }
        chosen
    }

    /// The corner a river table entry names, as an (x, y) offset within the cell.
    fn corner_offset(entry: u8) -> (i32, i32) {
        ((entry & 1) as i32, (entry >> 1) as i32)
    }

    /// Walk downhill from random starts, flattening as you go. Each walk starts at a random
    /// cell, flattens its four corners to the lowest of them, then steps: pick a direction,
    /// write the turn's water shape into the cell, flatten the two corners of the edge being
    /// crossed to the lowest height seen so far. A walk ends at the grid edge, on a cell that
    /// is already all water, at a reversal, or when the turn table says stop.
    fn carve_rivers(&mut self) {
        for _walk in 0..self.rivers {
            let mut x = self.rng.below(0x93) as i32;
            let mut y = self.rng.below(0x3b) as i32;
            let mut low: u8 = 0x64;
            let mut run = 0u8;
            let mut turn = 0u8;
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                low = low.min(self.height_in_grid(x + dx, y + dy));
            }
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                self.set_height_in_grid(x + dx, y + dy, low);
            }
            if low == 0 {
                self.redo_water_around(x, y);
            }
            let mut dir = self.rng.below(4) as u8;
            loop {
                if x >= COLS - 1 || x < 0 || y >= ROWS - 1 || y < 0 {
                    break;
                }
                let at = height_ofs(x, y);
                let corners = [0u16, 1, HEIGHT_ROW_STRIDE, HEIGHT_ROW_STRIDE + 1];
                if corners.iter().all(|ofs| self.heights.byte(at.wrapping_add(*ofs)) == 0) {
                    break;
                }
                let cell = terrain_ofs(x, y);
                let word = self.terrain.word(cell);
                if word & WATER_MASK == WATER_MASK {
                    break;
                }
                if word & WATER_MASK != 0 {
                    self.terrain.or_word(cell, WATER_MASK);
                }
                let next = self.pick_river_direction(x, y, dir, &mut turn, &mut run);
                // The picker always answers 0 to 3, so this cannot fire. It is the original's.
                if next == 0xff {
                    break;
                }
                if dir as u16 == (next as u16 ^ 2) {
                    break;
                }
                let write = RIVER_TURN_TABLE[((dir & 3) * 4 + (next & 3)) as usize];
                if write == 0 {
                    break;
                }
                let word = self.terrain.word(cell) & !DIAGONAL;
                self.terrain.set_word(cell, word | write as u16);
                let leaving = ((dir & 3) * 2) as usize;
                for entry in [RIVER_CORNER_TABLE[leaving], RIVER_CORNER_TABLE[leaving + 1]] {
                    let (dx, dy) = StrongholdRoll::corner_offset(entry);
                    low = low.min(self.height_in_grid(x + dx, y + dy));
                }
                let entering = ((next & 3) * 2) as usize;
                for entry in [RIVER_CORNER_TABLE[entering], RIVER_CORNER_TABLE[entering + 1]] {
                    let (dx, dy) = StrongholdRoll::corner_offset(entry);
                    self.set_height_in_grid(x + dx, y + dy, low);
                }
                if low == 0 {
                    self.redo_water_around(x, y);
                }
                x += X_STEP[(next & 3) as usize];
                y += Y_STEP[(next & 3) as usize];
                dir = next;
            }
        }
    }

    // ---- the height walk ----

    /// The random step's size for a running height, off a six way ladder.
    fn step_ladder(&mut self, acc: i32) -> i32 {
        if acc > 0x55 {
            self.rng.below(0x11) as i32 - 8
        } else if acc > 0x40 {
            self.rng.below(8) as i32 - 4
        } else if acc > 0x2a {
            self.rng.below(0xb) as i32 - 5
        } else if acc > 0x15 {
            self.rng.below(0x10) as i32 - 8
        } else if acc > 0 {
            self.rng.below(8) as i32 - 5
        } else {
            self.rng.below(9) as i32 - 2
        }
    }

    /// The random spread for a running height, off a ladder of a different shape.
    fn spread_ladder(acc: i32) -> i32 {
        if acc > 0x55 {
            9
        } else if acc > 0x40 {
            0x13
        } else if acc > 0x2a {
            5
        } else if acc > 0x15 {
            0xf
        } else if acc > 0 {
            5
        } else {
            0xb
        }
    }

    /// One tick of a column's slope bias, clamped to -1 ..= 1.
    fn slope_step(&mut self, slope: i8) -> i8 {
        let raw = ((self.rng.below(2) << 1).wrapping_add(slope as u8 as u16).wrapping_sub(1) & 0xff) as u8 as i8;
        raw.clamp(-1, 1)
    }

    /// Roll the terrain: heights, then water, then rivers, then what stands on it.
    fn roll_terrain(&mut self) {
        // 1. A slope profile, one bias per column, random walked; the two ends are flat.
        let mut profile = [0i8; STRONGHOLD_COLS];
        let mut slope: i8 = 0;
        for x in 0..STRONGHOLD_COLS {
            if self.rng.below(0xa) == 0 {
                slope = self.slope_step(slope);
            }
            if x == 0 || x == STRONGHOLD_COLS - 1 {
                slope = 0;
            }
            profile[x] = slope;
        }

        // 2. Both word grids cleared.
        self.decor.bytes[..WORD_GRID_BYTES].fill(0);
        self.terrain.bytes[..WORD_GRID_BYTES].fill(0);

        // 3. Sixty rows of heights, alternately left to right and right to left. Each corner
        // starts from its neighbour along the row plus the corner above (shifted by the
        // column's bias), halved, then takes a random step off the ladders. The two bodies
        // differ in two constants, almost certainly slips of the original author's; kept.
        let mut acc: i32;
        let mut step: i32 = 0;
        for y in 0..ROWS {
            let right_to_left = y & 1 != 0;
            let xs: Vec<i32> = if right_to_left { (0..COLS).rev().collect() } else { (0..COLS).collect() };
            for x in xs {
                let at_edge = if right_to_left { x == COLS - 1 } else { x == 0 };
                let prev = if right_to_left { x + 1 } else { x - 1 };
                let at = height_ofs(x, y);
                acc = 0;
                if !at_edge {
                    acc = self.heights.signed(height_ofs(prev, y));
                }
                if y != 0 {
                    let biased = at.wrapping_add(profile[x as usize] as i8 as u16).wrapping_sub(HEIGHT_ROW_STRIDE);
                    acc += self.heights.signed(biased);
                }
                acc = sar16(acc);
                if !at_edge && profile[prev as usize] != profile[x as usize] {
                    // The smoothing rolls out of 0x10 going right to left and 8 going left to right.
                    let pick = self.rng.below(if right_to_left { 0x10 } else { 8 });
                    if pick == 1 {
                        profile[x as usize] = profile[prev as usize];
                    } else if pick == 2 {
                        profile[prev as usize] = profile[x as usize];
                    }
                }
                if self.rng.below(0x10) == 0 {
                    if !at_edge {
                        slope = self.slope_step(profile[prev as usize]);
                        profile[x as usize] = slope;
                    }
                    step = self.step_ladder(acc);
                }
                let spread = StrongholdRoll::spread_ladder(acc);
                acc += self.rng.below(spread as u16) as i32 - (spread >> 1) + step + 1;
                // The low clamp is -0x7f going right to left and -0x23 going left to right.
                let low_clamp = if right_to_left { -0x7f } else { -0x23 };
                if acc < low_clamp {
                    acc = low_clamp;
                    step = abs16(step);
                }
                if acc > 0x7f {
                    acc = 0x7f;
                    step = -abs16(step);
                }
                self.heights.set_byte(at, acc as u8);
            }
        }

        // 4. Negatives become sea level, and each cell takes the flatter diagonal.
        for y in 0..ROWS {
            for x in 0..COLS {
                let at = height_ofs(x, y);
                if self.heights.signed(at) < 0 {
                    self.heights.set_byte(at, 0);
                }
                if x == COLS - 1 {
                    continue;
                }
                let down = abs16(self.heights.signed(at.wrapping_add(HEIGHT_ROW_STRIDE)) - self.heights.signed(at.wrapping_add(1)));
                let across =
                    abs16(self.heights.signed(at) - self.heights.signed(at.wrapping_add(HEIGHT_ROW_STRIDE + 1)));
                if down < across {
                    self.terrain.or_word(terrain_ofs(x, y), DIAGONAL);
                }
            }
        }

        // 5, 6, 7. Water, rivers, and water again because the rivers moved heights.
        self.mark_water_everywhere();
        self.carve_rivers();
        self.mark_water_everywhere();

        // 8. Trees, one word per cell, by height band with a random arm per band.
        self.place_trees();

        // 9. Buildings, beside flat dips, spaced apart.
        self.place_buildings();

        // 10. Visible edges, four bits per cell.
        self.mark_edges();
    }

    /// The tree pass. Six height bands, each with its own odds and its own family.
    fn place_trees(&mut self) {
        for y in 0..ROWS - 1 {
            for x in 0..COLS - 1 {
                let at = height_ofs(x, y);
                let h = |s: &Self, ofs: u16| s.heights.byte(at.wrapping_add(ofs));
                // The original tests the corner below twice and never the far corner; kept.
                if h(self, 0) == 0 || h(self, 1) == 0 || h(self, HEIGHT_ROW_STRIDE) == 0 || h(self, HEIGHT_ROW_STRIDE) == 0 {
                    continue;
                }
                if self.water_bits_in_grid(x, y) != 0 {
                    continue;
                }
                let height = self.heights.signed(at);
                let cell = decor_ofs(x, y);
                if height > 0x6a {
                    let pick = self.rng.below(4);
                    if pick != 2 && pick != 3 {
                        continue;
                    }
                    if !may_place_object(x, y) {
                        continue;
                    }
                    let word = self.rng.below(9) + 0x67;
                    self.decor.set_word(cell, word);
                } else if height > 0x55 {
                    let pick = self.rng.below(4);
                    if pick == 1 {
                        if !may_place_object(x, y) {
                            continue;
                        }
                        let word = if self.rng.below(2) != 0 { self.rng.below(9) + 0x67 } else { self.rng.below(6) + 0x3d };
                        self.decor.set_word(cell, word);
                    } else if pick == 3 {
                        if !may_place_object(x, y) {
                            continue;
                        }
                        let word = self.rng.below(9) + 0x67;
                        self.decor.set_word(cell, word);
                    }
                } else if height > 0x3f {
                    let pick = self.rng.below(4);
                    if pick != 3 {
                        continue;
                    }
                    if !may_place_object(x, y) {
                        continue;
                    }
                    if self.rng.below(2) == 0 {
                        continue;
                    }
                    let word = self.rng.below(9) + 0x67;
                    self.decor.set_word(cell, word);
                } else if height > 0x2a {
                    let pick = self.rng.below(4);
                    if pick == 1 {
                        let word = self.rng.below(6) + 0x3d;
                        self.decor.set_word(cell, word);
                    } else if pick == 3 {
                        if !may_place_object(x, y) {
                            continue;
                        }
                        let word = self.rng.below(9) + 0x67;
                        self.decor.set_word(cell, word);
                    }
                } else if height > 0x15 {
                    // Four arms that all do the same thing: the roll is made and thrown away.
                    let _pick = self.rng.below(4);
                    let word = self.rng.below(6) + 0x3d;
                    self.decor.set_word(cell, word);
                } else {
                    let pick = self.rng.below(4);
                    if pick == 2 {
                        let word = self.rng.below(6) + 0x3d;
                        self.decor.set_word(cell, word);
                    } else if pick == 3 {
                        if !may_place_object(x, y) {
                            continue;
                        }
                        if self.rng.below(2) != 0 {
                            continue;
                        }
                        let word = self.rng.below(9) + 0x67;
                        self.decor.set_word(cell, word);
                    }
                }
            }
        }
    }

    /// The building pass. One in two interior cells is considered; it has to be dry on six
    /// corners, its two left corners no higher than their neighbours (a dip), not water on
    /// this cell or the one to its left, of the right shape, and not two columns from another
    /// building. What passes gets a building written into the cell to its LEFT and itself cleared.
    fn place_buildings(&mut self) {
        for y in 1..ROWS - 2 {
            for x in 1..COLS - 2 {
                if self.rng.below(2) != 0 {
                    continue;
                }
                let at = height_ofs(x, y);
                let h = |s: &Self, delta: i32| s.heights.byte(at.wrapping_add(delta as u16));
                let hs = |s: &Self, delta: i32| s.heights.signed(at.wrapping_add(delta as u16));
                let row = HEIGHT_ROW_STRIDE as i32;
                let six = [-1, row - 1, 0, row, 1, row + 1];
                if six.iter().any(|d| h(self, *d) == 0) {
                    continue;
                }
                let comparisons = [(0, 1), (0, -1), (0, -row), (1, -row + 1), (0, row), (1, row + 1)];
                if comparisons.iter().any(|(a, b)| hs(self, *a) > hs(self, *b)) {
                    continue;
                }
                if self.water_bits_in_grid(x, y) != 0 || self.water_bits_in_grid(x - 1, y) != 0 {
                    continue;
                }
                if dip_shape(x, y) != 2 {
                    continue;
                }
                let two_left = self.decor.word(decor_ofs(x, y).wrapping_sub(2 * DECOR_COL_STRIDE));
                if kind_blocks_neighbour(two_left) {
                    continue;
                }
                if dip_shape(x, y) >= 3 {
                    continue;
                }
                let word = self.rng.below(5) + 0x92;
                self.decor.set_word(decor_ofs(x, y).wrapping_sub(DECOR_COL_STRIDE), word);
                self.decor.set_word(decor_ofs(x, y), 0);
            }
        }
    }

    /// Four edge bits per cell, one per direction, from the stand-in.
    fn mark_edges(&mut self) {
        for x in 0..COLS {
            for y in 0..ROWS {
                for dir in 0..4u8 {
                    if cell_has_edge(x, y, dir) {
                        self.terrain.or_word(terrain_ofs(x, y), 0x10 << dir);
                    }
                }
            }
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
    use crate::game_map::TERRAIN_WATER_MASK;

    // The seed-100 world as the real executable rolled it, captured under the reversing
    // pass's oracle with only audio and the palette stubbed.
    const WORLD_100_HEIGHT: &[u8] = include_bytes!("../test_data/world_100_height.bin");
    const WORLD_100_TERRAIN: &[u8] = include_bytes!("../test_data/world_100_terrain.bin");
    const WORLD_100_DECOR: &[u8] = include_bytes!("../test_data/world_100_decor.bin");

    fn words(bytes: &[u8]) -> Vec<u16> {
        bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect()
    }

    #[test]
    fn sixteen_bit_helpers() {
        assert_eq!(abs16(-5), 5);
        assert_eq!(abs16(7), 7);
        assert_eq!(abs16(-0x8000), -0x8000);
        assert_eq!(sar16(-3), -2);
        assert_eq!(sar16(7), 3);
        assert_eq!(height_ofs(-1, 0), 0xffff);
        assert_eq!(decor_ofs(1, 2).wrapping_sub(2 * DECOR_COL_STRIDE), 0xff8c);
    }

    #[test]
    fn seed_100_heights_and_water_match_the_captured_world() {
        let roll = StrongholdRoll::roll(100);
        assert_eq!(roll.river_count(), 16);
        let map = roll.into_game_map().expect("a valid map");
        assert_eq!(map.heights(), WORLD_100_HEIGHT, "heights differ from the captured world");
        let captured = words(WORLD_100_TERRAIN);
        let low_bits: Vec<u16> = map.terrain_words().iter().map(|w| w & 0x07).collect();
        let captured_low: Vec<u16> = captured.iter().map(|w| w & 0x07).collect();
        assert_eq!(low_bits, captured_low, "water or diagonal bits differ from the captured world");
        // The captured world carries edge bits the stand-in does not produce.
        assert!(captured.iter().any(|w| w & 0xf0 != 0));
        assert!(map.terrain_words().iter().all(|w| w & 0xf0 == 0));
    }

    #[test]
    fn seed_100_objects_are_close_but_not_exact() {
        // Trees and buildings drift from the game's placement once a refused placement would
        // have skipped random draws. This pins how close the stand-ins get so a change that
        // makes it worse is noticed.
        let map = StrongholdRoll::roll(100).into_game_map().expect("a valid map");
        let mut captured = vec![Decor::Empty; STRONGHOLD_COLS * STRONGHOLD_ROWS];
        let captured_words = words(WORLD_100_DECOR);
        for x in 0..STRONGHOLD_COLS {
            for y in 0..STRONGHOLD_ROWS {
                let word = captured_words[x * (DECOR_COL_STRIDE as usize / 2) + y];
                captured[y * STRONGHOLD_COLS + x] = Decor::from_word(word).expect("captured words are known");
            }
        }
        let same = map.decor_layer().iter().zip(&captured).filter(|(a, b)| a == b).count();
        let total = captured.len();
        let counts = map.counts();
        let captured_trees = captured.iter().filter(|d| d.is_tree()).count();
        let captured_buildings = captured.iter().filter(|d| d.is_building()).count();
        println!("decor cells identical: {same}/{total}; trees {} vs {captured_trees}; buildings {} vs {captured_buildings}", counts.trees, counts.buildings);
        assert!(same * 100 / total >= 50, "only {same} of {total} cells agree with the captured world");
        assert!(counts.trees > 0 && counts.buildings > 0);
    }

    #[test]
    fn the_five_named_worlds_roll_to_valid_maps() {
        for seed in crate::rng::NAMED_WORLD_SEEDS {
            let map = StrongholdRoll::roll(seed).into_game_map().expect("a valid map");
            let counts = map.counts();
            assert!(counts.water_triangles > 0, "seed {seed} has no water");
            assert!(counts.trees > 0, "seed {seed} has no trees");
            assert!(map.max_height() > 0x20, "seed {seed} is flat");
            assert!(map.terrain_words().iter().all(|w| w & !(TERRAIN_WATER_MASK | DIAGONAL) == 0));
        }
    }
}
