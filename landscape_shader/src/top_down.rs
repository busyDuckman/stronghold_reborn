// --------------------------------------------------------------------------------------------------------------------
// The top-down view: the world as coloured triangles, in cell units, ready for a GPU pass.
//
// This is geometry only. It knows nothing about wgpu; it turns a world and the view options
// into a flat list of coloured vertices (positions in cells, x right, y down) plus, for the
// DEM base, an RGBA image. gpu.rs owns the buffers and the shader that draw them. Keeping the
// two apart is what makes this half testable without a GPU.

use crate::game_map::{Decor, GameMap, GroundType, Triangle, TERRAIN_WATER_A, TERRAIN_WATER_B};
use crate::dem::Dem;
use crate::sprite::Sprite;
use crate::view_options::{TopDownBase, ViewOptions};
use crate::world::World;
use bytemuck::{Pod, Zeroable};

/// One vertex: a position in cell units and a straight (linear) colour with alpha.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub colour: [f32; 4],
}

/// A triangle list in cell units.
#[derive(Default)]
pub struct Geometry {
    pub vertices: Vec<Vertex>,
}

impl Geometry {
    pub fn triangle_count(&self) -> usize {
        self.vertices.len() / 3
    }

    fn push_tri(&mut self, a: [f32; 2], b: [f32; 2], c: [f32; 2], colour: [f32; 4]) {
        self.vertices.push(Vertex { pos: a, colour });
        self.vertices.push(Vertex { pos: b, colour });
        self.vertices.push(Vertex { pos: c, colour });
    }

    fn push_quad(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, colour: [f32; 4]) {
        self.push_tri([x0, y0], [x1, y0], [x1, y1], colour);
        self.push_tri([x0, y0], [x1, y1], [x0, y1], colour);
    }

    /// A regular polygon fan around a centre.
    fn push_ngon(&mut self, cx: f32, cy: f32, r: f32, sides: u32, colour: [f32; 4]) {
        let step = std::f32::consts::TAU / sides.max(3) as f32;
        for i in 0..sides.max(3) {
            let a0 = i as f32 * step;
            let a1 = a0 + step;
            self.push_tri([cx, cy], [cx + r * a0.cos(), cy + r * a0.sin()], [cx + r * a1.cos(), cy + r * a1.sin()], colour);
        }
    }

    /// A line of a given width in cell units.
    fn push_line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, colour: [f32; 4]) {
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= f32::EPSILON {
            return;
        }
        let (nx, ny) = (-dy / len * width * 0.5, dx / len * width * 0.5);
        self.push_tri([x0 + nx, y0 + ny], [x1 + nx, y1 + ny], [x1 - nx, y1 - ny], colour);
        self.push_tri([x0 + nx, y0 + ny], [x1 - nx, y1 - ny], [x0 - nx, y0 - ny], colour);
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Colours

/// The base colour of a ground type, linear RGB.
pub fn ground_colour(ground: GroundType) -> [f32; 4] {
    match ground {
        GroundType::Water => [0.05, 0.18, 0.45, 1.0],
        GroundType::Sand => [0.62, 0.55, 0.30, 1.0],
        GroundType::Grass => [0.16, 0.38, 0.10, 1.0],
        GroundType::Scrub => [0.30, 0.36, 0.10, 1.0],
        GroundType::Rock => [0.34, 0.30, 0.26, 1.0],
        GroundType::Snow => [0.85, 0.87, 0.90, 1.0],
    }
}

/// A distinct colour per side, spread round the hue wheel; the same side is the same colour.
pub fn side_colour(side: u8) -> [f32; 4] {
    // Twenty four sides: eight hues, three brightness levels.
    let hue = (side % 8) as f32 / 8.0;
    let level = 1.0 - (side / 8 % 3) as f32 * 0.28;
    let [r, g, b] = hue_to_rgb(hue);
    [r * level, g * level, b * level, 1.0]
}

fn hue_to_rgb(hue: f32) -> [f32; 3] {
    let h = hue.rem_euclid(1.0) * 6.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    match h as u32 {
        0 => [1.0, x, 0.0],
        1 => [x, 1.0, 0.0],
        2 => [0.0, 1.0, x],
        3 => [0.0, x, 1.0],
        4 => [x, 0.0, 1.0],
        _ => [1.0, 0.0, x],
    }
}

fn scale(colour: [f32; 4], factor: f32) -> [f32; 4] {
    [colour[0] * factor, colour[1] * factor, colour[2] * factor, colour[3]]
}

const TREE_LOW: [f32; 4] = [0.08, 0.30, 0.06, 1.0];
const TREE_HIGH: [f32; 4] = [0.04, 0.20, 0.12, 1.0];
const BUILDING_NEW: [f32; 4] = [0.60, 0.42, 0.22, 1.0];
const BUILDING_RUIN: [f32; 4] = [0.18, 0.14, 0.12, 1.0];
const OUTLINE: [f32; 4] = [0.02, 0.02, 0.02, 1.0];
const GRID_LINE: [f32; 4] = [1.0, 1.0, 1.0, 0.18];

// --------------------------------------------------------------------------------------------------------------------
// Fitting the map into a viewport

/// How the map sits in a viewport: pixels per cell on each axis, and where cell (0, 0) starts.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct Camera {
    pub cell_w: f32,
    pub cell_h: f32,
    pub origin_x: f32,
    pub origin_y: f32,
    pub cols: usize,
    pub rows: usize,
}

impl Camera {
    /// Fit the whole map into a viewport, centred, keeping either square cells or the
    /// original's two-by-three cell.
    pub fn fit(cols: usize, rows: usize, viewport_w: f32, viewport_h: f32, aspect_2_3: bool) -> Camera {
        let aspect = if aspect_2_3 { 1.5 } else { 1.0 };
        let by_width = viewport_w / cols.max(1) as f32;
        let by_height = viewport_h / (rows.max(1) as f32 * aspect);
        let cell_w = by_width.min(by_height).max(0.0);
        let cell_h = cell_w * aspect;
        Camera {
            cell_w,
            cell_h,
            origin_x: ((viewport_w - cell_w * cols as f32) * 0.5).max(0.0),
            origin_y: ((viewport_h - cell_h * rows as f32) * 0.5).max(0.0),
            cols,
            rows,
        }
    }

    /// Where a point in cell coordinates lands in the viewport, in pixels.
    pub fn to_pixels(&self, x: f32, y: f32) -> (f32, f32) {
        (self.origin_x + x * self.cell_w, self.origin_y + y * self.cell_h)
    }

    /// The cell under a viewport pixel, or None off the map.
    pub fn cell_at(&self, px: f32, py: f32) -> Option<(i32, i32)> {
        if self.cell_w <= 0.0 || self.cell_h <= 0.0 {
            return None;
        }
        let x = ((px - self.origin_x) / self.cell_w).floor();
        let y = ((py - self.origin_y) / self.cell_h).floor();
        if x < 0.0 || y < 0.0 || x >= self.cols as f32 || y >= self.rows as f32 {
            return None;
        }
        Some((x as i32, y as i32))
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Building the geometry

/// Everything the top-down view draws for a world, under the options.
pub fn build_geometry(world: &World, options: &ViewOptions) -> Geometry {
    let mut geometry = Geometry::default();
    let map = world.map();
    if options.base == TopDownBase::Cells {
        push_cells(&mut geometry, map, options.show_water);
    }
    if options.show_grid {
        push_grid(&mut geometry, map);
    }
    if options.show_trees || options.show_buildings {
        push_decor(&mut geometry, map, options);
    }
    if options.show_sprites {
        push_sprites(&mut geometry, world.sprites(), options.show_sprite_orders);
    }
    geometry
}

/// Two ground-typed triangles per cell, shaded a little by the original's slope term.
fn push_cells(geometry: &mut Geometry, map: &GameMap, show_water: bool) {
    for y in 0..map.rows() as i32 {
        for x in 0..map.cols() as i32 {
            let Some(ground) = map.ground_at(x, y) else { continue };
            let Some(word) = map.terrain_at(x, y) else { continue };
            let rising = word & crate::game_map::TERRAIN_DIAGONAL != 0;
            let (fx, fy) = (x as f32, y as f32);
            let (tl, tr, bl, br) = ([fx, fy], [fx + 1.0, fy], [fx, fy + 1.0], [fx + 1.0, fy + 1.0]);
            for (tri, water_bit) in [(Triangle::A, TERRAIN_WATER_A), (Triangle::B, TERRAIN_WATER_B)] {
                let is_water = word & water_bit != 0;
                let [a, b, c] = map.triangle_corners(x, y, tri);
                // With water hidden the sea floor shows through as what the rule makes of it.
                let kind = if is_water && !show_water { GroundType::classify(a, b, c, false) } else { ground[tri as usize] };
                let (_, slope) = GroundType::shade_terms(a, b, c);
                let colour = scale(ground_colour(kind), 0.82 + 0.09 * slope as f32);
                let corners = match (tri, rising) {
                    (Triangle::A, true) => (tl, tr, bl),
                    (Triangle::B, true) => (bl, tr, br),
                    (Triangle::A, false) => (tl, br, bl),
                    (Triangle::B, false) => (tl, tr, br),
                };
                geometry.push_tri(corners.0, corners.1, corners.2, colour);
            }
        }
    }
}

/// A faint line every four cells, the block size the original rounds positions to.
fn push_grid(geometry: &mut Geometry, map: &GameMap) {
    let (cols, rows) = (map.cols() as f32, map.rows() as f32);
    let width = 0.08;
    for x in (0..=map.cols()).step_by(4) {
        geometry.push_line(x as f32, 0.0, x as f32, rows, width, GRID_LINE);
    }
    for y in (0..=map.rows()).step_by(4) {
        geometry.push_line(0.0, y as f32, cols, y as f32, width, GRID_LINE);
    }
}

/// Trees as small round blobs, buildings as squares sized by upgrade and faded by condition.
fn push_decor(geometry: &mut Geometry, map: &GameMap, options: &ViewOptions) {
    for y in 0..map.rows() as i32 {
        for x in 0..map.cols() as i32 {
            let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
            match map.decor_at(x, y) {
                Some(Decor::Tree { kind }) if options.show_trees => {
                    let colour = if kind >= 0x67 { TREE_HIGH } else { TREE_LOW };
                    geometry.push_ngon(cx, cy, 0.34, 8, colour);
                }
                Some(Decor::Building { upgrade, condition, .. }) if options.show_buildings => {
                    let half = 0.22 + 0.09 * upgrade.clamp(1, 3) as f32;
                    let t = condition.min(8) as f32 / 8.0;
                    let colour = [
                        BUILDING_RUIN[0] + (BUILDING_NEW[0] - BUILDING_RUIN[0]) * t,
                        BUILDING_RUIN[1] + (BUILDING_NEW[1] - BUILDING_RUIN[1]) * t,
                        BUILDING_RUIN[2] + (BUILDING_NEW[2] - BUILDING_RUIN[2]) * t,
                        1.0,
                    ];
                    geometry.push_quad(cx - half - 0.06, cy - half - 0.06, cx + half + 0.06, cy + half + 0.06, OUTLINE);
                    geometry.push_quad(cx - half, cy - half, cx + half, cy + half, colour);
                }
                _ => {}
            }
        }
    }
}

/// Units as diamonds in their side's colour, with a line to where they are heading.
fn push_sprites(geometry: &mut Geometry, sprites: &[Sprite], show_orders: bool) {
    for sprite in sprites {
        let (cx, cy) = (sprite.x + 0.5, sprite.y + 0.5);
        let colour = side_colour(sprite.side);
        if show_orders && sprite.is_moving() {
            let mut faint = colour;
            faint[3] = 0.55;
            geometry.push_line(cx, cy, sprite.dest_x + 0.5, sprite.dest_y + 0.5, 0.12, faint);
        }
        push_diamond(geometry, cx, cy, 0.52, OUTLINE);
        push_diamond(geometry, cx, cy, 0.40, colour);
    }
}

fn push_diamond(geometry: &mut Geometry, cx: f32, cy: f32, r: f32, colour: [f32; 4]) {
    geometry.push_tri([cx, cy - r], [cx + r, cy], [cx, cy + r], colour);
    geometry.push_tri([cx, cy - r], [cx, cy + r], [cx - r, cy], colour);
}

// --------------------------------------------------------------------------------------------------------------------
// The DEM as an image

/// The DEM as an RGBA8 image, one pixel per sample, height-ramped and hill-shaded.
pub fn dem_rgba(dem: &Dem) -> Vec<u8> {
    let (cols, rows) = (dem.cols(), dem.rows());
    let mut out = Vec::with_capacity(cols * rows * 4);
    let (_, high) = dem.range();
    let high = high.max(1.0);
    for j in 0..rows {
        for i in 0..cols {
            let h = dem.sample(i, j).unwrap_or(0.0);
            let base = if h <= 0.0 { ground_colour(GroundType::Water) } else { height_ramp(h / high) };
            // Light from the top-left: brighter where the ground rises towards it.
            let left = dem.sample(i.saturating_sub(1), j).unwrap_or(h);
            let right = dem.sample((i + 1).min(cols - 1), j).unwrap_or(h);
            let up = dem.sample(i, j.saturating_sub(1)).unwrap_or(h);
            let down = dem.sample(i, (j + 1).min(rows - 1)).unwrap_or(h);
            let shade = if h <= 0.0 { 1.0 } else { (1.0 + ((left - right) + (up - down)) * 0.06).clamp(0.55, 1.45) };
            for channel in 0..3 {
                out.push(to_srgb_byte(base[channel] * shade));
            }
            out.push(255);
        }
    }
    out
}

/// Sand at the shore through grass and rock to snow, in linear RGB.
fn height_ramp(t: f32) -> [f32; 4] {
    let stops: [(f32, [f32; 4]); 5] = [
        (0.0, ground_colour(GroundType::Sand)),
        (0.15, ground_colour(GroundType::Grass)),
        (0.55, ground_colour(GroundType::Scrub)),
        (0.8, ground_colour(GroundType::Rock)),
        (1.0, ground_colour(GroundType::Snow)),
    ];
    let t = t.clamp(0.0, 1.0);
    for pair in stops.windows(2) {
        let (t0, c0) = pair[0];
        let (t1, c1) = pair[1];
        if t <= t1 {
            let f = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
            return [c0[0] + (c1[0] - c0[0]) * f, c0[1] + (c1[1] - c0[1]) * f, c0[2] + (c1[2] - c0[2]) * f, 1.0];
        }
    }
    ground_colour(GroundType::Snow)
}

/// Linear to an 8-bit sRGB value, for an sRGB texture.
fn to_srgb_byte(linear: f32) -> u8 {
    let l = linear.clamp(0.0, 1.0);
    let s = if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::custom_gen::CustomParams;
    use crate::dem::DemSettings;

    fn small_world() -> World {
        let params = CustomParams { cols: 24, rows: 16, sprite_count: 6, building_count: 3, ..CustomParams::default() };
        World::custom(&params, DemSettings { factor: 2, ..DemSettings::default() }).expect("world")
    }

    #[test]
    fn cells_make_two_triangles_each_and_toggles_change_the_count() {
        let world = small_world();
        let base = ViewOptions { show_trees: false, show_buildings: false, show_sprites: false, ..ViewOptions::default() };
        let cells_only = build_geometry(&world, &base);
        assert_eq!(cells_only.triangle_count(), 24 * 16 * 2);
        let everything = build_geometry(&world, &ViewOptions::default());
        assert!(everything.triangle_count() > cells_only.triangle_count());
        let dem_base = build_geometry(&world, &ViewOptions { base: TopDownBase::Dem, ..base });
        assert_eq!(dem_base.triangle_count(), 0);
        let with_grid = build_geometry(&world, &ViewOptions { show_grid: true, ..base });
        assert!(with_grid.triangle_count() > cells_only.triangle_count());
        assert_eq!(everything.vertices.len() % 3, 0);
    }

    #[test]
    fn camera_fit_keeps_the_aspect_and_inverts() {
        let camera = Camera::fit(148, 60, 592.0, 600.0, true);
        assert!((camera.cell_w - 4.0).abs() < 1e-4);
        assert!((camera.cell_h - 6.0).abs() < 1e-4);
        assert_eq!(camera.origin_x, 0.0);
        assert!((camera.origin_y - 120.0).abs() < 1e-4);
        assert_eq!(camera.cell_at(0.0, 120.0), Some((0, 0)));
        assert_eq!(camera.cell_at(591.9, 479.9), Some((147, 59)));
        assert_eq!(camera.cell_at(0.0, 0.0), None);
        assert_eq!(camera.cell_at(600.0, 300.0), None);
        let (px, py) = camera.to_pixels(74.0, 30.0);
        assert!((px - 296.0).abs() < 1e-4 && (py - 300.0).abs() < 1e-4);
        assert_eq!(camera.cell_at(px, py), Some((74, 30)));
        let square = Camera::fit(10, 10, 100.0, 50.0, false);
        assert!((square.cell_w - 5.0).abs() < 1e-4 && (square.cell_h - 5.0).abs() < 1e-4);
    }

    #[test]
    fn dem_image_has_one_pixel_per_sample() {
        let world = small_world();
        let rgba = dem_rgba(world.dem());
        assert_eq!(rgba.len(), 48 * 32 * 4);
        assert!(rgba.chunks(4).all(|p| p[3] == 255));
    }

    #[test]
    fn side_colours_are_distinct_for_a_full_side_count() {
        let mut seen: Vec<[u8; 3]> = Vec::new();
        for side in 0..24u8 {
            let c = side_colour(side);
            let key = [to_srgb_byte(c[0]), to_srgb_byte(c[1]), to_srgb_byte(c[2])];
            assert!(!seen.contains(&key), "side {side} repeats a colour");
            seen.push(key);
        }
        assert_eq!(to_srgb_byte(0.0), 0);
        assert_eq!(to_srgb_byte(1.0), 255);
    }
}
