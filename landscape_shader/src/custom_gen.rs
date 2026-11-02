// --------------------------------------------------------------------------------------------------------------------
// The custom map generator: any size, seeded, tunable, nothing to do with the original's roll.
//
// Heights come from fractal noise cut off at a sea level, water and diagonals follow the
// same rule the game applies to its own heights, and then trees, buildings and units are
// scattered with a seeded generator. Same parameters, same map.

use crate::game_map::{Decor, GameMap, BUILDING_CONDITION_NEW, HEIGHT_MAX};
use crate::noise::fbm;
use crate::sprite::{Sprite, SPRITE_KIND_COUNT, SPRITE_SIDE_COUNT};
use anyhow::{bail, Result};
use wyrand::WyRand;

/// What the custom generator is asked for.
#[derive(Clone, PartialEq, Debug)]
pub struct CustomParams {
    pub cols: usize,
    pub rows: usize,
    pub seed: u64,
    /// The noise value below which ground is sea, 0.0 ..= 1.0. Higher means more water.
    pub sea_level: f32,
    /// The height the highest ground reaches, 1 ..= HEIGHT_MAX.
    pub relief: u8,
    /// How much the finer noise octaves contribute, 0.0 ..= 1.0. Higher is rougher.
    pub roughness: f32,
    /// How large the landforms are, in cells per noise period.
    pub feature_size: f32,
    /// The chance a dry cell gets a tree, 0.0 ..= 1.0.
    pub tree_density: f32,
    pub building_count: usize,
    pub sprite_count: usize,
}

impl Default for CustomParams {
    fn default() -> CustomParams {
        CustomParams {
            cols: 148,
            rows: 60,
            seed: 7,
            sea_level: 0.42,
            relief: 110,
            roughness: 0.5,
            feature_size: 24.0,
            tree_density: 0.35,
            building_count: 40,
            sprite_count: 60,
        }
    }
}

impl CustomParams {
    /// Refuse parameters that cannot make a map, with a reason.
    pub fn validate(&self) -> Result<()> {
        if self.cols < 2 || self.rows < 2 || self.cols > 4096 || self.rows > 4096 {
            bail!("map size {}x{} is outside 2..=4096 on each axis", self.cols, self.rows);
        }
        if !(0.0..=1.0).contains(&self.sea_level) {
            bail!("sea level {} is outside 0..=1", self.sea_level);
        }
        if self.relief == 0 || self.relief > HEIGHT_MAX {
            bail!("relief {} is outside 1..={HEIGHT_MAX}", self.relief);
        }
        if !(0.0..=1.0).contains(&self.roughness) {
            bail!("roughness {} is outside 0..=1", self.roughness);
        }
        if !(1.0..=1024.0).contains(&self.feature_size) {
            bail!("feature size {} is outside 1..=1024", self.feature_size);
        }
        if !(0.0..=1.0).contains(&self.tree_density) {
            bail!("tree density {} is outside 0..=1", self.tree_density);
        }
        Ok(())
    }
}

/// How uneven a cell may be and still take a building, in height units.
const BUILDING_MAX_SPREAD: u8 = 4;

/// A value in 0.0 ..< 1.0 from the generator.
fn unit(rng: &mut WyRand) -> f32 {
    (rng.rand() >> 40) as f32 / (1u64 << 24) as f32
}

/// A value in 0 ..< n from the generator; n must be non-zero.
fn below(rng: &mut WyRand, n: u64) -> u64 {
    rng.rand() % n.max(1)
}

/// Build the map: heights, water, trees and buildings. Units are scattered separately.
pub fn generate_map(params: &CustomParams) -> Result<GameMap> {
    params.validate()?;
    let mut map = GameMap::new_flat(params.cols, params.rows)?;
    let scale = 1.0 / params.feature_size;
    for y in 0..params.rows as i32 {
        for x in 0..params.cols as i32 {
            let n = fbm(x as f32 * scale, y as f32 * scale, params.seed, 6, params.roughness);
            let above_sea = ((n - params.sea_level) / (1.0 - params.sea_level).max(1e-3)).max(0.0);
            let height = (above_sea.powf(1.3) * params.relief as f32).round().min(HEIGHT_MAX as f32) as u8;
            map.set_height(x, y, height)?;
        }
    }
    map.mark_diagonals_and_water();
    let mut rng = WyRand::new(params.seed ^ 0x5eed_5eed);
    place_trees(&mut map, params, &mut rng)?;
    place_buildings(&mut map, params.building_count, &mut rng)?;
    map.refresh_ground();
    Ok(map)
}

/// Trees on dry cells, clumped by a second noise field so they come in woods, not static.
fn place_trees(map: &mut GameMap, params: &CustomParams, rng: &mut WyRand) -> Result<()> {
    for y in 0..map.rows() as i32 {
        for x in 0..map.cols() as i32 {
            if !map.is_dry(x, y) {
                continue;
            }
            let clump = fbm(x as f32 * 0.15, y as f32 * 0.15, params.seed ^ 0x7ee5, 3, 0.5);
            let chance = params.tree_density * (clump * 1.6).min(1.0);
            if unit(rng) >= chance {
                continue;
            }
            // Low ground takes the smaller family, high ground the larger, as the original does.
            let kind = if map.height_at(x, y) > 0x55 { 0x67 + below(rng, 9) as u8 } else { 0x3d + below(rng, 6) as u8 };
            map.set_decor(x, y, Decor::Tree { kind })?;
        }
    }
    Ok(())
}

/// Buildings on nearly flat dry empty cells, at least three cells from each other.
fn place_buildings(map: &mut GameMap, count: usize, rng: &mut WyRand) -> Result<()> {
    let mut placed: Vec<(i32, i32)> = Vec::with_capacity(count);
    let attempts = count.saturating_mul(50);
    for _ in 0..attempts {
        if placed.len() >= count {
            break;
        }
        let x = below(rng, map.cols() as u64) as i32;
        let y = below(rng, map.rows() as u64) as i32;
        if !map.is_dry(x, y) || map.corner_spread(x, y) > BUILDING_MAX_SPREAD || map.decor_at(x, y) != Some(Decor::Empty) {
            continue;
        }
        if placed.iter().any(|(px, py)| (px - x).abs() < 3 && (py - y).abs() < 3) {
            continue;
        }
        let kind = 0x92 + below(rng, 5) as u8;
        let upgrade = 1 + below(rng, 3) as u8;
        map.set_decor(x, y, Decor::Building { kind, upgrade, condition: BUILDING_CONDITION_NEW })?;
        placed.push((x, y));
    }
    if placed.len() < count {
        log::warn!("asked for {count} buildings, found flat dry room for {}", placed.len());
    }
    Ok(())
}

/// Units scattered on dry cells, every side and kind represented, half of them on the move.
pub fn scatter_sprites(map: &GameMap, count: usize, seed: u64) -> Vec<Sprite> {
    let mut rng = WyRand::new(seed ^ 0x5921_7e5);
    let mut dry: Vec<(i32, i32)> = Vec::new();
    for y in 0..map.rows() as i32 {
        for x in 0..map.cols() as i32 {
            if map.is_dry(x, y) {
                dry.push((x, y));
            }
        }
    }
    if dry.is_empty() {
        log::warn!("no dry cells to place {count} units on");
        return Vec::new();
    }
    let mut sprites = Vec::with_capacity(count);
    for i in 0..count {
        let (x, y) = dry[below(&mut rng, dry.len() as u64) as usize];
        let side = (i % SPRITE_SIDE_COUNT as usize) as u8;
        let kind = below(&mut rng, SPRITE_KIND_COUNT as u64) as u8;
        let mut sprite = Sprite::standing(x as f32, y as f32, kind, side);
        if below(&mut rng, 2) == 0 {
            let (dx, dy) = dry[below(&mut rng, dry.len() as u64) as usize];
            sprite.dest_x = dx as f32;
            sprite.dest_y = dy as f32;
        }
        sprites.push(sprite);
    }
    sprites
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_params_make_a_populated_map() {
        let params = CustomParams::default();
        let map = generate_map(&params).expect("map");
        let counts = map.counts();
        assert!(counts.water_triangles > 100, "{counts:?}");
        assert!(counts.trees > 100, "{counts:?}");
        assert!(counts.buildings > 10, "{counts:?}");
        assert!(map.max_height() > 60);
        let sprites = scatter_sprites(&map, 30, params.seed);
        assert_eq!(sprites.len(), 30);
        assert!(sprites.iter().all(|s| map.is_dry(s.x as i32, s.y as i32)));
    }

    #[test]
    fn same_seed_same_map() {
        let params = CustomParams { cols: 40, rows: 30, ..CustomParams::default() };
        let a = generate_map(&params).expect("map");
        let b = generate_map(&params).expect("map");
        assert_eq!(a.heights(), b.heights());
        assert_eq!(a.decor_layer(), b.decor_layer());
        let c = generate_map(&CustomParams { seed: 8, ..params }).expect("map");
        assert_ne!(a.heights(), c.heights());
    }

    #[test]
    fn bad_params_are_refused() {
        assert!(generate_map(&CustomParams { cols: 1, ..CustomParams::default() }).is_err());
        assert!(generate_map(&CustomParams { sea_level: 1.5, ..CustomParams::default() }).is_err());
        assert!(generate_map(&CustomParams { relief: 0, ..CustomParams::default() }).is_err());
    }
}
