// --------------------------------------------------------------------------------------------------------------------
// Headless mode: compile-check a shader, or render one frame to a PNG, without the UI.
//
// "Headless-ish" is the honest name. macroquad still needs a real GL context, so a small
// window flashes up for the render and the process then exits. Compile errors and metadata
// issues go to stdout and the exit code is non-zero on failure, which is what makes this
// usable from a script, from CI, or by an agent that cannot look at a window.
//
//   shader_editor <file.glsl>                     the interactive editor (default)
//   shader_editor <file.glsl> --check             compile + metadata check + uniform dump
//   shader_editor <file.glsl> --out <file.png>    render one frame
// With --out:
//   --size <WxH>        render size (default: the resolution uniform's default=, else 512x512)
//   --time <secs>       virtual time: TIME uniforms wrap, anim uniforms sweep, integrals accumulate
//   --set <name=value>  override a uniform, repeatable. "0.75", "1,2,3", "#rrggbb", "text:\"Hi\""
// Unified shaders only (see dialect.rs), where the picture is a G-buffer rather than one target:
//   --channel <name>    lit (default), a declared pbr role, raw:<N>, or all for one file each
//   --tiles <n>         show the bake n by n, to see whether it joins itself
//   --toggle <NAME=on>  set an #ifdef the shader offers, the way the host's build would

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use macroquad::prelude::*;

use crate::dialect::{detect, Dialect};
use crate::prelude::*;
use crate::tuning::{check_editor_blocks, parse_const_value, ConstVal};
use crate::values::apply_set_override;

#[derive(Default, Debug)]
pub struct CliArgs {
    pub glsl: Option<String>,
    pub out: Option<String>,
    pub check: bool,
    pub size: Option<(u32, u32)>,
    pub time: Option<f32>,
    pub sets: Vec<(String, String)>,
    pub channel: Option<String>,
    pub tiles: Option<f32>,
    pub toggles: Vec<(String, bool)>,
}

impl CliArgs {
    pub fn is_headless(&self) -> bool { self.check || self.out.is_some() }
}

pub fn parse_cli() -> Result<CliArgs, String> {
    let mut cli = CliArgs::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--check" => cli.check = true,
            "--out" => cli.out = Some(args.next().ok_or("--out needs a file path")?),
            "--time" => {
                let v = args.next().ok_or("--time needs a value in seconds")?;
                cli.time = Some(v.parse::<f32>().map_err(|_| format!("--time '{}' is not a number", v))?);
            }
            "--size" => {
                let v = args.next().ok_or("--size needs WxH, eg 512x128")?;
                let (w, h) = v.split_once(['x', 'X']).ok_or_else(|| format!("--size '{}' is not WxH", v))?;
                let w = w.trim().parse::<u32>().map_err(|_| format!("--size width '{}' is not a number", w))?;
                let h = h.trim().parse::<u32>().map_err(|_| format!("--size height '{}' is not a number", h))?;
                cli.size = Some((w.max(1), h.max(1)));
            }
            "--set" => {
                let v = args.next().ok_or("--set needs name=value")?;
                let (k, val) = v.split_once('=').ok_or_else(|| format!("--set '{}' is not name=value", v))?;
                cli.sets.push((k.trim().to_string(), val.trim().to_string()));
            }
            "--channel" => cli.channel = Some(args.next().ok_or("--channel needs a name, a raw:N or all")?),
            "--tiles" => {
                let v = args.next().ok_or("--tiles needs a count")?;
                let n = v.parse::<f32>().map_err(|_| format!("--tiles '{}' is not a number", v))?;
                if n < 1.0 { return Err(format!("--tiles '{}' is less than one tile", v)); }
                cli.tiles = Some(n);
            }
            "--toggle" => {
                let v = args.next().ok_or("--toggle needs NAME=on or NAME=off")?;
                let (k, val) = v.split_once('=').ok_or_else(|| format!("--toggle '{}' is not NAME=on", v))?;
                let on = match val.trim().to_ascii_lowercase().as_str() {
                    "on" | "1" | "true" => true,
                    "off" | "0" | "false" => false,
                    other => return Err(format!("--toggle {}: '{}' is not on or off", k.trim(), other)),
                };
                cli.toggles.push((k.trim().to_string(), on));
            }
            other if other.starts_with("--") => return Err(format!("unknown flag '{}'", other)),
            other => {
                if cli.glsl.is_some() { return Err(format!("unexpected extra argument '{}'", other)); }
                cli.glsl = Some(other.to_string());
            }
        }
    }
    Ok(cli)
}

// --------------------------------------------------------------------------------------------------------------------
// Reporting. This is a CLI tool, so stdout IS the output.

fn print_metadata_issues(errs: &[String]) {
    if errs.is_empty() { return; }
    println!("METADATA ISSUES:");
    for e in errs { println!("- {}", e); }
}

fn print_uniform_specs(specs: &[UniformSpec]) {
    println!("UNIFORMS:");
    for s in specs {
        let h = &s.hint;
        let mut parts = Vec::new();
        if let Some(f) = &h.format  { parts.push(format!("format={}", f)); }
        if let Some(d) = &h.default { parts.push(format!("default={}", d)); }
        if let Some(m) = h.min      { parts.push(format!("min={}", m)); }
        if let Some(m) = h.max      { parts.push(format!("max={}", m)); }
        if let Some(w) = &h.what    { parts.push(format!("what={}", w)); }
        if let Some((w, ht)) = h.size { parts.push(format!("size=[{},{}]", w, ht)); }
        if let Some(a) = h.anim     { parts.push(format!("anim={}", a)); }
        let hints = if parts.is_empty() { String::new() } else { format!("  !({})", parts.join(", ")) };
        let notes = if h.notes.is_empty() { String::new() } else { format!("  -- {}", h.notes) };
        println!("  {} {}{}{}", ty_name(s.ty), s.name, hints, notes);
    }
}

fn print_editor_blocks(blocks: &[EditorBlock]) {
    if blocks.is_empty() { return; }
    println!("EDITOR TUNING (the #else consts the project compiles):");
    for b in blocks {
        if let Some(g) = &b.group { println!("  group \"{}\":", g); }
        for c in &b.consts {
            let parsed = if parse_const_value(c).is_some() { "" } else { "   [UNPARSABLE]" };
            println!("  const {} {} = {}{}", ty_name(c.ty), c.name, c.raw, parsed);
        }
    }
}

// --------------------------------------------------------------------------------------------------------------------
// Virtual time: the same motion the editor's controls produce, evaluated at one instant.

/// Set every self-driving uniform to what it would read at t seconds.
/// Order matters: TIME and anim first, integrals last, so an integral sees the final source value.
pub fn seat_virtual_time(values: &mut UniformValues, specs: &[UniformSpec], explicit: &HashSet<String>, t: f32) {
    for s in specs {
        if explicit.contains(&s.name) || !matches_fmt(&s.hint, "TIME") { continue; }
        let loop_len = s.hint.max.unwrap_or(10.0).max(0.1);
        values.float.insert(s.name.clone(), t.rem_euclid(loop_len));
    }

    for s in specs {
        if explicit.contains(&s.name) { continue; }
        if matches_fmt(&s.hint, "TIME") || matches_fmt(&s.hint, "INTEGRAL") { continue; }
        let (Some(period), Some(lo), Some(hi)) = (s.hint.anim, s.hint.min, s.hint.max) else { continue };
        if period <= 0.0 || !matches!(s.ty, GLSLType::Float) { continue; }
        values.float.insert(s.name.clone(), lo + (hi - lo) * (t.rem_euclid(period) / period));
    }

    for s in specs {
        if explicit.contains(&s.name) || !matches_fmt(&s.hint, "INTEGRAL") { continue; }
        values.float.insert(s.name.clone(), integral_at(values, specs, s, t));
    }
}

/// The value of one format=integral uniform at time t.
///   - Steady or TIME source: the integral is what * t.
///   - anim source: the integral of the ramp over the current cycle, reset each period,
///     matching the editor resetting the accumulator when the sweep wraps.
fn integral_at(values: &UniformValues, specs: &[UniformSpec], spec: &UniformSpec, t: f32) -> f32 {
    let what = spec.hint.what.as_deref();
    let anim_src = what
        .and_then(|wn| specs.iter().find(|o| o.name == wn))
        .and_then(|o| match (o.hint.anim, o.hint.min, o.hint.max) {
            (Some(period), Some(lo), Some(hi)) if period > 0.0 => Some((period, lo, hi)),
            _ => None,
        });
    match anim_src {
        Some((period, lo, hi)) => {
            let tau = t.rem_euclid(period);                     // seconds into the current cycle
            lo * tau + (hi - lo) * tau * tau / (2.0 * period)   // integral of lerp(lo, hi, .) over [0, tau]
        }
        None => what.and_then(|wn| values.float.get(wn)).copied().unwrap_or(0.0) * t,
    }
}

// --------------------------------------------------------------------------------------------------------------------
// The run.

/// Compile, check, and optionally render, with no UI. Returns the process exit code.
/// The file decides which backend runs, the same way it does for the interactive editor.
pub async fn run(cli: CliArgs) -> i32 {
    let Some(glsl) = cli.glsl.clone() else {
        println!("ERROR: no .glsl file given");
        return 1;
    };
    let src = match fs::read_to_string(&glsl) {
        Ok(s) => s,
        Err(e) => { println!("ERROR: cannot read {}: {}", glsl, e); return 1; }
    };
    let detection = detect(&src);
    println!("DIALECT {}  ({})", detection.dialect.name(), detection.because);
    for e in &detection.errors { println!("- {}", e); }

    match detection.dialect {
        Dialect::Paired => run_paired(cli).await,
        Dialect::Unified => crate::unified_headless::run(&cli, Path::new(&glsl), &src),
    }
}

/// The paired path: two files, one target, drawn by macroquad.
async fn run_paired(cli: CliArgs) -> i32 {
    let Some(glsl) = cli.glsl.clone() else {
        println!("ERROR: no .glsl file given");
        return 1;
    };
    for (name, _) in &cli.toggles {
        println!("ERROR: --toggle {}: a paired shader has no #ifdef toggles", name);
        return 1;
    }
    if cli.channel.is_some() || cli.tiles.is_some() {
        println!("ERROR: --channel and --tiles are for unified shaders; a paired shader has one target");
        return 1;
    }
    let frag_path = PathBuf::from(&glsl);
    let vert_path = paired_vert(&frag_path);
    if !vert_path.exists() {
        println!("ERROR: missing paired vertex file: {}", vert_path.display());
        return 1;
    }
    let fp = FilePair { frag: frag_path, vert: vert_path };

    let frag_src = match fs::read_to_string(&fp.frag) {
        Ok(s) => s,
        Err(e) => { println!("ERROR: cannot read {}: {}", fp.frag.display(), e); return 1; }
    };
    let (specs, mut parse_errs) = parse_uniforms(&frag_src);
    let (editor_blocks, mut block_errs) = parse_editor_blocks(&frag_src);
    parse_errs.append(&mut block_errs);
    parse_errs.extend(check_editor_blocks(&specs, &editor_blocks));

    let material = match load_material_pair(&fp) {
        Ok(m) => m,
        Err(e) => {
            println!("SHADER ERROR:\n{}", e);
            print_metadata_issues(&parse_errs);
            return 1;
        }
    };
    println!("SHADER OK");
    print_metadata_issues(&parse_errs);

    // Names are checked in every mode, --check included. An argument that is quietly ignored
    // because of the mode you happened to pass is worse than one that fails.
    for (name, _) in &cli.sets {
        if !specs.iter().any(|s| &s.name == name) {
            println!("ERROR: --set {}: no such uniform", name);
            return 1;
        }
    }

    if cli.check {
        print_uniform_specs(&specs);
        print_editor_blocks(&editor_blocks);
        return 0;
    }

    // render size: --size beats the resolution uniform's default=, which beats 512x512
    let (w, h) = cli.size.unwrap_or_else(|| {
        specs.iter()
            .find(|s| matches_fmt(&s.hint, "resolution"))
            .and_then(|s| s.hint.default.as_deref())
            .and_then(crate::glsl_meta::parse_vec2_str)
            .map(|v| (v.x.max(1.0) as u32, v.y.max(1.0) as u32))
            .unwrap_or((512, 512))
    });

    let mut values = UniformValues::default();
    seed_const_values(&mut values, &editor_blocks);   // saved consts beat the default= hints
    values.set_defaults(&specs, vec2(w as f32, h as f32));

    // --set wins over the defaults and over --time; sampler names route to the texture map
    let mut explicit: HashSet<String> = HashSet::new();
    let mut tex_overrides: HashMap<String, TexSource> = HashMap::new();
    for (name, raw) in &cli.sets {
        let Some(spec) = specs.iter().find(|s| &s.name == name) else {
            println!("ERROR: --set {}: no such uniform", name);
            return 1;
        };
        if matches!(spec.ty, GLSLType::Sampler2D) {
            match parse_tex_source(raw) {
                Ok(src) => { tex_overrides.insert(name.clone(), src); }
                Err(e) => { println!("ERROR: --set {}: {}", name, e); return 1; }
            }
            continue;
        }
        if let Err(e) = apply_set_override(&mut values, spec, raw) {
            println!("ERROR: {}", e);
            return 1;
        }
        explicit.insert(name.clone());
    }

    if let Some(t) = cli.time {
        seat_virtual_time(&mut values, &specs, &explicit, t);
    }

    material.mat.set_texture("ShaderTexture", make_checker_tex(w, h));

    let glsl_dir = fp.frag.parent().unwrap_or(Path::new(".")).to_path_buf();
    let (standins, tex_warns) = build_standins(&specs, &glsl_dir, &tex_overrides);
    print_metadata_issues(&tex_warns);
    for (name, si) in &standins {
        material.mat.set_texture(name, si.tex.clone());
    }

    let rt = render_target(w, h);
    set_camera(&Camera2D {
        render_target: Some(rt.clone()),
        zoom: vec2(2.0 / w as f32, 2.0 / h as f32),
        target: vec2(w as f32 / 2.0, h as f32 / 2.0),
        ..Default::default()
    });
    clear_background(Color::from_rgba(0, 0, 0, 0));

    let mut mat = material.mat;
    apply_uniforms(&mut mat, &specs, &values, vec2(w as f32, h as f32));
    gl_use_material(&mat);
    draw_rectangle(0.0, 0.0, w as f32, h as f32, WHITE);
    gl_use_default_material();
    set_default_camera();   // ends the render-target pass, flushing the draw

    // NOTE: no vertical flip here, unlike the on-screen preview. get_texture_data() already
    // flips rows into image convention during readback; draw_texture_ex does not.
    let img = rt.texture.get_texture_data();

    let out = cli.out.as_deref().unwrap_or("shader_out.png");
    img.export_png(out);
    println!("WROTE {}  ({}x{})", out, w, h);
    0
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glsl_meta::parse_uniforms;

    const SRC: &str = "\
uniform float u_time;      // !(format=TIME, max=4.0)
uniform float u_p;         // !(min=0.0, max=2.0, anim=8.0)
uniform float u_p_int;     // !(format=integral, what=u_p)
uniform float u_t_int;     // !(format=integral, what=u_time)
";

    fn seated(t: f32) -> UniformValues {
        let (specs, _) = parse_uniforms(SRC);
        let mut values = UniformValues::default();
        values.set_defaults(&specs, Vec2::ONE);
        seat_virtual_time(&mut values, &specs, &HashSet::new(), t);
        values
    }

    #[test]
    fn time_uniforms_wrap_on_their_loop_length() {
        assert_eq!(seated(10.0).float.get("u_time"), Some(&2.0), "10s into a 4s loop is 2s");
    }

    #[test]
    fn anim_uniforms_sweep_their_range() {
        // 8s period, min 0, max 2: at t=2 the sweep is a quarter through
        assert_eq!(seated(2.0).float.get("u_p"), Some(&0.5));
        assert_eq!(seated(8.0).float.get("u_p"), Some(&0.0), "a whole period is back at min");
    }

    #[test]
    fn an_integral_of_a_ramp_is_the_area_under_it() {
        // u_p ramps 0..2 over 8s, so at tau its integral is (2/8) * tau^2 / 2 = tau^2 / 8
        let v = seated(4.0).float.get("u_p_int").copied().expect("u_p_int");
        assert!((v - 2.0).abs() < 1e-5, "expected 2.0, got {v}");
    }

    #[test]
    fn an_integral_of_a_steady_source_is_source_times_time() {
        // u_time is at 2.0 when t=10 (4s loop), so its integral reads 2.0 * 10
        let v = seated(10.0).float.get("u_t_int").copied().expect("u_t_int");
        assert!((v - 20.0).abs() < 1e-5, "expected 20.0, got {v}");
    }

    #[test]
    fn an_explicit_set_is_not_overwritten_by_virtual_time() {
        let (specs, _) = parse_uniforms(SRC);
        let mut values = UniformValues::default();
        values.set_defaults(&specs, Vec2::ONE);
        values.float.insert("u_p".into(), 1.75);
        let explicit: HashSet<String> = ["u_p".to_string()].into_iter().collect();
        seat_virtual_time(&mut values, &specs, &explicit, 2.0);
        assert_eq!(values.float.get("u_p"), Some(&1.75));
    }
}
