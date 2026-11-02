// --------------------------------------------------------------------------------------------------------------------
// The egui layer: menu bar, the tabbed sidebar, the two viewports and the dialogs.
//
// This draws widgets and reports what the user asked for. It does not touch the GPU or the
// world; app.rs acts on the UiOutput it hands back.

use crate::camera::{Camera, FOV_MAX, FOV_MIN};
use crate::custom_gen::CustomParams;
use crate::game_map::{Decor, Triangle, HEIGHT_MAX, TERRAIN_DIAGONAL};
use crate::renderer::{RendererUiOutcome, ViewportRenderer};
use crate::rng::NAMED_WORLD_SEEDS;
use crate::skybox::Skybox;
use crate::top_down::Camera as TopDownCamera;
use crate::view_options::{TopDownBase, ViewOptions};
use crate::world::World;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

/// What the user asked the app to do this frame.
#[derive(Clone, PartialEq, Debug)]
pub enum UiAction {
    NewStronghold(u32),
    NewStrongholdRandom,
    NewCustom(CustomParams),
    Quit,
}

/// The sidebar's tabs.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SidebarTab {
    View,
    Camera,
    Renderer,
}

/// UI state that lives across frames.
pub struct UiState {
    pub tab: SidebarTab,
    /// The new custom map dialog, open while Some.
    pub custom_dialog: Option<CustomParams>,
    /// The parameters the last custom map was made with, so the dialog reopens on them.
    pub last_custom: CustomParams,
    /// Something to tell the user, shown until dismissed.
    pub error: Option<String>,
    /// Where the two viewports were drawn last frame, in points; the targets are sized from these.
    pub top_down_rect: Option<Rect>,
    pub render_rect: Option<Rect>,
    pub hover_cell: Option<(i32, i32)>,
}

impl Default for UiState {
    fn default() -> UiState {
        UiState {
            tab: SidebarTab::View,
            custom_dialog: None,
            last_custom: CustomParams::default(),
            error: None,
            top_down_rect: None,
            render_rect: None,
            hover_cell: None,
        }
    }
}

/// The offscreen targets egui shows as images.
#[derive(Copy, Clone)]
pub struct Textures {
    pub top_down: egui::TextureId,
    pub render: egui::TextureId,
}

/// What the Renderer tab needs to know and may change.
pub struct RendererPanel<'a> {
    pub names: Vec<&'static str>,
    pub selected: usize,
    /// The container's palette entries, and which one the atlas is built under.
    pub palettes: &'a [usize],
    pub palette: usize,
    /// Where the sprites came from, or why they did not.
    pub dat_note: &'a str,
    pub renderer: &'a mut dyn ViewportRenderer,
    pub sky_names: Vec<&'static str>,
    pub sky_selected: usize,
    pub skybox: &'a mut dyn Skybox,
}

#[derive(Default, Debug)]
pub struct UiOutput {
    pub actions: Vec<UiAction>,
    /// A top-down option changed; the geometry wants rebuilding.
    pub options_changed: bool,
    /// A DEM setting changed and the user has let go; the DEM wants rebuilding.
    pub dem_changed: bool,
    /// The world height scale changed; the 3D renderers want rebuilding.
    pub height_scale_changed: bool,
    pub renderer_selected: Option<usize>,
    pub palette_selected: Option<usize>,
    pub renderer_outcome: Option<RendererUiOutcome>,
    pub sky_selected: Option<usize>,
    /// The skybox's own knobs changed.
    pub sky_changed: bool,
    pub camera_reset: bool,
}

/// The height of the title strip above each viewport, in points.
const VIEWPORT_TITLE_H: f32 = 20.0;
/// How much of the central panel's height the top-down map takes; the render viewport gets the rest.
const MAP_HEIGHT_SHARE: f32 = 0.4;

/// Draw the whole UI for one frame.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    ctx: &egui::Context,
    world: &World,
    options: &mut ViewOptions,
    camera: &mut Camera,
    panel: &mut RendererPanel,
    state: &mut UiState,
    textures: Textures,
    top_down_camera: TopDownCamera,
    pixels_per_point: f32,
) -> UiOutput {
    let mut out = UiOutput::default();
    draw_menu(ctx, state, &mut out);
    let renderer_name = panel.names.get(panel.selected).copied().unwrap_or("?");
    draw_sidebar(ctx, world, options, camera, panel, state, &mut out);
    draw_viewports(ctx, state, textures, top_down_camera, camera, renderer_name, pixels_per_point);
    draw_custom_dialog(ctx, state, &mut out);
    draw_error(ctx, state);
    out
}

// --------------------------------------------------------------------------------------------------------------------
// Menu bar

fn draw_menu(ctx: &egui::Context, state: &mut UiState, out: &mut UiOutput) {
    egui::TopBottomPanel::top("menu").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                ui.menu_button("New", |ui| {
                    if ui.button("Custom...").clicked() {
                        state.custom_dialog = Some(state.last_custom.clone());
                        ui.close_menu();
                    }
                    ui.menu_button("Stronghold world", |ui| {
                        for (i, seed) in NAMED_WORLD_SEEDS.iter().enumerate() {
                            if ui.button(format!("World {} (seed {seed})", i + 1)).clicked() {
                                out.actions.push(UiAction::NewStronghold(*seed));
                                ui.close_menu();
                            }
                        }
                        ui.separator();
                        if ui.button("Random (seeded from the clock)").clicked() {
                            out.actions.push(UiAction::NewStrongholdRandom);
                            ui.close_menu();
                        }
                    });
                });
                ui.separator();
                if ui.button("Quit").clicked() {
                    out.actions.push(UiAction::Quit);
                    ui.close_menu();
                }
            });
        });
    });
}

// --------------------------------------------------------------------------------------------------------------------
// Sidebar

fn draw_sidebar(ctx: &egui::Context, world: &World, options: &mut ViewOptions, camera: &mut Camera, panel: &mut RendererPanel, state: &mut UiState, out: &mut UiOutput) {
    egui::SidePanel::left("options").resizable(true).default_width(290.0).show(ctx, |ui| {
        let map = world.map();
        let counts = map.counts();
        ui.heading("World");
        ui.label(world.title());
        ui.label(format!("{} x {} cells, highest {}", map.cols(), map.rows(), map.max_height()));
        ui.label(format!("{} trees, {} buildings, {} units, {} water triangles", counts.trees, counts.buildings, world.sprites().len(), counts.water_triangles));
        ui.separator();
        ui.horizontal(|ui| {
            ui.selectable_value(&mut state.tab, SidebarTab::View, "View");
            ui.selectable_value(&mut state.tab, SidebarTab::Camera, "Camera");
            ui.selectable_value(&mut state.tab, SidebarTab::Renderer, "Renderer");
        });
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| match state.tab {
            SidebarTab::View => view_tab(ui, world, options, state, out),
            SidebarTab::Camera => camera_tab(ui, world, options, camera, out),
            SidebarTab::Renderer => renderer_tab(ui, panel, out),
        });
    });
}

fn view_tab(ui: &mut egui::Ui, world: &World, options: &mut ViewOptions, state: &mut UiState, out: &mut UiOutput) {
    ui.heading("Top-down view");
    let mut changed = false;
    ui.label("Base layer");
    changed |= ui.radio_value(&mut options.base, TopDownBase::Cells, "Game map cells").changed();
    changed |= ui.radio_value(&mut options.base, TopDownBase::Dem, "High resolution DEM").changed();
    ui.add_space(4.0);
    changed |= ui.checkbox(&mut options.show_water, "Water").changed();
    changed |= ui.checkbox(&mut options.show_trees, "Trees").changed();
    changed |= ui.checkbox(&mut options.show_buildings, "Buildings").changed();
    changed |= ui.checkbox(&mut options.show_sprites, "Units (sprites)").changed();
    ui.indent("orders", |ui| {
        ui.add_enabled_ui(options.show_sprites, |ui| {
            changed |= ui.checkbox(&mut options.show_sprite_orders, "Lines to destinations").changed();
        });
    });
    changed |= ui.checkbox(&mut options.show_grid, "Grid every four cells").changed();
    changed |= ui.checkbox(&mut options.cell_aspect_2_3, "Cells two wide by three tall").changed();
    out.options_changed |= changed;

    ui.separator();
    ui.heading("DEM");
    ui.label("Derived from the game map for rendering; play never reads it.");
    let mut factor = options.dem.factor as u32;
    let factor_response = ui.add(egui::Slider::new(&mut factor, 1..=16).text("samples per cell"));
    options.dem.factor = factor as usize;
    let detail_response = ui.add(egui::Slider::new(&mut options.dem.detail, 0.0..=1.5).text("added detail"));
    for response in [&factor_response, &detail_response] {
        // Rebuild when the slider is released, or on a click or typed value; not per drag frame.
        if response.drag_stopped() || (response.changed() && !response.dragged()) {
            out.dem_changed = true;
        }
    }
    let dem = world.dem();
    let (low, high) = dem.range();
    ui.label(format!("{} x {} samples, {low:.1} to {high:.1}", dem.cols(), dem.rows()));

    ui.separator();
    ui.heading("Under the cursor");
    match state.hover_cell {
        Some((x, y)) => describe_cell(ui, world, x, y),
        None => {
            ui.label("Move over the top-down view.");
        }
    }
}

fn camera_tab(ui: &mut egui::Ui, world: &World, options: &mut ViewOptions, camera: &mut Camera, out: &mut UiOutput) {
    ui.heading("Camera");
    ui.label(format!("At ({:.1}, {:.1}), heading {:.0} degrees", camera.x, camera.y, camera.yaw.to_degrees()));
    ui.label(format!("Eye at {:.1} height units", camera.eye_height_units(world.map())));
    if ui.button("Reset to the map centre").clicked() {
        out.camera_reset = true;
    }
    ui.add_space(6.0);
    ui.checkbox(&mut camera.follow_ground, "Follow the ground");
    ui.add_enabled_ui(camera.follow_ground, |ui| {
        ui.add(egui::Slider::new(&mut camera.eye_height, 0.5..=60.0).text("eye height above ground"));
    });
    ui.add_enabled_ui(!camera.follow_ground, |ui| {
        ui.add(egui::Slider::new(&mut camera.altitude, 1.0..=250.0).text("altitude"));
    });
    ui.add(egui::Slider::new(&mut camera.fov_degrees, FOV_MIN..=FOV_MAX).text("field of view"));
    ui.add(egui::Slider::new(&mut camera.move_speed, 1.0..=60.0).text("walk speed, cells/s"));
    ui.separator();
    ui.heading("Height scale");
    let response = ui.add(egui::Slider::new(&mut options.height_scale, 0.01..=0.3).logarithmic(true).text("cells per height unit"));
    if response.drag_stopped() || (response.changed() && !response.dragged()) {
        out.height_scale_changed = true;
    }
    ui.separator();
    ui.heading("Keys");
    ui.label("Click the render viewport first so the keys go to it, not to a text box.");
    ui.monospace("Up / Down      walk forward / back");
    ui.monospace("Left / Right   turn");
    ui.monospace("Q / E          strafe");
    ui.monospace("PgUp / PgDn    eye height or altitude");
    ui.monospace("+ / -          field of view");
    ui.monospace("Shift          walk faster");
}

fn renderer_tab(ui: &mut egui::Ui, panel: &mut RendererPanel, out: &mut UiOutput) {
    ui.heading("Renderer");
    let mut selected = panel.selected;
    egui::ComboBox::from_label("draws the viewport").selected_text(panel.names.get(selected).copied().unwrap_or("?")).show_ui(ui, |ui| {
        for (i, name) in panel.names.iter().enumerate() {
            ui.selectable_value(&mut selected, i, *name);
        }
    });
    if selected != panel.selected {
        out.renderer_selected = Some(selected);
    }
    ui.separator();
    ui.heading("Sprites");
    ui.label(panel.dat_note);
    if !panel.palettes.is_empty() {
        let mut palette = panel.palette;
        egui::ComboBox::from_label("palette entry").selected_text(format!("{palette}")).show_ui(ui, |ui| {
            for entry in panel.palettes {
                ui.selectable_value(&mut palette, *entry, format!("{entry}"));
            }
        });
        if palette != panel.palette {
            out.palette_selected = Some(palette);
        }
        ui.label("Which palette belongs to which sprite is not settled; pick what looks right.");
    }
    ui.separator();
    ui.heading("Sky");
    let mut sky = panel.sky_selected;
    egui::ComboBox::from_label("behind the terrain").selected_text(panel.sky_names.get(sky).copied().unwrap_or("?")).show_ui(ui, |ui| {
        for (i, name) in panel.sky_names.iter().enumerate() {
            ui.selectable_value(&mut sky, i, *name);
        }
    });
    if sky != panel.sky_selected {
        out.sky_selected = Some(sky);
    }
    out.sky_changed = panel.skybox.options_ui(ui);
    ui.separator();
    ui.heading(panel.names.get(panel.selected).copied().unwrap_or("Renderer"));
    out.renderer_outcome = Some(panel.renderer.options_ui(ui));
}

/// The facts about one cell, for the sidebar.
fn describe_cell(ui: &mut egui::Ui, world: &World, x: i32, y: i32) {
    let map = world.map();
    ui.label(format!("Cell ({x}, {y})"));
    let [h00, h10, h01, h11] = map.corner_heights(x, y);
    ui.label(format!("Corners {h00} {h10} / {h01} {h11} of {HEIGHT_MAX}"));
    if let (Some(ground), Some(word)) = (map.ground_at(x, y), map.terrain_at(x, y)) {
        let split = if word & TERRAIN_DIAGONAL != 0 { "rising" } else { "falling" };
        ui.label(format!("Ground A {} / B {}, split {split}", ground[Triangle::A as usize].name(), ground[Triangle::B as usize].name()));
        ui.label(format!("Terrain word {word:#06x}"));
    }
    match map.decor_at(x, y) {
        Some(Decor::Tree { kind }) => ui.label(format!("Tree, kind {kind:#x}")),
        Some(Decor::Building { kind, upgrade, condition }) => ui.label(format!("Building, kind {kind:#x}, upgrade {upgrade}, condition {condition}/8")),
        _ => ui.label("Nothing standing here"),
    };
    let here: Vec<String> = world
        .sprites()
        .iter()
        .filter(|s| s.x.floor() as i32 == x && s.y.floor() as i32 == y)
        .map(|s| format!("unit kind {} side {} -> ({}, {})", s.kind, s.side, s.dest_x, s.dest_y))
        .collect();
    for line in here {
        ui.label(line);
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Viewports

#[allow(clippy::too_many_arguments)]
fn draw_viewports(
    ctx: &egui::Context,
    state: &mut UiState,
    textures: Textures,
    camera: TopDownCamera,
    eye: &Camera,
    renderer_name: &str,
    pixels_per_point: f32,
) {
    egui::CentralPanel::default().frame(egui::Frame::NONE.inner_margin(6.0)).show(ctx, |ui| {
        // The render viewport on top with the larger share; the top-down map beneath it.
        let full = ui.available_rect_before_wrap();
        let gap = 8.0;
        let map_h = ((full.height() - gap) * MAP_HEIGHT_SHARE).max(1.0);
        let render_h = (full.height() - gap - map_h).max(1.0);
        let top = Rect::from_min_size(full.min, Vec2::new(full.width(), render_h));
        let bottom = Rect::from_min_size(Pos2::new(full.min.x, full.min.y + render_h + gap), Vec2::new(full.width(), map_h));

        let render = viewport(ui, top, &format!("Render viewport: {renderer_name}"), textures.render);
        // Clicking the viewport takes keyboard focus away from any text box, so keys reach the camera.
        let response = ui.interact(render, ui.id().with("render_focus"), Sense::click());
        if response.clicked() {
            ui.memory_mut(|m| m.request_focus(response.id));
        }
        state.render_rect = Some(render);

        let top_down = viewport(ui, bottom, "Top-down: the game map", textures.top_down);
        let response = ui.interact(top_down, ui.id().with("top_down_hover"), Sense::hover());
        state.hover_cell = response.hover_pos().and_then(|pos| {
            let px = (pos - top_down.min) * pixels_per_point;
            camera.cell_at(px.x, px.y)
        });
        state.top_down_rect = Some(top_down);
        draw_camera_marker(ui, top_down, camera, eye, pixels_per_point);
    });
}

/// How far the view cone reaches across the map, in cells.
const CAMERA_CONE_CELLS: f32 = 16.0;

/// The 3D camera drawn over the top-down map: a dot where it stands and a translucent wedge
/// spanning its field of view along its heading. Painted with egui so it costs no geometry.
fn draw_camera_marker(ui: &egui::Ui, viewport: Rect, camera: TopDownCamera, eye: &Camera, pixels_per_point: f32) {
    if camera.cell_w <= 0.0 || camera.cell_h <= 0.0 {
        return;
    }
    let to_screen = |x: f32, y: f32| {
        let (px, py) = camera.to_pixels(x, y);
        viewport.min + Vec2::new(px, py) / pixels_per_point
    };
    let half = eye.fov_degrees.clamp(FOV_MIN, FOV_MAX).to_radians() * 0.5;
    let stand = to_screen(eye.x, eye.y);
    // The wedge edges follow the map's own axes, so a stretched cell stretches the cone too.
    let mut wedge = vec![stand];
    let steps = 12;
    for i in 0..=steps {
        let angle = eye.yaw - half + (2.0 * half) * i as f32 / steps as f32;
        wedge.push(to_screen(eye.x + angle.cos() * CAMERA_CONE_CELLS, eye.y + angle.sin() * CAMERA_CONE_CELLS));
    }
    let painter = ui.painter().with_clip_rect(viewport);
    let fill = Color32::from_rgba_unmultiplied(255, 220, 60, 60);
    let edge = Stroke::new(1.0, Color32::from_rgb(255, 220, 60));
    painter.add(egui::Shape::convex_polygon(wedge, fill, edge));
    let [fx, fy] = eye.forward();
    painter.line_segment([stand, to_screen(eye.x + fx * CAMERA_CONE_CELLS, eye.y + fy * CAMERA_CONE_CELLS)], edge);
    painter.circle(stand, 4.0, Color32::from_rgb(255, 220, 60), Stroke::new(1.0, Color32::BLACK));
}

/// A titled frame showing one offscreen target at 1:1 pixels. Returns the image rect in points.
fn viewport(ui: &mut egui::Ui, rect: Rect, title: &str, texture: egui::TextureId) -> Rect {
    ui.painter().text(rect.left_top() + Vec2::new(4.0, 2.0), Align2::LEFT_TOP, title, FontId::proportional(13.0), Color32::from_gray(170));
    let image = Rect::from_min_max(Pos2::new(rect.min.x, rect.min.y + VIEWPORT_TITLE_H), rect.max);
    if image.width() >= 1.0 && image.height() >= 1.0 {
        ui.put(image, egui::Image::new(egui::load::SizedTexture::new(texture, image.size())));
        ui.painter().rect_stroke(image, 0.0, Stroke::new(1.0, Color32::from_gray(80)), StrokeKind::Inside);
    }
    image
}

// --------------------------------------------------------------------------------------------------------------------
// Dialogs

fn draw_custom_dialog(ctx: &egui::Context, state: &mut UiState, out: &mut UiOutput) {
    let Some(mut params) = state.custom_dialog.take() else { return };
    let mut open = true;
    let mut generate = false;
    let mut cancel = false;
    egui::Window::new("New custom map").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
        egui::Grid::new("custom_params").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            ui.label("Columns");
            ui.add(egui::DragValue::new(&mut params.cols).range(2..=1024));
            ui.end_row();
            ui.label("Rows");
            ui.add(egui::DragValue::new(&mut params.rows).range(2..=1024));
            ui.end_row();
            ui.label("Seed");
            ui.add(egui::DragValue::new(&mut params.seed));
            ui.end_row();
            ui.label("Sea level");
            ui.add(egui::Slider::new(&mut params.sea_level, 0.0..=0.9));
            ui.end_row();
            ui.label("Relief (highest ground)");
            ui.add(egui::Slider::new(&mut params.relief, 1..=HEIGHT_MAX));
            ui.end_row();
            ui.label("Roughness");
            ui.add(egui::Slider::new(&mut params.roughness, 0.0..=1.0));
            ui.end_row();
            ui.label("Feature size (cells)");
            ui.add(egui::Slider::new(&mut params.feature_size, 4.0..=200.0).logarithmic(true));
            ui.end_row();
            ui.label("Tree density");
            ui.add(egui::Slider::new(&mut params.tree_density, 0.0..=1.0));
            ui.end_row();
            ui.label("Buildings");
            ui.add(egui::DragValue::new(&mut params.building_count).range(0..=5000));
            ui.end_row();
            ui.label("Units");
            ui.add(egui::DragValue::new(&mut params.sprite_count).range(0..=5000));
            ui.end_row();
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button("Generate").clicked() {
                generate = true;
            }
            if ui.button("Cancel").clicked() {
                cancel = true;
            }
        });
    });
    if generate {
        state.last_custom = params.clone();
        out.actions.push(UiAction::NewCustom(params));
    } else if open && !cancel {
        state.custom_dialog = Some(params);
    }
}

fn draw_error(ctx: &egui::Context, state: &mut UiState) {
    let Some(message) = state.error.clone() else { return };
    let mut dismiss = false;
    egui::Window::new("Problem").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, Vec2::ZERO).show(ctx, |ui| {
        ui.label(message);
        if ui.button("OK").clicked() {
            dismiss = true;
        }
    });
    if dismiss {
        state.error = None;
    }
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests. The widgets need a window; what can be checked headless is checked here.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_state_starts_closed_and_clean() {
        let state = UiState::default();
        assert_eq!(state.tab, SidebarTab::View);
        assert!(state.custom_dialog.is_none());
        assert!(state.error.is_none());
        assert!(state.top_down_rect.is_none() && state.render_rect.is_none());
        assert_eq!(state.last_custom, CustomParams::default());
    }

    #[test]
    fn ui_output_starts_quiet() {
        let out = UiOutput::default();
        assert!(out.actions.is_empty() && !out.options_changed && !out.dem_changed && !out.height_scale_changed);
        assert!(out.renderer_selected.is_none() && out.palette_selected.is_none() && out.renderer_outcome.is_none());
    }
}
