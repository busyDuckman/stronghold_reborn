// --------------------------------------------------------------------------------------------------------------------
// The unified editor: the same window as the paired one, over a different backend.
//
// Left panel: what you are looking at, then a control for every member of every uniform block,
// generated from the block's own comments. Right: the preview. Bottom: what naga said.
//
// The render happens on demand rather than every frame, because a unified shader is usually a
// bake: terrain.glsl evaluates seven terrain types per pixel and three times over for the
// normal, so there is no sense redrawing it while nothing has changed. Anything that could
// change the picture sets the dirty flag, and a shader with a moving uniform marks itself dirty
// every frame, which is the only case that wants the old behaviour.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use egui::Color32;
use macroquad::prelude::*;

use crate::controls::{value_row, Driven, Motion, TUNING_COL};
use crate::dialect::{detect, Dialect};
use crate::glsl_meta::{ty_name, GLSLType, Hint};
use crate::mq_helpers::draw_checkerboard;
use crate::unified::composite::{ViewMode, ViewParams};
use crate::unified::gpu::{Gpu, Scene};
use crate::unified::pack::leaf_specs;
use crate::unified::parse::{parse, Unified};
use crate::unified::roles::{Role, ALL_ROLES};
use crate::unified::tune::{parse_tune, seed_tune_values, write_tune_values, TuneAssign};
use crate::values::UniformValues;

/// One control row, copied out of the shader so the panel can borrow values and motion freely.
struct Row {
    key: String,
    label: String,
    section: Option<String>,
    ty: GLSLType,
    hint: Hint,
    tuned: bool,
}

pub struct UnifiedApp {
    path: PathBuf,
    stamp: Option<SystemTime>,
    src: String,
    shader: Unified,
    tune: Vec<TuneAssign>,
    tuned: HashSet<String>,
    // a define you flipped in the UI has to survive a reparse, or the checkbox would spring
    // back to what the file says every time the shader recompiles
    toggle_state: Vec<(String, bool)>,
    values: UniformValues,
    motion: Motion,

    gpu: Option<Gpu>,
    scene: Option<Scene>,
    log: String,
    source_ok: bool,

    view: ViewParams,
    bake_px: (u32, u32),
    paused_global: bool,
    dirty: bool,
    preview: Option<Texture2D>,
    filter: String,

    ui_left_w: f32,
    ui_bottom_h: f32,
}

impl UnifiedApp {
    pub fn new(path: &Path) -> Result<Self, String> {
        let src = fs::read_to_string(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        // a device is optional: without one there is no preview, but the metadata and the
        // compile check are still worth having, so say so rather than refusing to open
        let (gpu, gpu_err) = match Gpu::new() {
            Ok(g) => (Some(g), None),
            Err(e) => (None, Some(e)),
        };

        let mut app = Self {
            path: path.to_path_buf(),
            stamp: fs::metadata(path).and_then(|m| m.modified()).ok(),
            src,
            shader: Unified::default(),
            tune: Vec::new(),
            tuned: HashSet::new(),
            toggle_state: Vec::new(),
            values: UniformValues::default(),
            motion: Motion::default(),
            gpu,
            scene: None,
            log: String::new(),
            source_ok: false,
            view: ViewParams::default(),
            bake_px: (512, 512),
            paused_global: false,
            dirty: true,
            preview: None,
            filter: String::new(),
            ui_left_w: 0.0,
            ui_bottom_h: 0.0,
        };
        app.rebuild(gpu_err);
        Ok(app)
    }

    /// Reparse, recompile and reseed. Keeps the current values, so tuning survives an edit.
    fn rebuild(&mut self, extra: Option<String>) {
        let detection = detect(&self.src);
        self.shader = parse(&self.src);
        for t in self.shader.toggles.iter_mut() {
            if let Some((_, on)) = self.toggle_state.iter().find(|(d, _)| d == &t.define) { t.on = *on; }
        }
        let (tune, tune_errs) = parse_tune(&self.src);
        self.tune = tune;
        self.tuned = self.tune.iter().map(|a| a.key.clone()).collect();

        let mut problems = Vec::new();
        if detection.dialect != Dialect::Unified {
            problems.push(format!("read as {} ({})", detection.dialect.name(), detection.because));
        }
        problems.extend(detection.errors.clone());
        problems.extend(self.shader.errors.clone());
        problems.extend(tune_errs);

        // the tune values are the shader's own defaults, so they are seeded before the hints
        problems.extend(seed_tune_values(&mut self.values, &self.tune, &self.shader));
        let specs = leaf_specs(&self.shader);
        self.values.set_defaults(&specs, vec2(self.bake_px.0 as f32, self.bake_px.1 as f32));
        let driven: Vec<Driven> = specs.iter()
            .map(|s| Driven { key: &s.name, ty: s.ty, hint: &s.hint })
            .collect();
        self.motion.rebuild(&driven, &mut self.values, get_time());

        self.scene = None;
        self.source_ok = false;
        match &self.gpu {
            None => problems.push(extra.unwrap_or_else(|| "no GPU device".to_string())),
            Some(gpu) => match Scene::new(gpu, &self.shader, &self.src) {
                Ok(scene) => {
                    if scene.used_builtin_vertex {
                        problems.push("no vertex stage in the file; using the built-in fullscreen triangle".to_string());
                    }
                    self.scene = Some(scene);
                    self.source_ok = true;
                }
                Err(e) => problems.push(e),
            },
        }
        self.log = compose_log(&self.shader, &problems);
        self.dirty = true;
    }

    pub fn poll_reload(&mut self) {
        let now = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if now == self.stamp { return; }
        self.stamp = now;
        match fs::read_to_string(&self.path) {
            Ok(s) => { self.src = s; self.rebuild(None); }
            Err(e) => self.log = format!("cannot read {}: {}", self.path.display(), e),
        }
    }

    pub fn update_animated_uniforms(&mut self) {
        if !self.motion.is_running(self.paused_global) { return; }
        self.motion.update(&mut self.values, self.paused_global, get_time());
        self.dirty = true;
    }

    /// Render if anything changed, then blit the last picture into the free area.
    pub fn draw_scene(&mut self) {
        if self.dirty {
            self.render_now();
            self.dirty = false;
        }

        let free_x = self.ui_left_w.max(0.0);
        let free_w = (screen_width() - self.ui_left_w).max(1.0);
        let free_h = (screen_height() - self.ui_bottom_h).max(1.0);

        let Some(tex) = self.preview.clone() else { return };
        // fit the bake into the free area without distorting it: a texture judged at the wrong
        // aspect is a texture judged wrongly
        let scale = (free_w / tex.width()).min(free_h / tex.height()).min(4.0);
        let size = vec2(tex.width() * scale, tex.height() * scale);
        let pos = vec2(free_x + (free_w - size.x) * 0.5, (free_h - size.y) * 0.5);

        gl_use_default_material();
        draw_checkerboard(pos, size, 16.0);
        draw_texture_ex(&tex, pos.x, pos.y, WHITE,
            DrawTextureParams { dest_size: Some(size), ..Default::default() });
    }

    fn render_now(&mut self) {
        let (Some(gpu), Some(scene)) = (self.gpu.as_ref(), self.scene.as_mut()) else { return };
        let (w, h) = self.bake_px;
        match scene.render(gpu, &self.values, &self.view, w, h) {
            Ok(frame) => {
                // row 0 first, which is how the view pass wrote it, so preview and PNG agree
                let image = Image { bytes: frame.rgba, width: frame.width as u16, height: frame.height as u16 };
                self.preview = Some(Texture2D::from_image(&image));
            }
            Err(e) => {
                self.source_ok = false;
                self.log = format!("RENDER FAILED: {}", e);
            }
        }
    }

    /// Forget every tweak, back to the tune values and the default= hints.
    fn reset_params(&mut self) {
        self.values = UniformValues::default();
        seed_tune_values(&mut self.values, &self.tune, &self.shader);
        let specs = leaf_specs(&self.shader);
        self.values.set_defaults(&specs, vec2(self.bake_px.0 as f32, self.bake_px.1 as f32));
        let driven: Vec<Driven> = specs.iter()
            .map(|s| Driven { key: &s.name, ty: s.ty, hint: &s.hint })
            .collect();
        self.motion.rebuild(&driven, &mut self.values, get_time());
        self.dirty = true;
    }

    /// Write the live values onto the tune assignments, then reload from what landed on disk.
    fn save_tuning(&mut self) {
        let result = write_tune_values(&self.src, &self.tune, &self.shader, &self.values)
            .and_then(|(out, n)| {
                fs::write(&self.path, &out).map_err(|e| format!("write failed: {e}"))?;
                Ok((out, n))
            });
        match result {
            Ok((out, n)) => {
                self.src = out;
                // refresh the stamp so the poll does not reload a second time
                self.stamp = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
                self.rebuild(None);
                self.log.push_str(&format!("\nSaved {} tuning value(s) to {}.", n, self.path.display()));
            }
            Err(e) => self.log = format!("SAVE FAILED: {e}"),
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// The panels.

impl UnifiedApp {
    pub fn ui(&mut self) {
        egui_macroquad::ui(|ctx| {
            let left = egui::SidePanel::left("settings").resizable(true).default_width(400.0).show(ctx, |ui| {
                ui.heading("View");
                ui.separator();
                self.ui_view(ui);
                ui.separator();
                self.ui_global(ui);
                ui.separator();
                ui.heading("Uniform blocks");
                ui.horizontal(|ui| {
                    ui.label("filter");
                    ui.text_edit_singleline(&mut self.filter);
                    if ui.button("x").clicked() { self.filter.clear(); }
                });
                // a G-buffer shader can easily run to a hundred members, so the list scrolls
                // and the filter is the way anyone finds one of them twice
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| self.ui_blocks(ui));
            });
            self.ui_left_w = left.response.rect.width();

            let bottom = egui::TopBottomPanel::bottom("output").resizable(true).default_height(160.0).show(ctx, |ui| {
                ui.heading("Output");
                ui.separator();
                if self.source_ok {
                    ui.colored_label(TUNING_COL, "Shader OK");
                } else {
                    ui.colored_label(Color32::from_rgb(220, 40, 40), "Shader Error");
                }
                ui.separator();
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
                    ui.label(&self.log);
                });
            });
            self.ui_bottom_h = bottom.response.rect.height();
        });
        egui_macroquad::draw();
    }

    /// Which channel is on screen, how it is lit, and how big the bake is.
    fn ui_view(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            changed |= ui.selectable_value(&mut self.view.mode, ViewMode::Lit, "Lit").clicked();
            // one button per role the shader actually declared, so the buttons cannot lie
            for role in ALL_ROLES.iter().filter(|r| self.shader.roles.contains_key(r)) {
                changed |= ui.selectable_value(&mut self.view.mode, ViewMode::Channel(*role), role.name()).clicked();
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("raw");
            for out in &self.shader.outputs {
                let label = format!("{}: {}", out.location, out.name);
                changed |= ui.selectable_value(&mut self.view.mode,
                    ViewMode::Raw(out.location as usize), label).clicked();
            }
        });

        if matches!(self.view.mode, ViewMode::Lit) {
            ui.horizontal(|ui| {
                ui.label("light");
                changed |= ui.add(egui::DragValue::new(&mut self.view.azimuth_deg)
                    .speed(1.0).suffix(" az")).changed();
                changed |= ui.add(egui::DragValue::new(&mut self.view.elevation_deg)
                    .speed(1.0).range(1.0..=89.0).suffix(" el")).changed();
            });
            ui.horizontal(|ui| {
                ui.label("ambient");
                changed |= ui.add(egui::Slider::new(&mut self.view.ambient, 0.0..=1.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("exposure");
                changed |= ui.add(egui::Slider::new(&mut self.view.exposure, 0.1..=4.0)).changed();
            });
        }

        ui.horizontal(|ui| {
            ui.label("tiles");
            // a tiling bake that does not join itself is only visible at 2x2 or more
            for n in [1.0f32, 2.0, 4.0] {
                changed |= ui.selectable_value(&mut self.view.tiles, n, format!("{}x{}", n, n)).clicked();
            }
        });
        ui.horizontal(|ui| {
            ui.label("bake px");
            let mut w = self.bake_px.0 as i32;
            let mut h = self.bake_px.1 as i32;
            let a = ui.add(egui::DragValue::new(&mut w).range(16..=4096)).changed();
            let b = ui.add(egui::DragValue::new(&mut h).range(16..=4096)).changed();
            if a || b {
                self.bake_px = (w.max(16) as u32, h.max(16) as u32);
                changed = true;
            }
            for n in [256u32, 512, 1024] {
                if ui.button(format!("{}", n)).clicked() { self.bake_px = (n, n); changed = true; }
            }
        });
        if changed { self.dirty = true; }
    }

    fn ui_global(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if self.motion.is_running(false) {
                if ui.button(if self.paused_global { "Play" } else { "Pause" }).clicked() {
                    self.paused_global = !self.paused_global;
                }
            }
            if ui.button("Reload").on_hover_text("Recompile from disk, keeping the current values").clicked() {
                self.stamp = None;
                self.poll_reload();
            }
            if ui.button("Reset params")
                .on_hover_text("Back to the shader's own tune values, then its default= hints")
                .clicked()
            {
                self.reset_params();
            }
            let has_tune = !self.tune.is_empty();
            if ui.add_enabled(has_tune, egui::Button::new("Save tuning"))
                .on_hover_text("Write the live values onto the shader's !(tune=) assignments")
                .on_disabled_hover_text("No !(tune=...) region in this shader")
                .clicked()
            {
                self.save_tuning();
            }
            if ui.button("Re-render").clicked() { self.dirty = true; }
        });

        if self.shader.toggles.is_empty() { return; }
        ui.separator();
        ui.label("Defines");
        let mut flipped = None;
        for (i, t) in self.shader.toggles.iter().enumerate() {
            let mut on = t.on;
            if ui.checkbox(&mut on, format!("{}  ({})", t.label, t.define))
                .on_hover_text("Recompiles, the way the host's build would")
                .changed()
            {
                flipped = Some((i, on));
            }
        }
        // a define changes what compiles, so flipping one goes through the whole rebuild
        if let Some((i, on)) = flipped {
            let define = self.shader.toggles[i].define.clone();
            match self.toggle_state.iter_mut().find(|(d, _)| d == &define) {
                Some(entry) => entry.1 = on,
                None => self.toggle_state.push((define, on)),
            }
            self.rebuild(None);
        }
    }

    /// Every block's members, grouped as the shader grouped them.
    fn ui_blocks(&mut self, ui: &mut egui::Ui) {
        let needle = self.filter.trim().to_ascii_lowercase();
        let mut edited = false;
        for bi in 0..self.shader.blocks.len() {
            let block = &self.shader.blocks[bi];
            let title = format!("{} (binding {}, {} bytes)", block.instance, block.binding, block.size);
            let rows: Vec<Row> = block.leaves.iter()
                .filter(|l| needle.is_empty() || l.key.to_ascii_lowercase().contains(&needle))
                .map(|l| Row {
                    key: l.key.clone(),
                    label: l.label.clone(),
                    section: if needle.is_empty() { l.section.clone() } else { None },
                    ty: l.ty,
                    hint: l.hint.clone(),
                    tuned: self.tuned.contains(&l.key),
                })
                .collect();
            let opaque: Vec<String> = block.opaques.iter()
                .map(|o| format!("{} {} at {} ({} bytes): no control", o.type_name, o.key, o.offset, o.size))
                .collect();
            if rows.is_empty() && opaque.is_empty() { continue; }

            egui::CollapsingHeader::new(egui::RichText::new(title).strong())
                .id_salt(("block", bi))
                .default_open(true)
                .show(ui, |ui| {
                    for note in &opaque { ui.label(egui::RichText::new(note).small()); }
                    edited |= self.ui_rows(ui, &rows, bi);
                });
        }
        if edited { self.dirty = true; }
    }

    /// Consecutive rows sharing a section collapse together under it.
    fn ui_rows(&mut self, ui: &mut egui::Ui, rows: &[Row], bi: usize) -> bool {
        let mut edited = false;
        let mut i = 0usize;
        while i < rows.len() {
            match &rows[i].section {
                None => { edited |= self.ui_one(ui, &rows[i]); i += 1; }
                Some(section) => {
                    let mut j = i;
                    while j < rows.len() && rows[j].section.as_deref() == Some(section.as_str()) { j += 1; }
                    let title = section.rsplit('.').next().unwrap_or(section).to_string();
                    let group = &rows[i..j];
                    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
                        .id_salt(("section", bi, i))
                        .default_open(false)
                        .show(ui, |ui| {
                            for row in group { edited |= self.ui_one(ui, row); }
                        });
                    i = j;
                }
            }
        }
        edited
    }

    /// One member: its name and type, its notes, then its control.
    /// A tuned member shows green, because those are the values Save commits.
    fn ui_one(&mut self, ui: &mut egui::Ui, row: &Row) -> bool {
        ui.separator();
        let mut name = egui::RichText::new(format!("{} {}", ty_name(row.ty), row.label)).strong();
        if row.tuned { name = name.color(TUNING_COL); }
        ui.label(name).on_hover_text(&row.key);
        if !row.hint.notes.is_empty() {
            ui.label(egui::RichText::new(&row.hint.notes).small());
        }
        value_row(ui, &row.key, row.ty, &row.hint, &mut self.values, &mut self.motion)
    }
}

// --------------------------------------------------------------------------------------------------------------------
// The log.

/// What the Output panel says: the shader's shape first, then anything wrong with it.
fn compose_log(shader: &Unified, problems: &[String]) -> String {
    let mut out = String::new();
    out.push_str(&format!("dialect: unified   stages: {} / {}\n",
        shader.vertex_define.as_deref().unwrap_or("(built-in)"),
        shader.fragment_define.as_deref().unwrap_or("(none)")));
    for o in &shader.outputs {
        let roles: Vec<String> = o.binds.iter()
            .map(|b| format!("{}:{}", b.role.name(), b.spelling))
            .collect();
        out.push_str(&format!("  location {}  {} {}  [{}]\n", o.location, ty_name(o.ty), o.name, roles.join(", ")));
    }
    let missing: Vec<&str> = ALL_ROLES.iter()
        .filter(|r| r.lights() && **r != Role::Data && !shader.roles.contains_key(r))
        .map(|r| r.name())
        .collect();
    if !missing.is_empty() {
        out.push_str(&format!("  lit view falls back for: {}\n", missing.join(", ")));
    }
    if problems.is_empty() { return out; }
    out.push_str("\nIssues:");
    for p in problems {
        out.push_str("\n- ");
        out.push_str(p);
    }
    out
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "\
#version 450
layout(std140, binding = 0) uniform Globals {
    float a;    // !(default=1)
} globals;
#ifdef FRAGMENT
layout(location = 0) out vec4 o_albedo;   // !(pbr=albedo:rgb)
layout(location = 1) out vec4 o_n;        // !(pbr=normal:xyz, pbr=height:w)
void main() {}
#endif
";

    #[test]
    fn the_log_names_the_stages_the_outputs_and_the_roles() {
        let shader = parse(SRC);
        assert!(shader.errors.is_empty(), "{:?}", shader.errors);
        let log = compose_log(&shader, &[]);
        assert!(log.contains("stages: (built-in) / FRAGMENT"), "{log}");
        assert!(log.contains("location 1  vec4 o_n  [normal:xyz, height:w]"), "{log}");
        assert!(log.contains("lit view falls back for:"), "{log}");
        assert!(log.contains("roughness"), "{log}");
    }

    #[test]
    fn problems_are_listed_under_the_shape() {
        let shader = parse(SRC);
        let log = compose_log(&shader, &["something is off".to_string()]);
        assert!(log.contains("Issues:"), "{log}");
        assert!(log.contains("- something is off"), "{log}");
    }
}
