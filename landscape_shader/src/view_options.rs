// --------------------------------------------------------------------------------------------------------------------
// What the sidebar controls: which layers the top-down view paints, and how the DEM is derived.

use crate::dem::DemSettings;

/// What the top-down view paints underneath the objects.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum TopDownBase {
    /// The game map's cells, two ground-typed triangles each.
    Cells,
    /// The derived high resolution DEM, shaded by height.
    Dem,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ViewOptions {
    pub base: TopDownBase,
    pub show_water: bool,
    pub show_trees: bool,
    pub show_buildings: bool,
    pub show_sprites: bool,
    /// A line from each moving unit to where it is heading.
    pub show_sprite_orders: bool,
    /// A line every four cells, the block size the original rounds positions to.
    pub show_grid: bool,
    /// Draw a cell two wide by three tall, as the original's map does, instead of square.
    pub cell_aspect_2_3: bool,
    /// World height scale for the 3D renderers and the camera: cells per height unit.
    pub height_scale: f32,
    pub dem: DemSettings,
}

impl Default for ViewOptions {
    fn default() -> ViewOptions {
        ViewOptions {
            base: TopDownBase::Cells,
            show_water: true,
            show_trees: true,
            show_buildings: true,
            show_sprites: true,
            show_sprite_orders: true,
            show_grid: false,
            cell_aspect_2_3: true,
            height_scale: 0.06,
            dem: DemSettings::default(),
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
    fn defaults_show_everything_but_the_grid() {
        let options = ViewOptions::default();
        assert!(options.show_water && options.show_trees && options.show_buildings && options.show_sprites);
        assert!(!options.show_grid);
        assert_eq!(options.base, TopDownBase::Cells);
    }
}
