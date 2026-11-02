// --------------------------------------------------------------------------------------------------------------------
// Tuning blocks: the #ifdef EDITOR / #else / #endif pairs that let a shader have live
// sliders here and plain consts everywhere else.
//
//   #ifdef EDITOR   // !(group="craters")
//   uniform float t_crater_freq;   // !(default=2.0, min=0.5, max=8)
//   #else
//   const float t_crater_freq = 0.5;
//   #endif
//
// The editor compiles the #ifdef arm, so the values are live uniforms it can drive. The
// project compiles the file as written and gets the #else consts. "Save tuning" writes the
// current slider values onto those consts, which is the only way a shader that may not grow
// its uniform set (a background pass, say) can be tuned by hand at all.

use std::sync::LazyLock;

use macroquad::prelude::*;
use regex::Regex;

use crate::glsl_meta::{split_top_level_commas, ty_name, unquote, GLSLType, UniformSpec};
use crate::values::UniformValues;

#[derive(Clone, Debug)]
pub struct TuningConst {
    pub name: String,
    pub ty: GLSLType,
    pub raw: String,       // value text as written, eg "0.5" or "vec2(0.16, 0.50)"
    pub line: usize,       // 0-based line index; Save rewrites in place
}

#[derive(Clone, Debug)]
pub struct EditorBlock {
    pub endif_line: usize,          // Save inserts missing consts just before this line
    pub group: Option<String>,      // from !(group="name"); named groups render as one section
    pub uniforms: Vec<String>,      // uniform names declared in the #ifdef arm
    pub consts: Vec<TuningConst>,   // consts found in the #else arm
}

/// Typed value of a tuning const, parsed out of its GLSL text.
pub enum ConstVal { F(f32), V2(Vec2), V3(Vec3), V4(Vec4), I(i32), U(u32) }

static UNIFORM_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*uniform\s+(\w+)\s+([A-Za-z_]\w*)\s*;"#).expect("literal regex"));
static CONST_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*const\s+(\w+)\s+([A-Za-z_]\w*)\s*=\s*([^;]+);"#).expect("literal regex"));

// --------------------------------------------------------------------------------------------------------------------
// Recognising and enabling the blocks.

/// Only the exact `#ifdef EDITOR` guard counts, not `#if defined(EDITOR)`.
pub fn is_ifdef_editor(line: &str) -> bool {
    match line.trim_start().strip_prefix("#ifdef") {
        Some(rest) => rest.starts_with(char::is_whitespace)
            && rest.trim_start().split_whitespace().next() == Some("EDITOR"),
        None => false,
    }
}

/// Take the #ifdef arm, by swapping the guard line rather than prepending a #define.
/// Same line count either way, so GLSL compile errors still point at the file on disk.
pub fn enable_editor_blocks(src: &str) -> String {
    src.lines()
        .map(|l| if is_ifdef_editor(l) { "#if 1 // #ifdef EDITOR (enabled by shader_editor)" } else { l })
        .collect::<Vec<&str>>()
        .join("\n")
}

/// Read the optional `// !(group="name")` hint off an #ifdef EDITOR guard line.
fn parse_block_hint(line: &str, line_no: usize) -> (Option<String>, Vec<String>) {
    let mut group = None;
    let mut errs = Vec::new();
    let Some(inner) = line.split("!(").nth(1).and_then(|rest| rest.split(')').next()) else {
        return (None, errs);
    };
    for raw in split_top_level_commas(inner) {
        let token = raw.trim();
        if token.is_empty() { continue; }
        match token.split_once('=') {
            Some((k, v)) if k.trim().eq_ignore_ascii_case("group") => {
                let name = unquote(v).to_string();
                if name.is_empty() {
                    errs.push(format!("EDITOR block at line {}: group name is empty", line_no));
                } else {
                    group = Some(name);
                }
            }
            _ => errs.push(format!("EDITOR block at line {}: unknown hint '{}' (only group=\"name\" here)", line_no, token)),
        }
    }
    (group, errs)
}

/// Every #ifdef EDITOR block in the source, with the uniforms it declares and the consts it saves to.
pub fn parse_editor_blocks(glsl: &str) -> (Vec<EditorBlock>, Vec<String>) {
    let lines: Vec<&str> = glsl.lines().collect();
    let mut blocks = Vec::new();
    let mut errs = Vec::new();

    let mut i = 0;
    while i < lines.len() {
        if !is_ifdef_editor(lines[i]) { i += 1; continue; }

        // find this block's #else / #endif, stepping over any nested #if...#endif pair
        let (mut else_line, mut endif_line) = (None, None);
        let mut depth = 0usize;
        let mut j = i + 1;
        while j < lines.len() {
            let t = lines[j].trim_start();
            if t.starts_with("#if") {
                depth += 1;
            } else if t.starts_with("#endif") {
                if depth == 0 { endif_line = Some(j); break; }
                depth -= 1;
            } else if t.starts_with("#else") && depth == 0 {
                else_line = Some(j);
            }
            j += 1;
        }
        let Some(endif) = endif_line else {
            errs.push(format!("EDITOR block at line {}: no matching #endif", i + 1));
            break;
        };
        let Some(els) = else_line else {
            errs.push(format!("EDITOR block at line {}: needs an #else arm holding the consts", i + 1));
            i = endif + 1;
            continue;
        };

        let (group, mut hint_errs) = parse_block_hint(lines[i], i + 1);
        errs.append(&mut hint_errs);
        let mut block = EditorBlock { endif_line: endif, group, uniforms: Vec::new(), consts: Vec::new() };

        for k in (i + 1)..els {
            if let Some(cap) = UNIFORM_LINE_RE.captures(lines[k]) {
                block.uniforms.push(cap[2].to_string());
            }
        }
        for k in (els + 1)..endif {
            let Some(cap) = CONST_LINE_RE.captures(lines[k]) else { continue };
            let ty = match &cap[1] {
                "float" => GLSLType::Float,
                "vec2"  => GLSLType::Vec2,
                "vec3"  => GLSLType::Vec3,
                "vec4"  => GLSLType::Vec4,
                "int"   => GLSLType::Int,
                "uint"  => GLSLType::Uint,
                other => {
                    errs.push(format!("tuning const {}: unsupported type '{}'", &cap[2], other));
                    continue;
                }
            };
            block.consts.push(TuningConst { name: cap[2].to_string(), ty, raw: cap[3].trim().to_string(), line: k });
        }
        blocks.push(block);
        i = endif + 1;
    }
    (blocks, errs)
}

// --------------------------------------------------------------------------------------------------------------------
// Reading and writing the const values.

fn parse_glsl_float(t: &str) -> Option<f32> {
    t.trim().trim_end_matches(['f', 'F']).parse::<f32>().ok()
}

/// Parse "vecN(a, b, ...)". A single argument splats, as it does in GLSL.
fn parse_vec_ctor(raw: &str, n: usize) -> Option<Vec4> {
    let raw = raw.trim();
    let open = raw.find('(')?;
    let close = raw.rfind(')')?;
    if raw[..open].trim() != format!("vec{}", n) { return None; }
    let nums = raw[open + 1..close].split(',').map(parse_glsl_float).collect::<Option<Vec<f32>>>()?;
    let nums = if nums.len() == 1 { vec![nums[0]; n] } else { nums };
    if nums.len() != n { return None; }
    let mut v = [0.0f32; 4];
    v[..n].copy_from_slice(&nums);
    Some(vec4(v[0], v[1], v[2], v[3]))
}

/// A typed value out of GLSL value text: what sits between the '=' and the ';'.
/// Shared with the unified path, whose tune regions hold assignments rather than consts.
pub fn parse_glsl_value(ty: GLSLType, raw: &str) -> Option<ConstVal> {
    match ty {
        GLSLType::Float => parse_glsl_float(raw).map(ConstVal::F),
        GLSLType::Vec2  => parse_vec_ctor(raw, 2).map(|v| ConstVal::V2(vec2(v.x, v.y))),
        GLSLType::Vec3  => parse_vec_ctor(raw, 3).map(|v| ConstVal::V3(vec3(v.x, v.y, v.z))),
        GLSLType::Vec4  => parse_vec_ctor(raw, 4).map(ConstVal::V4),
        GLSLType::Int   => raw.trim().parse::<i32>().ok().map(ConstVal::I),
        GLSLType::Uint  => raw.trim().trim_end_matches(['u', 'U']).parse::<u32>().ok().map(ConstVal::U),
        GLSLType::Sampler2D => None,
    }
}

pub fn parse_const_value(c: &TuningConst) -> Option<ConstVal> {
    parse_glsl_value(c.ty, &c.raw)
}

/// Cross-check the #ifdef uniforms against the #else consts.
///   - A uniform with no const is a warning; Save creates it.
///   - A const with no uniform, a type mismatch, or an unparsable value is an error: the
///     project's own compile is what is at stake, and it will not be tested here.
pub fn check_editor_blocks(specs: &[UniformSpec], blocks: &[EditorBlock]) -> Vec<String> {
    let mut errs = Vec::new();
    for b in blocks {
        for uname in &b.uniforms {
            let Some(spec) = specs.iter().find(|s| &s.name == uname) else { continue };
            if spec.ty == GLSLType::Sampler2D {
                errs.push(format!("tuning uniform {}: sampler2D cannot be saved as a const", uname));
                continue;
            }
            match b.consts.iter().find(|c| &c.name == uname) {
                None => errs.push(format!("warning: tuning uniform {} has no const in the #else arm (Save will create it)", uname)),
                Some(c) if c.ty != spec.ty => errs.push(format!(
                    "tuning const {}: type {} does not match uniform type {}", uname, ty_name(c.ty), ty_name(spec.ty))),
                Some(c) if parse_const_value(c).is_none() => {
                    errs.push(format!("tuning const {}: cannot parse value '{}'", uname, c.raw));
                }
                Some(_) => {}
            }
        }
        for c in &b.consts {
            if !b.uniforms.iter().any(|u| u == &c.name) {
                errs.push(format!("tuning const {}: no matching uniform in the #ifdef EDITOR arm", c.name));
            }
        }
    }
    errs
}

/// Seed tuning values from the #else consts. or_insert on purpose: a saved const beats the
/// default= hint (seeded after this), but a live slider tweak survives a reload.
pub fn seed_const_values(values: &mut UniformValues, blocks: &[EditorBlock]) {
    for c in blocks.iter().flat_map(|b| b.consts.iter()) {
        match parse_const_value(c) {
            Some(ConstVal::F(v))  => { values.float.entry(c.name.clone()).or_insert(v); }
            Some(ConstVal::V2(v)) => { values.vec2.entry(c.name.clone()).or_insert(v); }
            Some(ConstVal::V3(v)) => { values.vec3.entry(c.name.clone()).or_insert(v); }
            Some(ConstVal::V4(v)) => { values.vec4.entry(c.name.clone()).or_insert(v); }
            Some(ConstVal::I(v))  => { values.int.entry(c.name.clone()).or_insert(v); }
            Some(ConstVal::U(v))  => { values.uint.entry(c.name.clone()).or_insert(v); }
            None => {}
        }
    }
}

/// The current editor value of a tuning uniform, as GLSL value text.
/// Shared with the unified path's tune regions, which write the same text after a '='.
pub fn fmt_glsl_value(ty: GLSLType, name: &str, values: &UniformValues) -> Option<String> {
    // {:?} on f32 is the shortest text that round-trips, and always keeps a '.' or an 'e'
    fn f(v: f32) -> String { format!("{:?}", v) }
    Some(match ty {
        GLSLType::Float => f(*values.float.get(name)?),
        GLSLType::Vec2  => { let v = values.vec2.get(name)?; format!("vec2({}, {})", f(v.x), f(v.y)) }
        GLSLType::Vec3  => { let v = values.vec3.get(name)?; format!("vec3({}, {}, {})", f(v.x), f(v.y), f(v.z)) }
        GLSLType::Vec4  => { let v = values.vec4.get(name)?; format!("vec4({}, {}, {}, {})", f(v.x), f(v.y), f(v.z), f(v.w)) }
        GLSLType::Int   => format!("{}", values.int.get(name)?),
        GLSLType::Uint  => format!("{}u", values.uint.get(name)?),
        GLSLType::Sampler2D => return None,
    })
}

/// Rewrite the #else consts of every EDITOR block with the current editor values.
///   - Only the text between '=' and ';' of a matched line changes, so trailing comments
///     and the name-column alignment survive.
///   - A tuning uniform with no const gets one appended just above the #endif.
///   - Line endings are preserved, CRLF included.
/// Pure text transform, so it is unit tested rather than tested by looking at a file.
/// Returns the new source and how many values were written.
pub fn write_tuning_consts(src: &str, specs: &[UniformSpec], values: &UniformValues) -> Result<(String, usize), String> {
    let (blocks, _) = parse_editor_blocks(src);
    if blocks.is_empty() {
        return Err("no #ifdef EDITOR tuning block in this shader".to_string());
    }

    let crlf = src.contains("\r\n");
    let trailing_nl = src.ends_with('\n');
    let mut lines: Vec<String> = src.lines().map(str::to_string).collect();

    let mut saved = 0usize;
    let mut appends: Vec<(usize, Vec<String>)> = Vec::new();

    for b in &blocks {
        let mut new_lines = Vec::new();
        for uname in &b.uniforms {
            let Some(spec) = specs.iter().find(|s| &s.name == uname) else { continue };
            let Some(value) = fmt_glsl_value(spec.ty, uname, values) else { continue };
            match b.consts.iter().find(|c| &c.name == uname) {
                Some(c) => {
                    if c.ty != spec.ty { continue; }   // already an error in the log; do not write a broken const
                    let line = lines[c.line].clone();
                    let Some(eq) = line.find('=') else { continue };
                    let Some(semi) = line[eq..].find(';').map(|s| eq + s) else { continue };
                    lines[c.line] = format!("{}= {}{}", &line[..eq], value, &line[semi..]);
                }
                None => new_lines.push(format!("const {} {} = {};", ty_name(spec.ty), uname, value)),
            }
            saved += 1;
        }
        if !new_lines.is_empty() { appends.push((b.endif_line, new_lines)); }
    }

    // bottom-up, so the earlier blocks' line indices stay valid as rows are inserted
    for (endif, new_lines) in appends.into_iter().rev() {
        for (k, text) in new_lines.into_iter().enumerate() {
            lines.insert(endif + k, text);
        }
    }

    let eol = if crlf { "\r\n" } else { "\n" };
    let mut out = lines.join(eol);
    if trailing_nl { out.push_str(eol); }
    Ok((out, saved))
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
#version 330
#ifdef EDITOR
uniform float t_freq;    // !(default=2.0, min=0.5, max=8)
uniform vec2  t_rad;     // !(default=[0.1,0.5])
uniform float t_new;     // !(default=1.0)
#else
// editor saved
const float t_freq   = 0.5;   // keep this comment
const vec2  t_rad    = vec2(0.16, 0.50);
#endif
void main() {}
";

    #[test]
    fn parses_block_uniforms_and_consts() {
        let (blocks, errs) = parse_editor_blocks(SRC);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].uniforms, vec!["t_freq", "t_rad", "t_new"]);
        assert_eq!(blocks[0].consts.len(), 2);
        assert!(matches!(parse_const_value(&blocks[0].consts[0]), Some(ConstVal::F(v)) if v == 0.5));
        assert!(matches!(parse_const_value(&blocks[0].consts[1]), Some(ConstVal::V2(v)) if v == vec2(0.16, 0.50)));
    }

    #[test]
    fn missing_const_warns_and_orphan_const_errors() {
        let (specs, _) = parse_uniforms(SRC);
        let (blocks, _) = parse_editor_blocks(SRC);
        let errs = check_editor_blocks(&specs, &blocks);
        assert_eq!(errs.len(), 1, "{errs:?}");
        assert!(errs[0].contains("t_new") && errs[0].contains("warning"), "{errs:?}");

        let orphan_src = SRC.replace("// editor saved", "const float t_orphan = 1.0;");
        let (specs, _) = parse_uniforms(&orphan_src);
        let (blocks, _) = parse_editor_blocks(&orphan_src);
        let errs = check_editor_blocks(&specs, &blocks);
        assert!(errs.iter().any(|e| e.contains("t_orphan") && e.contains("no matching uniform")), "{errs:?}");
    }

    #[test]
    fn save_rewrites_in_place_and_appends_missing() {
        let (specs, _) = parse_uniforms(SRC);
        let mut values = UniformValues::default();
        values.float.insert("t_freq".into(), 6.25);
        values.vec2.insert("t_rad".into(), vec2(0.2, 0.3));
        values.float.insert("t_new".into(), 1.5);

        let (out, saved) = write_tuning_consts(SRC, &specs, &values).expect("a tuning block");
        assert_eq!(saved, 3);
        // value replaced in place; alignment prefix and trailing comment intact
        assert!(out.contains("const float t_freq   = 6.25;   // keep this comment"), "{out}");
        assert!(out.contains("const vec2  t_rad    = vec2(0.2, 0.3);"), "{out}");
        // the const-less uniform gets created just above #endif
        assert!(out.contains("const float t_new = 1.5;\n#endif"), "{out}");
        // the default= hints are never rewritten
        assert!(out.contains("// !(default=2.0, min=0.5, max=8)"), "{out}");
    }

    #[test]
    fn a_crlf_file_stays_crlf() {
        let (specs, _) = parse_uniforms(SRC);
        let mut values = UniformValues::default();
        values.float.insert("t_new".into(), 1.5);

        let crlf_src = SRC.replace('\n', "\r\n");
        let (out, _) = write_tuning_consts(&crlf_src, &specs, &values).expect("a tuning block");
        assert!(!out.replace("\r\n", "").contains('\n'), "mixed line endings");
        assert!(out.contains("const float t_new = 1.5;\r\n#endif"), "{out}");
    }

    #[test]
    fn block_group_hint() {
        let src = SRC.replace("#ifdef EDITOR", "#ifdef EDITOR   // !(group=\"craters\")");
        let (blocks, errs) = parse_editor_blocks(&src);
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(blocks[0].group.as_deref(), Some("craters"));
        // the guard swap still recognises the hinted line
        assert!(enable_editor_blocks(&src).lines().nth(1).expect("line 2").starts_with("#if 1"));

        let bad = SRC.replace("#ifdef EDITOR", "#ifdef EDITOR // !(grp=1)");
        let (blocks, errs) = parse_editor_blocks(&bad);
        assert_eq!(blocks[0].group, None);
        assert!(errs.iter().any(|e| e.contains("unknown hint")), "{errs:?}");
    }

    #[test]
    fn editor_arm_is_enabled_without_shifting_lines() {
        let enabled = enable_editor_blocks(SRC);
        assert_eq!(enabled.lines().count(), SRC.lines().count());
        assert!(enabled.lines().nth(1).expect("line 2").starts_with("#if 1"), "{enabled}");
        assert!(!enabled.lines().any(is_ifdef_editor));
    }

    #[test]
    fn a_block_with_no_else_arm_is_an_error() {
        let src = "#ifdef EDITOR\nuniform float t_a;\n#endif\n";
        let (blocks, errs) = parse_editor_blocks(src);
        assert!(blocks.is_empty());
        assert!(errs.iter().any(|e| e.contains("needs an #else arm")), "{errs:?}");
    }
}
