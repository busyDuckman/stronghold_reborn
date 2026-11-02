// --------------------------------------------------------------------------------------------------------------------
// The game map: the world as the simulation sees it, one entry per cell.
//
// Cells are the unit of play. Trees, buildings and units sit on cells and the map's size in
// cells is the world's size: the original game is 148 by 60, and a bigger grid is a bigger
// world, not a finer one. Rendering detail is a separate, derived thing (see dem.rs).
//
// A cell (x, y) is a quad. Its four corner heights are height[x, y], height[x+1, y],
// height[x, y+1] and height[x+1, y+1], so the height grid is really a corner grid that the
// last column and row of cells read past; the game reads outside the grid as sea level and
// so does this. Each cell is split into two triangles along one diagonal (a terrain bit says
// which) and each triangle carries its own ground type, because that is how the original
// colours its map: a shoreline runs through a cell, not around it.

use anyhow::{anyhow, bail, Result};

/// The original game's grid, columns.
pub const STRONGHOLD_COLS: usize = 148;
/// The original game's grid, rows.
pub const STRONGHOLD_ROWS: usize = 60;
/// Highest height a corner takes; 0 is sea level.
pub const HEIGHT_MAX: u8 = 0x7f;

// The terrain word's bits, kept as the original game keeps them so a captured world drops in.
/// Triangle A (the one holding the cell's bottom-left corner) sits at sea level: water.
pub const TERRAIN_WATER_A: u16 = 0x0001;
/// Triangle B (the one holding the cell's top-right corner) sits at sea level: water.
pub const TERRAIN_WATER_B: u16 = 0x0002;
/// Both water bits.
pub const TERRAIN_WATER_MASK: u16 = 0x0003;
/// Set: the cell splits from its top-right corner to its bottom-left. Clear: top-left to bottom-right.
pub const TERRAIN_DIAGONAL: u16 = 0x0004;
/// Four edge bits, one per direction (up, right, down, left): 0x10 shifted left by the direction.
pub const TERRAIN_EDGE_MASK: u16 = 0x00f0;
/// Two occupancy bits the original's unit spawner sets. No generator here writes them.
pub const TERRAIN_OCCUPIED_MASK: u16 = 0x3000;

/// Which half of a cell.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Triangle {
    /// Holds the bottom-left corner (x, y+1).
    A,
    /// Holds the top-right corner (x+1, y).
    B,
}

impl Triangle {
    /// The terrain water bit for this triangle.
    pub fn water_bit(self) -> u16 {
        match self {
            Triangle::A => TERRAIN_WATER_A,
            Triangle::B => TERRAIN_WATER_B,
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Ground type

/// What a triangle of ground is made of. Stored, not derived: the generators fill it in and
/// later work will paint richer types than the height-and-slope rule that fills it today.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum GroundType {
    Water,
    Sand,
    Grass,
    Scrub,
    Rock,
    Snow,
}

impl GroundType {
    /// Every type, in display order.
    pub const ALL: [GroundType; 6] =
        [GroundType::Water, GroundType::Sand, GroundType::Grass, GroundType::Scrub, GroundType::Rock, GroundType::Snow];

    /// Short name for the sidebar.
    pub fn name(self) -> &'static str {
        match self {
            GroundType::Water => "water",
            GroundType::Sand => "sand",
            GroundType::Grass => "grass",
            GroundType::Scrub => "scrub",
            GroundType::Rock => "rock",
            GroundType::Snow => "snow",
        }
    }

    /// The original's shading terms for a triangle: a height band 0..=2 and a slope level 0..=4.
    ///   - The band is the three corner heights summed, shifted down seven and clamped; the
    ///     first add is eight bits wide and drops its carry, exactly as the game does it.
    ///   - The slope is the difference of two corners, quartered, offset by two and clamped.
    /// The corner order (a, b, c) is the game's own and matters for the slope sign.
    pub fn shade_terms(a: u8, b: u8, c: u8) -> (u8, u8) {
        let sum = (c.wrapping_add(a) as u16) + b as u16;
        let band = ((sum >> 7) as u8).min(2);
        let slope = ((b.wrapping_sub(a) as i8) >> 2) + 2;
        (band, slope.clamp(0, 4) as u8)
    }

    /// The placeholder rule that fills a triangle's type from its three corners.
    ///   - Water when the triangle's water bit is set.
    ///   - Sand where dry ground touches sea level.
    ///   - Otherwise by the original's height band: grass, scrub, rock; steep upland is rock,
    ///     and the highest ground is snow.
    /// NOTE: a stand-in for real terrain painting, kept simple on purpose.
    pub fn classify(a: u8, b: u8, c: u8, water: bool) -> GroundType {
        if water {
            return GroundType::Water;
        }
        if a == 0 || b == 0 || c == 0 {
            return GroundType::Sand;
        }
        let (band, slope) = GroundType::shade_terms(a, b, c);
        let steep = slope == 0 || slope == 4;
        match band {
            0 => GroundType::Grass,
            1 if steep => GroundType::Rock,
            1 => GroundType::Scrub,
            _ if !steep && a.min(b).min(c) > 0x6a => GroundType::Snow,
            _ => GroundType::Rock,
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// What stands on a cell

/// What stands on a cell. The original keeps one word per cell; these are its known values.
///   - Two families of tree, one placed on low ground and one on high ground.
///   - One family of building, placed on flat dips and spaced apart.
/// NOTE: which family is which is a reading of where the generator puts them, not something
/// the game's data has confirmed yet. The raw word survives in the kind field so nothing is lost.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Decor {
    Empty,
    /// One of the tree kinds; kind is the original's word for it.
    Tree { kind: u8 },
    /// A building. kind is the base kind, upgrade 1..=3 picks the next kinds along, and
    /// condition counts down from 8 (pristine) as it takes damage.
    Building { kind: u8, upgrade: u8, condition: u8 },
}

/// The building family's condition when new.
pub const BUILDING_CONDITION_NEW: u8 = 8;

impl Decor {
    /// From the original's word. Unknown non-zero values are an error, not an empty cell.
    pub fn from_word(word: u16) -> Result<Decor> {
        match word {
            0 => Ok(Decor::Empty),
            0x3d..=0x42 | 0x67..=0x6f => Ok(Decor::Tree { kind: word as u8 }),
            0x92..=0x96 => Ok(Decor::Building { kind: word as u8, upgrade: 1, condition: BUILDING_CONDITION_NEW }),
            _ => bail!("unknown decor word {word:#x}"),
        }
    }

    /// Back to the original's word (the base kind; upgrade and condition are not in it).
    pub fn word(self) -> u16 {
        match self {
            Decor::Empty => 0,
            Decor::Tree { kind } => kind as u16,
            Decor::Building { kind, .. } => kind as u16,
        }
    }

    pub fn is_tree(self) -> bool {
        matches!(self, Decor::Tree { .. })
    }

    pub fn is_building(self) -> bool {
        matches!(self, Decor::Building { .. })
    }
}

/// How many of each thing a map holds, for the sidebar.
#[derive(Copy, Clone, Default, Debug)]
pub struct DecorCounts {
    pub trees: usize,
    pub buildings: usize,
    pub water_triangles: usize,
}

// --------------------------------------------------------------------------------------------------------------------
// The map

/// The game map. All layers are row major, one entry per cell.
pub struct GameMap {
    cols: usize,
    rows: usize,
    height: Vec<u8>,
    terrain: Vec<u16>,
    ground: Vec<[GroundType; 2]>,
    decor: Vec<Decor>,
}

impl std::fmt::Debug for GameMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "GameMap {}x{}", self.cols, self.rows)
    }
}

impl GameMap {
    /// A flat map at sea level, every cell empty.
    pub fn new_flat(cols: usize, rows: usize) -> Result<GameMap> {
        let count = GameMap::checked_cell_count(cols, rows)?;
        Ok(GameMap {
            cols,
            rows,
            height: vec![0; count],
            terrain: vec![0; count],
            ground: vec![[GroundType::Water; 2]; count],
            decor: vec![Decor::Empty; count],
        })
    }

    /// From the three stored layers; the ground types are derived from them.
    pub fn from_layers(cols: usize, rows: usize, height: Vec<u8>, terrain: Vec<u16>, decor: Vec<Decor>) -> Result<GameMap> {
        let count = GameMap::checked_cell_count(cols, rows)?;
        if height.len() != count || terrain.len() != count || decor.len() != count {
            bail!(
                "layer sizes do not match a {cols}x{rows} map: height {}, terrain {}, decor {}",
                height.len(),
                terrain.len(),
                decor.len()
            );
        }
        if let Some(bad) = height.iter().find(|h| **h > HEIGHT_MAX) {
            bail!("height {bad} is above the maximum {HEIGHT_MAX}");
        }
        let mut map = GameMap { cols, rows, height, terrain, ground: vec![[GroundType::Water; 2]; count], decor };
        map.refresh_ground();
        Ok(map)
    }

    fn checked_cell_count(cols: usize, rows: usize) -> Result<usize> {
        if cols < 2 || rows < 2 {
            bail!("a map needs at least 2x2 cells, got {cols}x{rows}");
        }
        match cols.checked_mul(rows) {
            Some(count) if count <= u32::MAX as usize => Ok(count),
            _ => bail!("{cols}x{rows} cells is too many"),
        }
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn cell_count(&self) -> usize {
        self.height.len()
    }

    /// The row major index of a cell, or None outside the map.
    pub fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x as usize >= self.cols || y as usize >= self.rows {
            return None;
        }
        Some(y as usize * self.cols + x as usize)
    }

    /// The corner height at a grid point. Outside the grid is sea level, which is what the
    /// original reads there and what the last column and row of cells rely on.
    pub fn height_at(&self, x: i32, y: i32) -> u8 {
        self.index(x, y).and_then(|i| self.height.get(i).copied()).unwrap_or(0)
    }

    /// The ground height at a point in cell units, blended from the four corners around it.
    pub fn height_bilinear(&self, x: f32, y: f32) -> f32 {
        let (x0, y0) = (x.floor(), y.floor());
        let (tx, ty) = (x - x0, y - y0);
        let [h00, h10, h01, h11] = self.corner_heights(x0 as i32, y0 as i32).map(|h| h as f32);
        let top = h00 + (h10 - h00) * tx;
        let bottom = h01 + (h11 - h01) * tx;
        top + (bottom - top) * ty
    }

    /// A cell's four corner heights: top-left, top-right, bottom-left, bottom-right.
    pub fn corner_heights(&self, x: i32, y: i32) -> [u8; 4] {
        [self.height_at(x, y), self.height_at(x + 1, y), self.height_at(x, y + 1), self.height_at(x + 1, y + 1)]
    }

    pub fn terrain_at(&self, x: i32, y: i32) -> Option<u16> {
        self.index(x, y).and_then(|i| self.terrain.get(i).copied())
    }

    pub fn decor_at(&self, x: i32, y: i32) -> Option<Decor> {
        self.index(x, y).and_then(|i| self.decor.get(i).copied())
    }

    pub fn ground_at(&self, x: i32, y: i32) -> Option<[GroundType; 2]> {
        self.index(x, y).and_then(|i| self.ground.get(i).copied())
    }

    /// Whether a triangle is water, or None outside the map.
    pub fn is_water(&self, x: i32, y: i32, tri: Triangle) -> Option<bool> {
        self.terrain_at(x, y).map(|word| word & tri.water_bit() != 0)
    }

    /// Whether the cell splits from top-right to bottom-left (true) or top-left to bottom-right.
    pub fn splits_rising(&self, x: i32, y: i32) -> Option<bool> {
        self.terrain_at(x, y).map(|word| word & TERRAIN_DIAGONAL != 0)
    }

    pub fn set_height(&mut self, x: i32, y: i32, height: u8) -> Result<()> {
        if height > HEIGHT_MAX {
            bail!("height {height} is above the maximum {HEIGHT_MAX}");
        }
        let i = self.index(x, y).ok_or_else(|| anyhow!("({x}, {y}) is outside the map"))?;
        self.height[i] = height;
        Ok(())
    }

    pub fn set_terrain(&mut self, x: i32, y: i32, word: u16) -> Result<()> {
        let i = self.index(x, y).ok_or_else(|| anyhow!("({x}, {y}) is outside the map"))?;
        self.terrain[i] = word;
        Ok(())
    }

    pub fn set_decor(&mut self, x: i32, y: i32, decor: Decor) -> Result<()> {
        let i = self.index(x, y).ok_or_else(|| anyhow!("({x}, {y}) is outside the map"))?;
        self.decor[i] = decor;
        Ok(())
    }

    /// The three corners of a triangle in the order the original's shading reads them.
    pub fn triangle_corners(&self, x: i32, y: i32, tri: Triangle) -> [u8; 3] {
        let [h00, h10, h01, h11] = self.corner_heights(x, y);
        // Outside the map every corner is sea level, so which diagonal is taken changes nothing.
        let rising = self.splits_rising(x, y).unwrap_or(false);
        match (tri, rising) {
            (Triangle::A, true) => [h00, h10, h01],
            (Triangle::B, true) => [h01, h11, h10],
            (Triangle::A, false) => [h01, h11, h00],
            (Triangle::B, false) => [h00, h10, h11],
        }
    }

    /// Choose each cell's diagonal and mark its water triangles from the heights, the way the
    /// original does after it has rolled a height grid.
    ///   - The diagonal is the flatter of the two: the corners it joins differ by less.
    ///   - A triangle whose three corners are all at sea level is water.
    /// Existing diagonal and water bits are replaced; the other bits are kept.
    pub fn mark_diagonals_and_water(&mut self) {
        for y in 0..self.rows as i32 {
            for x in 0..self.cols as i32 {
                let [h00, h10, h01, h11] = self.corner_heights(x, y);
                let falling_span = (h00 as i32 - h11 as i32).abs();
                let rising_span = (h01 as i32 - h10 as i32).abs();
                let rising = rising_span < falling_span;
                let Some(i) = self.index(x, y) else { continue };
                let mut word = self.terrain[i] & !(TERRAIN_DIAGONAL | TERRAIN_WATER_MASK);
                if rising {
                    word |= TERRAIN_DIAGONAL;
                }
                let (tri_a, tri_b) =
                    if rising { ([h00, h10, h01], [h01, h11, h10]) } else { ([h01, h11, h00], [h00, h10, h11]) };
                if tri_a == [0, 0, 0] {
                    word |= TERRAIN_WATER_A;
                }
                if tri_b == [0, 0, 0] {
                    word |= TERRAIN_WATER_B;
                }
                self.terrain[i] = word;
            }
        }
    }

    /// Re-derive every triangle's ground type from the heights and water bits.
    pub fn refresh_ground(&mut self) {
        for y in 0..self.rows as i32 {
            for x in 0..self.cols as i32 {
                let Some(i) = self.index(x, y) else { continue };
                let word = self.terrain[i];
                let [a0, b0, c0] = self.triangle_corners(x, y, Triangle::A);
                let [a1, b1, c1] = self.triangle_corners(x, y, Triangle::B);
                self.ground[i] = [
                    GroundType::classify(a0, b0, c0, word & TERRAIN_WATER_A != 0),
                    GroundType::classify(a1, b1, c1, word & TERRAIN_WATER_B != 0),
                ];
            }
        }
    }

    /// Whether the cell is dry land on all four corners with no water bit set.
    pub fn is_dry(&self, x: i32, y: i32) -> bool {
        let Some(word) = self.terrain_at(x, y) else { return false };
        word & TERRAIN_WATER_MASK == 0 && self.corner_heights(x, y).iter().all(|h| *h > 0)
    }

    /// How far apart the cell's highest and lowest corners are; 0 is a flat cell.
    pub fn corner_spread(&self, x: i32, y: i32) -> u8 {
        let corners = self.corner_heights(x, y);
        let high = corners.iter().copied().max().unwrap_or(0);
        let low = corners.iter().copied().min().unwrap_or(0);
        high - low
    }

    pub fn counts(&self) -> DecorCounts {
        let mut counts = DecorCounts::default();
        for decor in &self.decor {
            match decor {
                Decor::Tree { .. } => counts.trees += 1,
                Decor::Building { .. } => counts.buildings += 1,
                Decor::Empty => {}
            }
        }
        counts.water_triangles = self.terrain.iter().map(|w| (w & TERRAIN_WATER_MASK).count_ones() as usize).sum();
        counts
    }

    /// Highest corner on the map.
    pub fn max_height(&self) -> u8 {
        self.height.iter().copied().max().unwrap_or(0)
    }

    /// The height layer, row major.
    pub fn heights(&self) -> &[u8] {
        &self.height
    }

    /// The terrain words, row major.
    pub fn terrain_words(&self) -> &[u16] {
        &self.terrain
    }

    /// The decor layer, row major.
    pub fn decor_layer(&self) -> &[Decor] {
        &self.decor
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
    fn shade_terms_match_the_originals_constants() {
        // Flat ground at zero: band 0, slope term exactly 2.
        assert_eq!(GroundType::shade_terms(0, 0, 0), (0, 2));
        // The first add drops its carry: 0x80 + 0x80 wraps to 0 before 0x7f is added, so the
        // band is 0 where a wide add would give 2. b - a is -1, so the slope term is 1.
        assert_eq!(GroundType::shade_terms(0x80, 0x7f, 0x80), (0, 1));
        // Three corners at the top of the range reach band 2.
        assert_eq!(GroundType::shade_terms(0x7f, 0x7f, 0x7f), (2, 2));
        // Slope clamps both ways.
        assert_eq!(GroundType::shade_terms(0, 40, 0).1, 4);
        assert_eq!(GroundType::shade_terms(40, 0, 0).1, 0);
    }

    #[test]
    fn decor_words_round_trip_and_reject_unknowns() {
        for word in [0u16, 0x3d, 0x42, 0x67, 0x6f, 0x92, 0x96] {
            let decor = Decor::from_word(word).expect("known word");
            assert_eq!(decor.word(), word);
        }
        assert!(Decor::from_word(0x43).is_err());
        assert!(Decor::from_word(0x97).is_err());
        assert!(Decor::from_word(1).is_err());
    }

    #[test]
    fn a_flat_map_is_all_water() {
        let mut map = GameMap::new_flat(4, 3).expect("map");
        map.mark_diagonals_and_water();
        map.refresh_ground();
        let counts = map.counts();
        assert_eq!(counts.water_triangles, 24);
        assert_eq!(map.ground_at(0, 0), Some([GroundType::Water; 2]));
        assert!(map.index(4, 0).is_none());
        assert!(map.index(-1, 0).is_none());
    }

    #[test]
    fn a_raised_corner_picks_the_flatter_diagonal() {
        let mut map = GameMap::new_flat(3, 3).expect("map");
        // Only the top-left corner of cell (0, 0) is raised: the rising diagonal joins two
        // sea-level corners, so it is the flatter one and triangle B is water.
        map.set_height(0, 0, 20).expect("in range");
        map.mark_diagonals_and_water();
        map.refresh_ground();
        assert_eq!(map.splits_rising(0, 0), Some(true));
        assert_eq!(map.is_water(0, 0, Triangle::A), Some(false));
        assert_eq!(map.is_water(0, 0, Triangle::B), Some(true));
        let ground = map.ground_at(0, 0).expect("in range");
        assert_eq!(ground[0], GroundType::Sand);
        assert_eq!(ground[1], GroundType::Water);
    }

    #[test]
    fn layer_sizes_are_checked() {
        assert!(GameMap::from_layers(2, 2, vec![0; 3], vec![0; 4], vec![Decor::Empty; 4]).is_err());
        assert!(GameMap::from_layers(2, 2, vec![0x80; 4], vec![0; 4], vec![Decor::Empty; 4]).is_err());
        assert!(GameMap::new_flat(1, 5).is_err());
    }
}
