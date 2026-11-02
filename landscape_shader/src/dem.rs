// --------------------------------------------------------------------------------------------------------------------
// The high resolution DEM: a finer height field derived from the game map, for rendering only.
//
// The game map's heights are one per cell corner, which is what play needs and far too
// coarse to draw a landscape from. This takes that corner grid, resamples it factor times
// finer, and adds fractal (Brownian) detail scaled by how much the ground moves locally, so
// slopes get texture and flats and water stay flat. Nothing reads it back into play.

use crate::game_map::GameMap;
use crate::noise::fbm;
use anyhow::{bail, Result};

/// How the DEM is derived. Kept beside the view options so the sidebar can change it.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct DemSettings {
    /// Samples per cell along each axis, 1 ..= 16.
    pub factor: usize,
    /// How strong the added detail is, in height units per unit of local relief; 0 is none.
    pub detail: f32,
    /// Seed for the detail noise.
    pub seed: u64,
}

impl Default for DemSettings {
    fn default() -> DemSettings {
        DemSettings { factor: 4, detail: 0.35, seed: 1 }
    }
}

/// The largest factor allowed; 148 x 60 x 16 x 16 is 2.3 million samples, plenty.
pub const DEM_FACTOR_MAX: usize = 16;

/// A height field `factor` times finer than the game map, in the game map's height units.
pub struct Dem {
    cols: usize,
    rows: usize,
    factor: usize,
    heights: Vec<f32>,
}

impl std::fmt::Debug for Dem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Dem {}x{} (x{})", self.cols, self.rows, self.factor)
    }
}

impl Dem {
    /// Derive from a game map. Sample (i, j) sits at corner-grid position (i, j) / factor.
    pub fn from_game_map(map: &GameMap, settings: DemSettings) -> Result<Dem> {
        if settings.factor == 0 || settings.factor > DEM_FACTOR_MAX {
            bail!("DEM factor {} is outside 1..={DEM_FACTOR_MAX}", settings.factor);
        }
        if !settings.detail.is_finite() || settings.detail < 0.0 {
            bail!("DEM detail {} must be a non-negative number", settings.detail);
        }
        let cols = map.cols() * settings.factor;
        let rows = map.rows() * settings.factor;
        let mut heights = Vec::with_capacity(cols * rows);
        let inv = 1.0 / settings.factor as f32;
        for j in 0..rows {
            for i in 0..cols {
                let (u, v) = (i as f32 * inv, j as f32 * inv);
                heights.push(Dem::sample_map(map, u, v, settings));
            }
        }
        Ok(Dem { cols, rows, factor: settings.factor, heights })
    }

    /// One sample: the corner heights around (u, v) blended, plus detail scaled by their spread.
    fn sample_map(map: &GameMap, u: f32, v: f32, settings: DemSettings) -> f32 {
        let (x0, y0) = (u.floor() as i32, v.floor() as i32);
        let (tx, ty) = (u - x0 as f32, v - y0 as f32);
        let corners = map.corner_heights(x0, y0);
        let [h00, h10, h01, h11] = corners.map(|h| h as f32);
        let top = h00 + (h10 - h00) * tx;
        let bottom = h01 + (h11 - h01) * tx;
        let base = top + (bottom - top) * ty;
        // Water and any patch enclosed by sea-level corners stays exactly flat.
        if corners == [0, 0, 0, 0] {
            return 0.0;
        }
        let relief = (corners.iter().copied().max().unwrap_or(0) - corners.iter().copied().min().unwrap_or(0)) as f32;
        let noise = fbm(u * 2.0, v * 2.0, settings.seed, 4, 0.5) - 0.5;
        let detail = noise * settings.detail * (relief + 1.0);
        (base + detail).max(0.0)
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn factor(&self) -> usize {
        self.factor
    }

    pub fn heights(&self) -> &[f32] {
        &self.heights
    }

    /// The sample at (i, j), or None outside the field.
    pub fn sample(&self, i: usize, j: usize) -> Option<f32> {
        if i >= self.cols || j >= self.rows {
            return None;
        }
        self.heights.get(j * self.cols + i).copied()
    }

    /// Lowest and highest sample.
    pub fn range(&self) -> (f32, f32) {
        self.heights.iter().fold((f32::MAX, f32::MIN), |(lo, hi), h| (lo.min(*h), hi.max(*h)))
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    fn stepped_map() -> GameMap {
        let mut map = GameMap::new_flat(6, 4).expect("map");
        for y in 0..4 {
            for x in 2..6 {
                map.set_height(x, y, 40).expect("in range");
            }
        }
        map.mark_diagonals_and_water();
        map.refresh_ground();
        map
    }

    #[test]
    fn dem_is_factor_times_finer_and_water_stays_flat() {
        let dem = Dem::from_game_map(&stepped_map(), DemSettings { factor: 4, detail: 0.5, seed: 3 }).expect("dem");
        assert_eq!((dem.cols(), dem.rows()), (24, 16));
        // Cell (0, 0) has four sea-level corners: every sample inside it is exactly zero.
        for j in 0..4 {
            for i in 0..4 {
                assert_eq!(dem.sample(i, j), Some(0.0));
            }
        }
        // Samples on the plateau sit near 40 with detail on top, never below zero.
        let on_plateau = dem.sample(16, 8).expect("in range");
        assert!((on_plateau - 40.0).abs() < 6.0, "{on_plateau}");
        assert!(dem.heights().iter().all(|h| *h >= 0.0));
        assert!(dem.sample(24, 0).is_none());
    }

    #[test]
    fn no_detail_means_plain_interpolation() {
        let dem = Dem::from_game_map(&stepped_map(), DemSettings { factor: 2, detail: 0.0, seed: 3 }).expect("dem");
        // Halfway across the step from corner 1 (0) to corner 2 (40) is 20.
        assert_eq!(dem.sample(3, 0), Some(20.0));
        assert_eq!(dem.sample(4, 0), Some(40.0));
    }

    #[test]
    fn settings_are_checked() {
        let map = stepped_map();
        assert!(Dem::from_game_map(&map, DemSettings { factor: 0, detail: 0.0, seed: 0 }).is_err());
        assert!(Dem::from_game_map(&map, DemSettings { factor: 17, detail: 0.0, seed: 0 }).is_err());
        assert!(Dem::from_game_map(&map, DemSettings { factor: 2, detail: -1.0, seed: 0 }).is_err());
    }
}
