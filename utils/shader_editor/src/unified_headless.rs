// --------------------------------------------------------------------------------------------------------------------
// Headless mode for a unified shader: the check that tells you what the file says, and the
// render that writes it out.
//
// --check needs no GPU at all, because naga does the compiling: it works on a build box, in a
// container, over ssh. That matters, since --check is the half an agent or a CI job can use.
// --out does need a device, and says so plainly if there is not one.
//
//   shader_editor terrain.glsl --check
//   shader_editor terrain.glsl --out .output/terrain.png --size 512x512
//   shader_editor terrain.glsl --out .output/t.png --channel all --tiles 2
//   shader_editor terrain.glsl --out .output/t.png --toggle USE_DEFAULTS=on
//   shader_editor terrain.glsl --out .output/t.png --set globals.patch_frequency=6

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use macroquad::prelude::*;

use crate::glsl_meta::{matches_fmt, ty_name, GLSLType};
use crate::headless::{seat_virtual_time, CliArgs};
use crate::png_out::write_rgba;
use crate::unified::composite::{ViewMode, ViewParams};
use crate::unified::gpu::{check_compile, defines_for, Gpu, Scene};
use crate::unified::pack::leaf_specs;
use crate::unified::parse::{parse, Unified};
use crate::unified::roles::{Role, ALL_ROLES};
use crate::unified::tune::{parse_tune, seed_tune_values, TuneAssign};
use crate::values::{apply_set_override, UniformValues};

/// Compile, check, and optionally render. Returns the process exit code.
pub fn run(cli: &CliArgs, path: &Path, src: &str) -> i32 {
    println!("FILE {}", path.display());
    let mut shader = parse(src);
    let (tune, tune_errs) = parse_tune(src);

    let mut problems = shader.errors.clone();
    problems.extend(tune_errs);

    // --toggle before anything else: it changes what compiles, so everything downstream of it
    for (name, on) in &cli.toggles {
        match shader.toggles.iter_mut().find(|t| &t.define == name) {
            Some(t) => t.on = *on,
            None => {
                println!("ERROR: --toggle {}: the shader offers no such #ifdef", name);
                return 1;
            }
        }
    }

    // seeded before the check returns, so a tune path naming a member no block has is caught
    // by --check rather than only by a render
    let mut values = UniformValues::default();
    problems.extend(seed_tune_values(&mut values, &tune, &shader));

    print_stages(&shader);
    print_outputs(&shader);
    print_blocks(&shader);
    print_toggles(&shader);
    print_tuning(&tune);

    let compile_errs = check_compile(&shader, src);
    if compile_errs.is_empty() {
        println!("SHADER OK");
    } else {
        println!("SHADER ERROR:");
        for e in &compile_errs { println!("{}", e); }
    }
    print_problems(&problems);

    // Names are checked in every mode, --check included. An argument that is quietly ignored
    // because of the mode you happened to pass is worse than one that fails.
    for (name, _) in &cli.sets {
        if shader.find_leaf(name).is_none() {
            println!("ERROR: --set {}: no block member has that path", name);
            return 1;
        }
    }
    if !compile_errs.is_empty() || !problems.is_empty() { return 1; }
    if cli.check { return 0; }

    let (w, h) = render_size(cli, &shader);
    let specs = leaf_specs(&shader);
    values.set_defaults(&specs, vec2(w as f32, h as f32));

    let mut explicit: HashSet<String> = HashSet::new();
    for (name, raw) in &cli.sets {
        let Some(spec) = specs.iter().find(|s| &s.name == name) else { continue };
        if let Err(e) = apply_set_override(&mut values, spec, raw) {
            println!("ERROR: {}", e);
            return 1;
        }
        explicit.insert(name.clone());
    }
    if let Some(t) = cli.time {
        seat_virtual_time(&mut values, &specs, &explicit, t);
    }

    let gpu = match Gpu::new() {
        Ok(g) => g,
        Err(e) => { println!("ERROR: {}", e); return 1; }
    };
    println!("ADAPTER {}", gpu.adapter);
    let mut scene = match Scene::new(&gpu, &shader, src) {
        Ok(s) => s,
        Err(e) => { println!("ERROR: {}", e); return 1; }
    };

    let out = cli.out.clone().unwrap_or_else(|| "shader_out.png".to_string());
    let views = match wanted_views(cli, &shader) {
        Ok(v) => v,
        Err(e) => { println!("ERROR: {}", e); return 1; }
    };
    for (suffix, mode) in views {
        let params = ViewParams { mode, tiles: cli.tiles.unwrap_or(1.0), ..Default::default() };
        let frame = match scene.render(&gpu, &values, &params, w, h) {
            Ok(f) => f,
            Err(e) => { println!("ERROR: {}", e); return 1; }
        };
        let target = with_suffix(&out, &suffix);
        if let Err(e) = write_rgba(&target, frame.width, frame.height, &frame.rgba) {
            println!("ERROR: {}", e);
            return 1;
        }
        println!("WROTE {}  ({}x{})", target, w, h);
    }
    0
}

// --------------------------------------------------------------------------------------------------------------------
// Which pictures to write.

/// The views --channel asks for, each with the filename suffix it gets. One view means no
/// suffix, so the plain case writes exactly the file that was asked for.
fn wanted_views(cli: &CliArgs, shader: &Unified) -> Result<Vec<(String, ViewMode)>, String> {
    let want = cli.channel.as_deref().unwrap_or("lit").trim().to_ascii_lowercase();
    if want == "lit" { return Ok(vec![(String::new(), ViewMode::Lit)]); }
    if want == "all" {
        let mut out = vec![("lit".to_string(), ViewMode::Lit)];
        for role in ALL_ROLES.iter().filter(|r| shader.roles.contains_key(r)) {
            out.push((role.name().to_string(), ViewMode::Channel(*role)));
        }
        for o in &shader.outputs {
            out.push((format!("raw{}", o.location), ViewMode::Raw(o.location as usize)));
        }
        return Ok(out);
    }
    if let Some(n) = want.strip_prefix("raw:").or_else(|| want.strip_prefix("raw")) {
        let idx: usize = n.trim().parse().map_err(|_| format!("--channel raw:{}: not a target number", n))?;
        if !shader.outputs.iter().any(|o| o.location as usize == idx) {
            return Err(format!("--channel raw:{}: the shader has no output at that location", idx));
        }
        return Ok(vec![(String::new(), ViewMode::Raw(idx))]);
    }
    let role = Role::from_name(&want).ok_or_else(|| format!("--channel {}: not lit, all, raw:N or a PBR role", want))?;
    if !shader.roles.contains_key(&role) {
        return Err(format!("--channel {}: the shader declares no {} role", want, role.name()));
    }
    Ok(vec![(String::new(), ViewMode::Channel(role))])
}

/// "out.png" plus "albedo" is "out.albedo.png". An empty suffix leaves the path alone.
fn with_suffix(out: &str, suffix: &str) -> String {
    if suffix.is_empty() { return out.to_string(); }
    let p = PathBuf::from(out);
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "png".to_string());
    match p.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => {
            dir.join(format!("{}.{}.{}", stem, suffix, ext)).to_string_lossy().to_string()
        }
        _ => format!("{}.{}.{}", stem, suffix, ext),
    }
}

/// --size beats a resolution member's default=, which beats 512x512.
fn render_size(cli: &CliArgs, shader: &Unified) -> (u32, u32) {
    cli.size.unwrap_or_else(|| {
        shader.leaves()
            .find(|(_, l)| matches!(l.ty, GLSLType::Vec2) && matches_fmt(&l.hint, "resolution"))
            .and_then(|(_, l)| l.hint.default.as_deref())
            .and_then(crate::glsl_meta::parse_vec2_str)
            .map(|v| (v.x.max(1.0) as u32, v.y.max(1.0) as u32))
            .unwrap_or((512, 512))
    })
}

// --------------------------------------------------------------------------------------------------------------------
// Reporting. This is a CLI tool, so stdout IS the output.

fn print_stages(shader: &Unified) {
    println!("STAGES vertex={}  fragment={}  verts={}",
        shader.vertex_define.as_deref().unwrap_or("(built-in fullscreen triangle)"),
        shader.fragment_define.as_deref().unwrap_or("(none)"),
        shader.vertex_count);
    if shader.vertex_define.is_some() {
        println!("  vertex defines:   {}", defines_for(shader, shader.vertex_define.as_deref()).join(" "));
    }
    println!("  fragment defines: {}", defines_for(shader, shader.fragment_define.as_deref()).join(" "));
}

fn print_outputs(shader: &Unified) {
    println!("OUTPUTS:");
    for o in &shader.outputs {
        let roles: Vec<String> = o.binds.iter()
            .map(|b| format!("{}:{}", b.role.name(), b.spelling))
            .collect();
        let notes = if o.notes.is_empty() { String::new() } else { format!("  -- {}", o.notes) };
        println!("  location {}  {} {}  [{}]{}", o.location, ty_name(o.ty), o.name, roles.join(", "), notes);
    }
    let missing: Vec<&str> = ALL_ROLES.iter()
        .filter(|r| r.lights() && **r != Role::Data && !shader.roles.contains_key(r))
        .map(|r| r.name())
        .collect();
    if !missing.is_empty() {
        println!("  lit view falls back for: {}", missing.join(", "));
    }
}

fn print_blocks(shader: &Unified) {
    println!("UNIFORM BLOCKS:");
    for b in &shader.blocks {
        println!("  {} as {}   binding {}  group {}  {} bytes", b.name, b.instance, b.binding, b.group, b.size);
        for l in &b.leaves {
            let h = &l.hint;
            let mut parts = Vec::new();
            if let Some(f) = &h.format  { parts.push(format!("format={}", f)); }
            if let Some(d) = &h.default { parts.push(format!("default={}", d)); }
            if let Some(m) = h.min      { parts.push(format!("min={}", m)); }
            if let Some(m) = h.max      { parts.push(format!("max={}", m)); }
            if let Some(m) = h.step     { parts.push(format!("step={}", m)); }
            if let Some(a) = h.anim     { parts.push(format!("anim={}", a)); }
            if let Some(w) = &h.what    { parts.push(format!("what={}", w)); }
            if let Some(n) = &h.lanes   { parts.push(format!("lanes=[{}]", n.join(", "))); }
            let hints = if parts.is_empty() { String::new() } else { format!("  !({})", parts.join(", ")) };
            println!("    +{:<5} {:<6} {}{}", l.offset, ty_name(l.ty), l.key, hints);
        }
        for o in &b.opaques {
            println!("    +{:<5} {:<6} {}   ({} bytes, no control generated)", o.offset, o.type_name, o.key, o.size);
        }
    }
}

fn print_toggles(shader: &Unified) {
    if shader.toggles.is_empty() { return; }
    println!("TOGGLES (#ifdef defines the editor offers as checkboxes):");
    for t in &shader.toggles {
        println!("  {:<20} {:<4} {}", t.define, if t.on { "on" } else { "off" }, t.label);
    }
}

fn print_tuning(tune: &[TuneAssign]) {
    if tune.is_empty() { return; }
    println!("TUNING (!(tune=) assignments loaded as defaults, and what Save writes):");
    for a in tune {
        println!("  line {:<5} {} = {}", a.line + 1, a.key, a.raw);
    }
}

fn print_problems(problems: &[String]) {
    if problems.is_empty() { return; }
    println!("METADATA ISSUES:");
    for p in problems { println!("- {}", p); }
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
    vec2 res_px;   // !(format=resolution, default=[320,200])
} globals;
#ifdef FRAGMENT
layout(location = 0) out vec4 o_albedo;   // !(pbr=albedo:rgb)
layout(location = 1) out vec4 o_n;        // !(pbr=normal:xyz, pbr=height:w)
void main() {}
#endif
";

    fn cli() -> CliArgs { CliArgs::default() }

    #[test]
    fn a_resolution_member_sets_the_default_bake_size() {
        let shader = parse(SRC);
        assert!(shader.errors.is_empty(), "{:?}", shader.errors);
        assert_eq!(render_size(&cli(), &shader), (320, 200));
        let sized = CliArgs { size: Some((64, 32)), ..CliArgs::default() };
        assert_eq!(render_size(&sized, &shader), (64, 32));
    }

    #[test]
    fn channel_all_covers_the_lit_view_every_role_and_every_target() {
        let shader = parse(SRC);
        let c = CliArgs { channel: Some("all".into()), ..CliArgs::default() };
        let views = wanted_views(&c, &shader).expect("all");
        let names: Vec<&str> = views.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(names, vec!["lit", "albedo", "normal", "height", "raw0", "raw1"]);
    }

    #[test]
    fn one_view_writes_exactly_the_file_that_was_asked_for() {
        let shader = parse(SRC);
        let c = CliArgs { channel: Some("normal".into()), ..CliArgs::default() };
        let views = wanted_views(&c, &shader).expect("normal");
        assert_eq!(views.len(), 1);
        assert!(views[0].0.is_empty());
        assert_eq!(with_suffix(".output/t.png", ""), ".output/t.png");
        assert_eq!(with_suffix(".output/t.png", "albedo"), ".output/t.albedo.png");
        assert_eq!(with_suffix("t.png", "raw1"), "t.raw1.png");
    }

    #[test]
    fn a_channel_the_shader_does_not_write_is_an_error() {
        let shader = parse(SRC);
        let c = CliArgs { channel: Some("roughness".into()), ..CliArgs::default() };
        let e = wanted_views(&c, &shader).expect_err("no roughness");
        assert!(e.contains("declares no roughness role"), "{e}");

        let c = CliArgs { channel: Some("raw:5".into()), ..CliArgs::default() };
        let e = wanted_views(&c, &shader).expect_err("no target 5");
        assert!(e.contains("no output at that location"), "{e}");

        let c = CliArgs { channel: Some("sparkle".into()), ..CliArgs::default() };
        assert!(wanted_views(&c, &shader).is_err());
    }

    #[test]
    fn a_raw_target_is_selectable_by_either_spelling() {
        let shader = parse(SRC);
        for spelling in ["raw:1", "raw1"] {
            let c = CliArgs { channel: Some(spelling.into()), ..CliArgs::default() };
            let views = wanted_views(&c, &shader).expect(spelling);
            assert_eq!(views[0].1, ViewMode::Raw(1));
        }
    }
}
