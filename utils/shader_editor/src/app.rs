// --------------------------------------------------------------------------------------------------------------------
// The interactive editor: preview on the right, generated controls on the left, compiler
// output along the bottom, and a poll that recompiles whenever either file is saved.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use egui::Color32;
use macroquad::prelude::*;

use crate::controls::{value_row, Driven, Motion, TUNING_COL};
use crate::prelude::*;
use crate::shader_file::{changed, stamp_pair, FileStamp};
use crate::standins::build_standin_tex;
use crate::tuning::{check_editor_blocks, write_tuning_consts};

/// How often the files on disk are checked, in seconds.
const POLL_INTERVAL_S: f64 = 0.5;

/// One row of the uniforms panel: a lone uniform, or a whole named tuning block as a
/// collapsing section. Indices into App::specs and App::editor_blocks.
enum UniformRow { Flat(usize), Group(usize, Vec<usize>) }

// --------------------------------------------------------------------------------------------------------------------
// The app.

pub struct App {
    file: FilePair,
    stamp: FileStamp,
    source_ok: bool,

    material: ShaderMaterial,
    log: String,

    specs: Vec<UniformSpec>,
    editor_blocks: Vec<EditorBlock>,
    values: UniformValues,
    motion: Motion,
    standins: HashMap<String, StandIn>,

    dest_size_px: Vec2,
    fit_to_window: bool,
    paused_global: bool,
    texture: Texture2D,
    // the preview renders into an offscreen target at exact pixel size, so gl_FragCoord and
    // any resolution uniform agree with how the project renders; then it is blitted to screen
    preview_rt: Option<RenderTarget>,
    checker_size_px: f32,

    ui_left_w: f32,
    ui_bottom_h: f32,
}

impl App {
    pub fn new(frag: &Path) -> Result<Self, String> {
        let vert = paired_vert(frag);
        if !vert.exists() {
            return Err(format!("Missing paired vertex file: {}", vert.display()));
        }
        let fp = FilePair { frag: frag.to_path_buf(), vert };
        let stamp = stamp_pair(&fp).map_err(|e| e.to_string())?;

        let frag_src = fs::read_to_string(&fp.frag).map_err(|e| e.to_string())?;
        let (specs, parse_errs) = parse_all(&frag_src);
        let material = load_material_pair(&fp)?;

        let tex = make_checker_tex(512, 512);
        let glsl_dir = fp.frag.parent().unwrap_or(Path::new(".")).to_path_buf();
        let (blocks, _) = parse_editor_blocks(&frag_src);
        let (standins, tex_warns) = build_standins(&specs, &glsl_dir, &HashMap::new());
        material.mat.set_texture("ShaderTexture", tex.clone());

        let mut app = Self {
            file: fp,
            stamp,
            source_ok: true,
            material,
            log: compose_log(&parse_errs, &tex_warns),
            specs,
            editor_blocks: blocks,
            values: UniformValues::default(),
            motion: Motion::default(),
            standins,
            dest_size_px: vec2(1024.0, 576.0),
            fit_to_window: true,
            paused_global: false,
            texture: tex,
            preview_rt: None,
            checker_size_px: 16.0,
            ui_left_w: 0.0,
            ui_bottom_h: 0.0,
        };
        app.seed_values_and_controls();
        Ok(app)
    }

    pub fn poll_reload(&mut self) {
        if let Ok(new_stamp) = stamp_pair(&self.file) {
            if changed(&self.stamp, &new_stamp) {
                self.stamp = new_stamp;
                self.reload_from_disk();
            }
        }
    }

    /// Recompile from disk and resync specs, controls and stand-ins. Slider values survive.
    fn reload_from_disk(&mut self) {
        let new_mat = match load_material_pair(&self.file) {
            Ok(m) => m,
            Err(e) => { self.source_ok = false; self.log = e; return; }
        };
        self.material = new_mat;
        self.source_ok = true;

        let Ok(src) = fs::read_to_string(&self.file.frag) else { return };
        let (specs, parse_errs) = parse_all(&src);
        let (blocks, _) = parse_editor_blocks(&src);
        self.specs = specs;
        self.editor_blocks = blocks;
        self.seed_values_and_controls();

        let glsl_dir = self.file.frag.parent().unwrap_or(Path::new(".")).to_path_buf();
        let (standins, tex_warns) = build_standins(&self.specs, &glsl_dir, &HashMap::new());
        self.standins = standins;
        self.log = compose_log(&parse_errs, &tex_warns);
    }

    /// Fill in missing values from the hints and rebuild the TIME / INTEGRAL / anim controls.
    fn seed_values_and_controls(&mut self) {
        // saved consts beat the default= hints; both use or_insert, so live tweaks survive
        seed_const_values(&mut self.values, &self.editor_blocks);
        let tex_size = vec2(self.texture.width(), self.texture.height());
        self.values.set_defaults(&self.specs, tex_size);

        let driven: Vec<Driven> = self.specs.iter()
            .map(|s| Driven { key: &s.name, ty: s.ty, hint: &s.hint })
            .collect();
        self.motion.rebuild(&driven, &mut self.values, get_time());
    }

    /// Forget every tweak, back to the default= hints (tuning params to their saved consts).
    fn reset_params(&mut self) {
        self.values = UniformValues::default();
        self.seed_values_and_controls();
    }

    /// Commit the live tuning values onto the #else consts, then reload so the
    /// created-on-save warnings clear. Slider values survive the reload.
    fn save_tuning(&mut self) {
        let result = fs::read_to_string(&self.file.frag)
            .map_err(|e| format!("read failed: {e}"))
            .and_then(|src| write_tuning_consts(&src, &self.specs, &self.values))
            .and_then(|(out, n)| {
                fs::write(&self.file.frag, out).map_err(|e| format!("write failed: {e}"))?;
                Ok(n)
            });
        match result {
            Ok(n) => {
                // refresh the stamp so the poll does not reload a second time
                if let Ok(s) = stamp_pair(&self.file) { self.stamp = s; }
                self.reload_from_disk();
                self.log.push_str(&format!("\nSaved {} tuning value(s).", n));
            }
            Err(e) => self.log = format!("SAVE FAILED: {e}"),
        }
    }

    /// Display plan for the uniforms panel: file order, with each named tuning block pulled
    /// together into one Group row at the position of its first member.
    fn uniform_rows(&self) -> Vec<UniformRow> {
        let mut rows = Vec::new();
        let mut grouped: Vec<usize> = Vec::new();
        for (si, spec) in self.specs.iter().enumerate() {
            let owner = self.editor_blocks.iter()
                .position(|b| b.group.is_some() && b.uniforms.contains(&spec.name));
            match owner {
                Some(bi) if !grouped.contains(&bi) => {
                    grouped.push(bi);
                    let members = self.specs.iter().enumerate()
                        .filter(|(_, s)| self.editor_blocks[bi].uniforms.contains(&s.name))
                        .map(|(k, _)| k)
                        .collect();
                    rows.push(UniformRow::Group(bi, members));
                }
                Some(_) => {}   // a later member of an already-emitted group
                None => rows.push(UniformRow::Flat(si)),
            }
        }
        rows
    }

// --------------------------------------------------------------------------------------------------------------------
// Per-frame update. (App impl continues)

    pub fn update_animated_uniforms(&mut self) {
        self.motion.update(&mut self.values, self.paused_global, get_time());
    }

    pub fn draw_scene(&mut self) {
        let free_x = self.ui_left_w.max(0.0);
        let free_w = (screen_width() - self.ui_left_w).max(0.0);
        let free_h = (screen_height() - self.ui_bottom_h).max(0.0);

        let mut preview_res = if self.fit_to_window { vec2(free_w, free_h) } else { self.dest_size_px };
        preview_res.x = preview_res.x.min(free_w);
        preview_res.y = preview_res.y.min(free_h);
        let pos = vec2(free_x + (free_w - preview_res.x) * 0.5, (free_h - preview_res.y) * 0.5);

        gl_use_default_material();
        draw_checkerboard(pos, preview_res, self.checker_size_px.max(2.0));

        let (rw, rh) = (preview_res.x.max(1.0) as u32, preview_res.y.max(1.0) as u32);
        let stale = self.preview_rt.as_ref()
            .map(|rt| rt.texture.width() as u32 != rw || rt.texture.height() as u32 != rh)
            .unwrap_or(true);
        if stale { self.preview_rt = Some(render_target(rw, rh)); }
        let Some(rt) = self.preview_rt.clone() else { return };

        set_camera(&Camera2D {
            render_target: Some(rt.clone()),
            zoom: vec2(2.0 / rw as f32, 2.0 / rh as f32),
            target: vec2(rw as f32 / 2.0, rh as f32 / 2.0),
            ..Default::default()
        });
        clear_background(Color::from_rgba(0, 0, 0, 0));

        apply_uniforms(&mut self.material.mat, &self.specs, &self.values, vec2(rw as f32, rh as f32));
        self.material.mat.set_texture("ShaderTexture", self.texture.clone());
        for (name, si) in &self.standins {
            self.material.mat.set_texture(name, si.tex.clone());
        }

        gl_use_material(&self.material.mat);
        draw_rectangle(0.0, 0.0, rw as f32, rh as f32, WHITE);
        gl_use_default_material();
        set_default_camera();

        // flip_y here but NOT in the headless PNG path: get_texture_data() flips rows into image
        // convention during readback, while draw_texture_ex samples the render target raw
        // (bottom-up, as GL stores it). Verified against a shader with known orientation.
        draw_texture_ex(&rt.texture, pos.x, pos.y, WHITE,
            DrawTextureParams { dest_size: Some(preview_res), flip_y: true, ..Default::default() });
    }
}

// --------------------------------------------------------------------------------------------------------------------
// The egui panels. Left: generated uniform controls. Bottom: compiler output.

impl App {
    pub fn ui(&mut self) {
        egui_macroquad::ui(|ctx| {
            let left = egui::SidePanel::left("settings").resizable(true).default_width(360.0).show(ctx, |ui| {
                ui.heading("Settings");
                ui.separator();
                self.ui_view_controls(ui);
                ui.separator();
                self.ui_global_buttons(ui);
                ui.separator();
                ui.heading("Uniforms");
                // the uniform list scrolls: a tuning session can run to dozens of sliders
                egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| self.ui_uniforms(ui));
            });
            self.ui_left_w = left.response.rect.width();

            let bottom = egui::TopBottomPanel::bottom("output").resizable(true).default_height(140.0).show(ctx, |ui| {
                ui.heading("Output");
                ui.separator();
                if self.source_ok {
                    ui.colored_label(Color32::from_rgb(0, 180, 0), "Shader OK");
                } else {
                    ui.colored_label(Color32::from_rgb(220, 40, 40), "Shader Error");
                }
                ui.separator();
                ui.label(&self.log);
            });
            self.ui_bottom_h = bottom.response.rect.height();
        });
        egui_macroquad::draw();
    }

    fn ui_view_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Preview size (px)");
            // the size boxes grey out while fit-to-window is overriding them
            ui.add_enabled(!self.fit_to_window, egui::DragValue::new(&mut self.dest_size_px.x).range(32.0..=4096.0));
            ui.add_enabled(!self.fit_to_window, egui::DragValue::new(&mut self.dest_size_px.y).range(32.0..=4096.0));
            ui.checkbox(&mut self.fit_to_window, "fit to window");
        });
        ui.horizontal(|ui| {
            ui.label("Checker size (px)");
            ui.add(egui::DragValue::new(&mut self.checker_size_px).range(2.0..=256.0));
        });
    }

    fn ui_global_buttons(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Global");
            if ui.button(if self.paused_global { "Play" } else { "Pause" }).clicked() {
                self.paused_global = !self.paused_global;
            }
            if ui.button("Reload").on_hover_text("Recompile from disk, keeping the current values").clicked() {
                // refresh the stamp so the poll does not reload a second time
                if let Ok(s) = stamp_pair(&self.file) { self.stamp = s; }
                self.reload_from_disk();
            }
            if ui.button("Reset params")
                .on_hover_text("All uniforms back to their default= hints (tuning params: their saved consts)")
                .clicked()
            {
                self.reset_params();
            }
            let has_tuning = !self.editor_blocks.is_empty();
            if ui.add_enabled(has_tuning, egui::Button::new("Save tuning"))
                .on_hover_text("Write the live tuning values onto the #else consts")
                .on_disabled_hover_text("No #ifdef EDITOR tuning block in this shader")
                .clicked()
            {
                self.save_tuning();
            }
        });
    }

    /// Grouped tuning blocks render as collapsing sections at the position of their first
    /// member; everything else stays a flat list in file order.
    fn ui_uniforms(&mut self, ui: &mut egui::Ui) {
        for row in self.uniform_rows() {
            match row {
                UniformRow::Flat(si) => {
                    let spec = self.specs[si].clone();
                    let is_tuning = self.editor_blocks.iter().any(|b| b.uniforms.contains(&spec.name));
                    self.ui_one_uniform(ui, &spec, is_tuning);
                }
                UniformRow::Group(bi, members) => {
                    ui.separator();
                    let title = self.editor_blocks[bi].group.clone().unwrap_or_default();
                    egui::CollapsingHeader::new(egui::RichText::new(title).strong().color(TUNING_COL))
                        .id_salt(("tuning_group", bi))
                        .default_open(true)
                        .show(ui, |ui| {
                            for si in members {
                                let spec = self.specs[si].clone();
                                self.ui_one_uniform(ui, &spec, true);
                            }
                        });
                }
            }
        }
    }

    /// One uniform's row: separator, name, notes, then controls matched to its type.
    /// A tuning param shows green, because those are the values Save commits.
    fn ui_one_uniform(&mut self, ui: &mut egui::Ui, spec: &UniformSpec, is_tuning: bool) {
        ui.separator();
        let mut name = egui::RichText::new(&spec.name).strong();
        if is_tuning { name = name.color(TUNING_COL); }
        ui.label(name);
        if !spec.hint.notes.is_empty() {
            ui.label(egui::RichText::new(&spec.hint.notes));
        }

        if matches!(spec.ty, GLSLType::Sampler2D) {
            self.ui_sampler(ui, spec);
        } else {
            value_row(ui, &spec.name, spec.ty, &spec.hint, &mut self.values, &mut self.motion);
        }
    }

    fn ui_sampler(&mut self, ui: &mut egui::Ui, spec: &UniformSpec) {
        let desc = match self.standins.get(&spec.name).map(|si| &si.source) {
            Some(TexSource::File(p))  => format!("file: {}", p),
            Some(TexSource::Text(t))  => format!("text: \"{}\"", t),
            Some(TexSource::Solid(_)) => "solid colour".to_string(),
            Some(TexSource::Checker) | None => "checkerboard".to_string(),
        };
        let mut reload = false;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(desc).small());
            reload = ui.button("Reload").clicked();
        });
        if !reload { return; }
        let dir = self.file.frag.parent().unwrap_or(Path::new(".")).to_path_buf();
        let (w, h) = spec.hint.size.unwrap_or((512, 512));
        let mut warns = Vec::new();
        if let Some(si) = self.standins.get_mut(&spec.name) {
            si.tex = build_standin_tex(&si.source, w, h, &dir, &spec.name, &mut warns);
        }
        for warning in warns { self.log.push_str(&format!("\n- {}", warning)); }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Shared helpers.

/// Parse the uniforms and the tuning blocks together, so the caller gets one error list.
fn parse_all(src: &str) -> (Vec<UniformSpec>, Vec<String>) {
    let (specs, mut errs) = parse_uniforms(src);
    let (blocks, mut block_errs) = parse_editor_blocks(src);
    errs.append(&mut block_errs);
    errs.extend(check_editor_blocks(&specs, &blocks));
    (specs, errs)
}

fn compose_log(parse_errs: &[String], tex_warns: &[String]) -> String {
    let mut out = String::from("GLSL: OK");
    if parse_errs.is_empty() && tex_warns.is_empty() { return out; }
    out.push_str("\nMetadata issues:");
    for e in parse_errs.iter().chain(tex_warns.iter()) {
        out.push_str("\n- ");
        out.push_str(e);
    }
    out
}

pub fn poll_interval_s() -> f64 { POLL_INTERVAL_S }
