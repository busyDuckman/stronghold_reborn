// --------------------------------------------------------------------------------------------------------------------
// The world: one game map, the units on it, and the DEM derived from it. No simulation.

use crate::custom_gen::{generate_map, scatter_sprites, CustomParams};
use crate::dem::{Dem, DemSettings};
use crate::game_map::GameMap;
use crate::sprite::Sprite;
use crate::stronghold_gen::StrongholdRoll;
use anyhow::{bail, Result};

/// Where a world came from, for the title and for rerolling.
#[derive(Clone, PartialEq, Debug)]
pub enum WorldSource {
    /// The original game's generator with one of its seeds.
    Stronghold { seed: u32 },
    /// The custom generator.
    Custom(CustomParams),
}

/// How many units a Stronghold world gets scattered onto it. The original's generator leaves
/// the unit table empty (its spawner fills it later and is not ported), so this stands in.
pub const STRONGHOLD_SPRITE_COUNT: usize = 46;

pub struct World {
    map: GameMap,
    sprites: Vec<Sprite>,
    dem: Dem,
    source: WorldSource,
}

impl std::fmt::Debug for World {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "World({:?}, {} sprites, {:?})", self.map, self.sprites.len(), self.dem)
    }
}

impl World {
    /// Put a world together, checking every unit stands on the map.
    pub fn assemble(map: GameMap, sprites: Vec<Sprite>, source: WorldSource, dem: DemSettings) -> Result<World> {
        for sprite in &sprites {
            if map.index(sprite.x.floor() as i32, sprite.y.floor() as i32).is_none() {
                bail!("a unit at ({}, {}) is off a {}x{} map", sprite.x, sprite.y, map.cols(), map.rows());
            }
        }
        let dem = Dem::from_game_map(&map, dem)?;
        Ok(World { map, sprites, dem, source })
    }

    /// The original game's world for a seed, with stand-in units scattered on it.
    pub fn stronghold(seed: u32, dem: DemSettings) -> Result<World> {
        let map = StrongholdRoll::roll(seed).into_game_map()?;
        let sprites = scatter_sprites(&map, STRONGHOLD_SPRITE_COUNT, seed as u64);
        World::assemble(map, sprites, WorldSource::Stronghold { seed }, dem)
    }

    /// A custom world.
    pub fn custom(params: &CustomParams, dem: DemSettings) -> Result<World> {
        let map = generate_map(params)?;
        let sprites = scatter_sprites(&map, params.sprite_count, params.seed);
        World::assemble(map, sprites, WorldSource::Custom(params.clone()), dem)
    }

    pub fn map(&self) -> &GameMap {
        &self.map
    }

    pub fn sprites(&self) -> &[Sprite] {
        &self.sprites
    }

    pub fn dem(&self) -> &Dem {
        &self.dem
    }

    pub fn source(&self) -> &WorldSource {
        &self.source
    }

    /// Derive the DEM again with new settings; the map and units are untouched.
    pub fn rebuild_dem(&mut self, settings: DemSettings) -> Result<()> {
        self.dem = Dem::from_game_map(&self.map, settings)?;
        Ok(())
    }

    /// A one-line description for the window and the sidebar.
    pub fn title(&self) -> String {
        match &self.source {
            WorldSource::Stronghold { seed } => format!("Stronghold world, seed {seed}"),
            WorldSource::Custom(params) => format!("Custom {}x{}, seed {}", params.cols, params.rows, params.seed),
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

    #[test]
    fn a_stronghold_world_assembles() {
        let world = World::stronghold(100, DemSettings { factor: 2, ..DemSettings::default() }).expect("world");
        assert_eq!(world.sprites().len(), STRONGHOLD_SPRITE_COUNT);
        assert_eq!((world.dem().cols(), world.dem().rows()), (296, 120));
        assert_eq!(world.title(), "Stronghold world, seed 100");
    }

    #[test]
    fn a_custom_world_assembles_and_rebuilds_its_dem() {
        let params = CustomParams { cols: 30, rows: 20, sprite_count: 5, ..CustomParams::default() };
        let mut world = World::custom(&params, DemSettings { factor: 1, ..DemSettings::default() }).expect("world");
        assert_eq!(world.dem().factor(), 1);
        world.rebuild_dem(DemSettings { factor: 3, ..DemSettings::default() }).expect("rebuild");
        assert_eq!((world.dem().cols(), world.dem().rows()), (90, 60));
    }

    #[test]
    fn units_off_the_map_are_refused() {
        let map = GameMap::new_flat(4, 4).expect("map");
        let sprites = vec![Sprite::standing(9.0, 0.0, 0, 0)];
        let result = World::assemble(map, sprites, WorldSource::Stronghold { seed: 1 }, DemSettings::default());
        assert!(result.is_err());
    }
}
