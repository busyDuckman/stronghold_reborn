// --------------------------------------------------------------------------------------------------------------------
// The generated controls, and the uniforms that move on their own.
//
// One uniform, one row, matched to its type and its hints. Both editors use this: a paired
// shader's flat uniform and a unified shader's block member differ only in what their key
// looks like, so there is no reason for two sets of sliders.
//
// Motion holds the controls for the uniforms that drive themselves: a TIME sawtooth, an
// anim= auto-lerp, a running integral. The accumulated values live in UniformValues like any
// other, so they upload the same way and nothing downstream knows the difference.

use std::collections::HashMap;

use egui::Color32;
use macroquad::prelude::*;

use crate::glsl_meta::{matches_fmt, GLSLType, Hint};
use crate::values::UniformValues;

/// Green marks tuning params: the values "Save tuning" commits, as opposed to live-only ones.
pub const TUNING_COL: Color32 = Color32::from_rgb(0, 180, 0);

/// format=TIME: a sawtooth of raw seconds over the loop length. Drives u_time.
#[derive(Clone, Debug, Default)]
pub struct TimeCtl {
    pub playing: bool,
    pub loop_len: f32,
    pub t: f32,
    pub last_update_wall: f64,
}

/// format=INTEGRAL: value += what * dt each frame, the way a running integral behaves in a game loop.
/// The accumulated value lives in UniformValues.float, so it uploads like any other float.
#[derive(Clone, Debug, Default)]
pub struct IntegralCtl {
    pub playing: bool,
    pub what: String,
    pub last_update_wall: f64,
}

/// anim=<secs>: sweeps a ranged float min to max over the period, then snaps back and loops.
/// Saves hand-scrubbing a value to see what an animation looks like.
/// `wrapped` flags the frame the sweep restarts, so an integral of this uniform resets in step.
#[derive(Clone, Debug, Default)]
pub struct AnimCtl {
    pub enabled: bool,
    pub period: f32,
    pub min: f32,
    pub max: f32,
    pub phase_t: f32,
    pub last_update_wall: f64,
    pub wrapped: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Motion {
    pub time: HashMap<String, TimeCtl>,
    pub integral: HashMap<String, IntegralCtl>,
    pub anim: HashMap<String, AnimCtl>,
}

/// What a uniform needs for Motion to drive it: its key, its type and its hints. Both a
/// UniformSpec and a block Leaf collapse to this, which is the whole point.
pub struct Driven<'a> {
    pub key: &'a str,
    pub ty: GLSLType,
    pub hint: &'a Hint,
}

impl Motion {
    /// Rebuild every self-driving control from the hints. Called on load and after a reload.
    /// `now` is the wall clock the controls start from; taking it as an argument rather than
    /// reading it keeps Motion testable without a window open.
    pub fn rebuild(&mut self, driven: &[Driven], values: &mut UniformValues, now: f64) {
        self.time.clear();
        self.integral.clear();
        self.anim.clear();

        for d in driven {
            if matches_fmt(d.hint, "TIME") {
                let loop_len = d.hint.max.unwrap_or(10.0).max(0.1);
                self.time.insert(d.key.to_string(),
                    TimeCtl { playing: true, loop_len, t: 0.0, last_update_wall: now });
            }
            if matches_fmt(d.hint, "INTEGRAL") {
                values.float.insert(d.key.to_string(), 0.0);   // restart accumulation, as TIME restarts at t=0
                self.integral.insert(d.key.to_string(), IntegralCtl {
                    playing: true,
                    what: d.hint.what.clone().unwrap_or_default(),
                    last_update_wall: now,
                });
            }
            // any ranged float can auto-lerp; an anim= hint pre-enables it and sets the period
            if !matches!(d.ty, GLSLType::Float) { continue; }
            if matches_fmt(d.hint, "TIME") || matches_fmt(d.hint, "INTEGRAL") { continue; }
            let (Some(lo), Some(hi)) = (d.hint.min, d.hint.max) else { continue };
            let hinted = d.hint.anim.filter(|a| *a > 0.0);
            self.anim.insert(d.key.to_string(), AnimCtl {
                enabled: hinted.is_some(),
                period: hinted.unwrap_or(5.0),
                min: lo,
                max: hi,
                phase_t: 0.0,
                last_update_wall: now,
                wrapped: false,
            });
        }
    }

    /// True while something is actually moving, so a backend that renders on demand knows to.
    pub fn is_running(&self, paused: bool) -> bool {
        if paused { return false; }
        self.time.values().any(|c| c.playing)
            || self.anim.values().any(|c| c.enabled)
            || self.integral.values().any(|c| c.playing)
    }

    /// Advance every self-driving uniform to the wall clock reading `now`.
    pub fn update(&mut self, values: &mut UniformValues, paused: bool, now: f64) {
        for (key, ctl) in self.time.iter_mut() {
            if paused || !ctl.playing { ctl.last_update_wall = now; continue; }
            let dt = (now - ctl.last_update_wall) as f32;
            ctl.last_update_wall = now;
            ctl.t = (ctl.t + dt).rem_euclid(ctl.loop_len);
            values.float.insert(key.clone(), ctl.t);
        }

        for (key, ctl) in self.anim.iter_mut() {
            if paused || !ctl.enabled {
                ctl.last_update_wall = now;
                ctl.wrapped = false;
                continue;
            }
            let dt = (now - ctl.last_update_wall) as f32;
            ctl.last_update_wall = now;
            let advanced = ctl.phase_t + dt;
            ctl.wrapped = advanced >= ctl.period;              // crossed max->min this frame
            ctl.phase_t = advanced.rem_euclid(ctl.period);
            let val = ctl.min + (ctl.max - ctl.min) * (ctl.phase_t / ctl.period);
            values.float.insert(key.clone(), val);
        }

        // integrals run last so they see this frame's final TIME / anim values. If the source
        // is an anim uniform that just wrapped, restart the integral too, so a looping preview
        // stays periodic rather than drifting up forever.
        for (key, ctl) in self.integral.iter_mut() {
            if paused || !ctl.playing { ctl.last_update_wall = now; continue; }
            let dt = (now - ctl.last_update_wall) as f32;
            ctl.last_update_wall = now;

            let src = values.float.get(&ctl.what).copied().unwrap_or(0.0);
            let src_wrapped = self.anim.get(&ctl.what).map(|a| a.enabled && a.wrapped).unwrap_or(false);
            let acc = values.float.entry(key.clone()).or_insert(0.0);
            if src_wrapped { *acc = 0.0; }
            *acc += src * dt;
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// The rows.

/// Snap a value to its step= hint, for something the shader needs whole.
fn quantise(v: f32, hint: &Hint) -> f32 {
    match hint.step {
        Some(s) if s > 0.0 => (v / s).round() * s,
        _ => v,
    }
}

/// One uniform's control row, matched to its type and hints. Returns true if it was edited.
/// Samplers are not here: a texture stand-in belongs to the paired path and its own panel.
pub fn value_row(ui: &mut egui::Ui, key: &str, ty: GLSLType, hint: &Hint,
    values: &mut UniformValues, motion: &mut Motion) -> bool
{
    match ty {
        GLSLType::Float => float_row(ui, key, hint, values, motion),
        GLSLType::Vec2 => vec_row(ui, key, hint, 2, values),
        GLSLType::Vec3 => vec_row(ui, key, hint, 3, values),
        GLSLType::Vec4 => vec_row(ui, key, hint, 4, values),
        GLSLType::Int => {
            let v = values.int.entry(key.to_string()).or_insert(0);
            let mut drag = egui::DragValue::new(v);
            if let (Some(lo), Some(hi)) = (hint.min, hint.max) { drag = drag.range(lo as i64..=hi as i64); }
            ui.horizontal(|ui| { ui.add_space(12.0); ui.add(drag) }).inner.changed()
        }
        GLSLType::Uint => {
            let v = values.uint.entry(key.to_string()).or_insert(0);
            let hi = hint.max.map(|m| m.max(0.0) as i64).unwrap_or(u32::MAX as i64);
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.add(egui::DragValue::new(v).range(0..=hi))
            }).inner.changed()
        }
        GLSLType::Sampler2D => false,
    }
}

fn float_row(ui: &mut egui::Ui, key: &str, hint: &Hint, values: &mut UniformValues, motion: &mut Motion) -> bool {
    time_ctl_row(ui, key, values, motion);
    integral_ctl_row(ui, key, values, motion);

    // slider range: an explicit min/max, else a TIME uniform scrubs over its loop length
    let range = match (hint.min, hint.max) {
        (Some(lo), Some(hi)) => Some((lo, hi)),
        _ if matches_fmt(hint, "TIME") => {
            Some((0.0, motion.time.get(key).map(|c| c.loop_len).unwrap_or(10.0)))
        }
        _ => None,
    };

    // while Anim drives this uniform the hand-edit widgets grey out: a drag would just be
    // overwritten next frame. The greyed box still shows the live value.
    let anim_on = motion.anim.get(key).map(|c| c.enabled).unwrap_or(false);
    let value = values.float.entry(key.to_string()).or_insert(0.0);
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_space(12.0);   // margin, so dragging left does not run off the window edge
        let row_h = ui.spacing().interact_size.y;
        ui.add_enabled_ui(!anim_on, |ui| {
            let mut drag = egui::DragValue::new(&mut *value).speed(hint.step.unwrap_or(0.01));
            if let Some((lo, hi)) = range { drag = drag.range(lo..=hi); }
            // fixed-width box: its natural width tracks the digit count, which would
            // otherwise shove the slider sideways every time the value ticks over
            changed |= ui.add_sized([80.0, row_h], drag).changed();
            if let Some((lo, hi)) = range {
                let mut slider = egui::Slider::new(&mut *value, lo..=hi).show_value(false);
                if let Some(s) = hint.step { slider = slider.step_by(s as f64); }
                changed |= ui.add(slider).changed();
            }
        });
    });
    if changed { *value = quantise(*value, hint); }

    // scrubbing a TIME uniform moves its clock too, so Play resumes from where you left it
    if changed {
        let v = *value;
        if let Some(ctl) = motion.time.get_mut(key) { ctl.t = v; }
    }

    anim_ctl_row(ui, key, motion);
    changed
}

/// The name of one component, from the lanes= hint if the member gave its lanes names.
fn lane_name(hint: &Hint, i: usize) -> String {
    match hint.lanes.as_ref().and_then(|l| l.get(i)) {
        Some(n) => n.clone(),
        None => ["x", "y", "z", "w"][i.min(3)].to_string(),
    }
}

fn vec_row(ui: &mut egui::Ui, key: &str, hint: &Hint, n: usize, values: &mut UniformValues) -> bool {
    if n == 3 && matches_fmt(hint, "RGB") {
        let col = values.vec3.entry(key.to_string()).or_insert(Vec3::ONE);
        let mut arr = [col.x, col.y, col.z];
        let changed = ui.color_edit_button_rgb(&mut arr).changed();
        if changed { *col = vec3(arr[0], arr[1], arr[2]); }
        return changed;
    }
    if n == 4 && matches_fmt(hint, "RGBA") {
        let col = values.vec4.entry(key.to_string()).or_insert(Vec4::ONE);
        let mut arr = [col.x, col.y, col.z, col.w];
        let changed = ui.color_edit_button_rgba_unmultiplied(&mut arr).changed();
        if changed { *col = vec4(arr[0], arr[1], arr[2], arr[3]); }
        return changed;
    }

    // read into a fixed four so one body serves vec2, vec3 and vec4
    let mut v = match n {
        2 => { let a = *values.vec2.entry(key.to_string()).or_insert(Vec2::ZERO); [a.x, a.y, 0.0, 0.0] }
        3 => { let a = *values.vec3.entry(key.to_string()).or_insert(Vec3::ONE); [a.x, a.y, a.z, 0.0] }
        _ => { let a = *values.vec4.entry(key.to_string()).or_insert(Vec4::ONE); [a.x, a.y, a.z, a.w] }
    };
    let mut changed = false;
    // a named-lane member gets one labelled row per lane, since the names are the whole point
    let stacked = hint.lanes.is_some() && n > 2;
    ui.vertical(|ui| {
        let mut draw = |ui: &mut egui::Ui, i: usize| {
            ui.label(lane_name(hint, i));
            let mut drag = egui::DragValue::new(&mut v[i]).speed(hint.step.unwrap_or(0.01).max(0.001));
            if let (Some(lo), Some(hi)) = (hint.min, hint.max) { drag = drag.range(lo..=hi); }
            changed |= ui.add_sized([70.0, ui.spacing().interact_size.y], drag).changed();
        };
        if stacked {
            for i in 0..n { ui.horizontal(|ui| { ui.add_space(12.0); draw(ui, i); }); }
        } else {
            ui.horizontal(|ui| {
                ui.add_space(12.0);
                for i in 0..n { draw(ui, i); }
            });
        }
    });
    if changed {
        for i in 0..n { v[i] = quantise(v[i], hint); }
        match n {
            2 => { values.vec2.insert(key.to_string(), vec2(v[0], v[1])); }
            3 => { values.vec3.insert(key.to_string(), vec3(v[0], v[1], v[2])); }
            _ => { values.vec4.insert(key.to_string(), vec4(v[0], v[1], v[2], v[3])); }
        }
    }
    changed
}

pub fn time_ctl_row(ui: &mut egui::Ui, key: &str, values: &mut UniformValues, motion: &mut Motion) {
    let Some(ctl) = motion.time.get_mut(key) else { return };
    let mut jog = 0.0f32;
    ui.horizontal(|ui| {
        ui.label("TIME loop");
        if ui.button(if ctl.playing { "Pause" } else { "Play" }).clicked() { ctl.playing = !ctl.playing; }
        if ui.button("Restart").clicked() { ctl.t = 0.0; ctl.last_update_wall = get_time(); }
        // frame jog: pause and step one 60fps frame, wrapping on the loop
        if ui.button("<").clicked() { jog = -1.0 / 60.0; }
        if ui.button(">").clicked() { jog =  1.0 / 60.0; }
        ui.label("length");
        ui.add(egui::DragValue::new(&mut ctl.loop_len).range(0.1..=600.0));
    });
    if jog != 0.0 {
        ctl.playing = false;
        ctl.t = (ctl.t + jog).rem_euclid(ctl.loop_len);
        let t = ctl.t;
        // the paused update loop will not push it, so sync the shown value here
        values.float.insert(key.to_string(), t);
    }
}

pub fn integral_ctl_row(ui: &mut egui::Ui, key: &str, values: &mut UniformValues, motion: &mut Motion) {
    let Some(ctl) = motion.integral.get_mut(key) else { return };
    let mut reset = false;
    ui.horizontal(|ui| {
        ui.label(format!("integral of {}", ctl.what));
        if ui.button(if ctl.playing { "Pause" } else { "Play" }).clicked() { ctl.playing = !ctl.playing; }
        if ui.button("Reset").clicked() { reset = true; }
    });
    if reset { values.float.insert(key.to_string(), 0.0); }
}

/// Auto-lerp control, under the value box it drives. Present for every ranged float;
/// pre-enabled when the uniform carries an anim=<secs> hint.
pub fn anim_ctl_row(ui: &mut egui::Ui, key: &str, motion: &mut Motion) {
    let Some(ctl) = motion.anim.get_mut(key) else { return };
    ui.horizontal(|ui| {
        ui.add_space(12.0);   // indent, to line up under the value box above
        ui.checkbox(&mut ctl.enabled, "Anim")
            .on_hover_text("Auto-lerp this uniform from min to max, looping");
        ui.label("min-max /");
        ui.add(egui::DragValue::new(&mut ctl.period).range(0.05..=600.0).suffix(" s"));
        if ui.button("Restart").clicked() { ctl.phase_t = 0.0; ctl.last_update_wall = get_time(); }
    });
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glsl_meta::parse_uniforms;

    fn driven_of(specs: &[crate::glsl_meta::UniformSpec]) -> Vec<Driven<'_>> {
        specs.iter().map(|s| Driven { key: &s.name, ty: s.ty, hint: &s.hint }).collect()
    }

    #[test]
    fn rebuild_makes_one_control_per_self_driving_uniform() {
        let src = "\
uniform float u_time;   // !(format=TIME, max=4.0)
uniform float u_p;      // !(min=0, max=2, anim=8)
uniform float u_q;      // !(min=0, max=1)
uniform float u_acc;    // !(format=integral, what=u_p)
uniform vec2  u_v;      // !(min=0, max=1)
";
        let (specs, errs) = parse_uniforms(src);
        assert!(errs.is_empty(), "{errs:?}");
        let mut values = UniformValues::default();
        let mut motion = Motion::default();
        motion.rebuild(&driven_of(&specs), &mut values, 0.0);

        assert_eq!(motion.time.len(), 1);
        assert_eq!(motion.integral.len(), 1);
        // every ranged float gets an anim control; only u_p has it pre-enabled
        assert_eq!(motion.anim.len(), 2);
        assert!(motion.anim.get("u_p").expect("u_p").enabled);
        assert!(!motion.anim.get("u_q").expect("u_q").enabled);
        assert!(!motion.anim.contains_key("u_v"), "anim is float-only");
    }

    #[test]
    fn nothing_moves_while_paused() {
        let (specs, _) = parse_uniforms("uniform float u_t; // !(format=TIME, max=4.0)\n");
        let mut values = UniformValues::default();
        let mut motion = Motion::default();
        motion.rebuild(&driven_of(&specs), &mut values, 0.0);
        assert!(motion.is_running(false));
        assert!(!motion.is_running(true));

        motion.update(&mut values, true, 1.0);
        assert_eq!(values.float.get("u_t").copied().unwrap_or(0.0), 0.0);
    }

    #[test]
    fn a_step_hint_snaps_the_value() {
        let mut hint = Hint::default();
        hint.step = Some(0.25);
        assert_eq!(quantise(0.31, &hint), 0.25);
        assert_eq!(quantise(0.4, &hint), 0.5);
        hint.step = None;
        assert_eq!(quantise(0.31, &hint), 0.31);
    }

    #[test]
    fn lane_names_fall_back_to_xyzw() {
        let mut hint = Hint::default();
        assert_eq!(lane_name(&hint, 2), "z");
        hint.lanes = Some(vec!["u0v0".into(), "u1v0".into()]);
        assert_eq!(lane_name(&hint, 0), "u0v0");
        assert_eq!(lane_name(&hint, 3), "w");
    }
}
