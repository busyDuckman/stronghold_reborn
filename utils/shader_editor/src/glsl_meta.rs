// --------------------------------------------------------------------------------------------------------------------
// Uniform metadata: what the editor learns about a shader by reading its uniform comments.
//
// A uniform declares its own control by what follows it on the line:
//   uniform float u_thickness_px;  // !(default=3.0, min=1, max=3.1415) some notes
//
// Everything inside !(...) is key=value; anything after the closing paren is notes shown
// under the control. Keys: format, default, min, max, what, size, anim, step, lanes, names, group.
// Formats: TIME, RGB, RGBA, RESOLUTION, INTEGRAL, MAT, TEXEL.
//
// The same !(...) form annotates everything the editor reads, not just uniforms: a uniform
// block member, a fragment output's PBR role, an #ifdef guard's stage or toggle. hint_tokens
// below is the one tokeniser they all share; each caller then accepts the keys that mean
// something where it sits, and reports the rest.

use std::sync::LazyLock;

use macroquad::prelude::*;
use regex::Regex;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GLSLType { Float, Vec2, Vec3, Vec4, Int, Uint, Sampler2D }

#[derive(Clone, Debug, Default)]
pub struct Hint {
    pub format: Option<String>,     // TIME, RGB, RGBA, MAT, RESOLUTION, INTEGRAL, TEXEL
    pub default: Option<String>,    // raw token, eg "3.0" or "#ff00aa80"
    pub min: Option<f32>,
    pub max: Option<f32>,
    pub step: Option<f32>,          // quantise the control, for a value the shader needs whole
    pub what: Option<String>,       // source uniform for format=integral (accumulates what * dt)
    pub size: Option<(u32, u32)>,   // pixel size for generated sampler stand-ins
    pub anim: Option<f32>,          // seconds per min->max sweep for the auto-lerp control
    pub lanes: Option<Vec<String>>, // names for a vecN's components, in place of x/y/z/w
    pub names: Option<Vec<String>>, // names for an array's elements, eg the seven terrain types
    pub group: Option<String>,      // fold this member into a named, collapsing section
    pub notes: String,              // the rest of the comment, shown under the control
}

#[derive(Clone, Debug)]
pub struct UniformSpec {
    pub name: String,
    pub ty: GLSLType,
    pub hint: Hint,
}

/// One key=value inside a !(...) block. `bare` marks a token that had no '=' at all, which
/// is always a mistake but is the caller's to report, since only it knows what was expected.
#[derive(Clone, Debug)]
pub struct HintToken {
    pub key: String,
    pub value: String,
    pub bare: bool,
}

// [ \t]* rather than \s* before the comment group: \s eats the newline and slurps the NEXT
// line's standalone comment into this uniform's notes.
static UNIFORM_RE: LazyLock<Regex> = LazyLock::new(|| {
    const PAT: &str = r#"(?m)^\s*uniform\s+(float|vec2|vec3|vec4|uint|sampler2D)\s+([A-Za-z_]\w*)\s*;[ \t]*(?://[ \t]*(.*))?$"#;
    Regex::new(PAT).expect("uniform regex is a literal")
});

pub fn ty_name(t: GLSLType) -> &'static str {
    match t {
        GLSLType::Float => "float",
        GLSLType::Vec2  => "vec2",
        GLSLType::Vec3  => "vec3",
        GLSLType::Vec4  => "vec4",
        GLSLType::Int   => "int",
        GLSLType::Uint  => "uint",
        GLSLType::Sampler2D => "sampler2D",
    }
}

/// The GLSL spelling of a type, back to the enum. Used where a declaration is parsed rather
/// than matched by regex: block members, struct fields, fragment outputs.
pub fn ty_from_name(s: &str) -> Option<GLSLType> {
    match s.trim() {
        "float" => Some(GLSLType::Float),
        "vec2"  => Some(GLSLType::Vec2),
        "vec3"  => Some(GLSLType::Vec3),
        "vec4"  => Some(GLSLType::Vec4),
        "int"   => Some(GLSLType::Int),
        "uint"  => Some(GLSLType::Uint),
        "sampler2D" => Some(GLSLType::Sampler2D),
        _ => None,
    }
}

/// How many floats a type occupies in a control row. Samplers have none.
pub fn ty_components(t: GLSLType) -> usize {
    match t {
        GLSLType::Float | GLSLType::Int | GLSLType::Uint => 1,
        GLSLType::Vec2 => 2,
        GLSLType::Vec3 => 3,
        GLSLType::Vec4 => 4,
        GLSLType::Sampler2D => 0,
    }
}

pub fn matches_fmt(h: &Hint, want: &str) -> bool {
    h.format.as_deref().map(|f| f.eq_ignore_ascii_case(want)).unwrap_or(false)
}

// --------------------------------------------------------------------------------------------------------------------
// Value parsing. Every default= token arrives as text and is checked against the uniform's type.

pub fn parse_hex_rgb(s: &str) -> Option<Vec3> {
    let t = s.trim_start_matches('#');
    if t.len() != 6 { return None; }
    let r = u8::from_str_radix(&t[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&t[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&t[4..6], 16).ok()? as f32 / 255.0;
    Some(vec3(r, g, b))
}

pub fn parse_hex_rgba(s: &str) -> Option<Vec4> {
    let t = s.trim_start_matches('#');
    if t.len() != 8 { return None; }
    let r = u8::from_str_radix(&t[0..2], 16).ok()? as f32 / 255.0;
    let g = u8::from_str_radix(&t[2..4], 16).ok()? as f32 / 255.0;
    let b = u8::from_str_radix(&t[4..6], 16).ok()? as f32 / 255.0;
    let a = u8::from_str_radix(&t[6..8], 16).ok()? as f32 / 255.0;
    Some(vec4(r, g, b, a))
}

/// Parse exactly N numbers out of a loose list.
///   - Accepts "1.0", "1.0 2.0", "1,2,3,4" and "[1,2]".
///   - Returns None on the wrong count, so a bad default is reported rather than half-read.
pub fn parse_vecn<const N: usize>(s: &str) -> Option<Vec4> {
    let clean = s.replace(['[', ']', ','], " ");
    let mut nums: Vec<f32> = clean.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    if nums.len() != N { return None; }
    nums.resize(4, 0.0);
    Some(vec4(nums[0], nums[1], nums[2], nums[3]))
}

pub fn parse_vec2_str(s: &str) -> Option<Vec2> { parse_vecn::<2>(s).map(|v| vec2(v.x, v.y)) }
pub fn parse_vec3_str(s: &str) -> Option<Vec3> { parse_vecn::<3>(s).map(|v| vec3(v.x, v.y, v.z)) }
pub fn parse_vec4_str(s: &str) -> Option<Vec4> { parse_vecn::<4>(s) }

/// Strip one layer of double quotes, if present.
pub fn unquote(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') { &s[1..s.len() - 1] } else { s }
}

/// Split on commas, but not commas inside [...] or "...".
/// So default=[512,128], min=0 and default=text:"Hello, world" both stay intact.
pub fn split_top_level_commas(inner: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut in_quotes = false;
    let mut start = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            '[' if !in_quotes => depth += 1,
            ']' if !in_quotes => depth = depth.saturating_sub(1),
            ',' if !in_quotes && depth == 0 => { out.push(&inner[start..i]); start = i + 1; }
            _ => {}
        }
    }
    out.push(&inner[start..]);
    out
}

/// A bracketed list of bare words: `[grass, rocks, snow]` -> three names.
/// Empty entries are dropped rather than kept as "", so a trailing comma is harmless.
pub fn parse_name_list(s: &str) -> Option<Vec<String>> {
    let t = s.trim();
    let inner = t.strip_prefix('[')?.strip_suffix(']')?;
    let names: Vec<String> = split_top_level_commas(inner)
        .into_iter()
        .map(|p| unquote(p).trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    if names.is_empty() { None } else { Some(names) }
}

// --------------------------------------------------------------------------------------------------------------------
// The shared !(...) tokeniser.

/// The key=value tokens inside the comment's !(...) block, or None if it has no block.
/// Keys come back lowercased; values keep their case, because names and paths are values.
pub fn hint_tokens(comment: &str) -> Option<Vec<HintToken>> {
    let inner = comment.split("!(").nth(1).and_then(|rest| rest.split(')').next())?;
    let mut out = Vec::new();
    for raw in split_top_level_commas(inner) {
        let token = raw.trim();
        if token.is_empty() { continue; }
        match token.split_once('=') {
            Some((k, v)) => out.push(HintToken {
                key: k.trim().to_ascii_lowercase(),
                value: v.trim().to_string(),
                bare: false,
            }),
            None => out.push(HintToken { key: token.to_string(), value: String::new(), bare: true }),
        }
    }
    Some(out)
}

/// Whatever follows the !(...) block, minus a leading dash if the author used one.
/// A comment with no block is all notes.
pub fn hint_notes(comment: &str) -> String {
    match comment.find("!(") {
        Some(idx) => {
            let after = comment[idx..].find(')').map(|i| idx + i + 1).unwrap_or(comment.len());
            let mut tail = comment[after..].trim().to_string();
            if tail.starts_with('-') { tail.remove(0); }
            tail.trim().to_string()
        }
        None => comment.trim().to_string(),
    }
}

/// Read one control-shaping token into a Hint. Returns false for a key this does not own, so
/// a caller with extra keys of its own (a guard's stage=, an output's pbr=) can take those
/// first and let everything else fall through to here.
/// `whose` names the thing being annotated, for the error text.
pub fn apply_control_token(hint: &mut Hint, tok: &HintToken, whose: &str, errs: &mut Vec<String>) -> bool {
    let num = |v: &str, what: &str, errs: &mut Vec<String>| -> Option<f32> {
        match v.parse::<f32>() {
            Ok(x) => Some(x),
            Err(_) => { errs.push(format!("{}: {} '{}' is not a number", whose, what, v)); None }
        }
    };
    match tok.key.as_str() {
        "format"  => hint.format = Some(tok.value.clone()),
        "default" => hint.default = Some(tok.value.clone()),
        "what"    => hint.what = Some(tok.value.clone()),
        "group"   => {
            let name = unquote(&tok.value).to_string();
            if name.is_empty() { errs.push(format!("{}: group name is empty", whose)); } else { hint.group = Some(name); }
        }
        "min"  => hint.min = num(&tok.value, "min", errs),
        "max"  => hint.max = num(&tok.value, "max", errs),
        "anim" => hint.anim = num(&tok.value, "anim", errs),
        "step" => match num(&tok.value, "step", errs) {
            Some(x) if x > 0.0 => hint.step = Some(x),
            Some(x) => errs.push(format!("{}: step '{}' must be > 0", whose, x)),
            None => {}
        },
        "size" => match parse_vec2_str(&tok.value) {
            Some(v) if v.x >= 1.0 && v.y >= 1.0 => hint.size = Some((v.x as u32, v.y as u32)),
            _ => errs.push(format!("{}: size '{}' is not [w,h]", whose, tok.value)),
        },
        "lanes" => match parse_name_list(&tok.value) {
            Some(n) => hint.lanes = Some(n),
            None => errs.push(format!("{}: lanes '{}' is not [a, b, ...]", whose, tok.value)),
        },
        "names" => match parse_name_list(&tok.value) {
            Some(n) => hint.names = Some(n),
            None => errs.push(format!("{}: names '{}' is not [a, b, ...]", whose, tok.value)),
        },
        _ => return false,
    }
    true
}

// --------------------------------------------------------------------------------------------------------------------
// Parsing the uniform declarations themselves.

/// Read the !(...) block plus trailing notes off one uniform comment.
/// Unknown keys and unparsable numbers are reported rather than ignored, so a typo in a
/// hint shows up in the editor's log instead of silently doing nothing.
fn parse_hint_from_comment(comment: &str, uniform: &str) -> (Hint, Vec<String>) {
    let mut hint = Hint::default();
    let mut errs = Vec::new();
    let whose = format!("uniform {}", uniform);

    for tok in hint_tokens(comment).unwrap_or_default() {
        if tok.bare {
            errs.push(format!("uniform {}: unknown flag '{}' (want key=value)", uniform, tok.key));
            continue;
        }
        if !apply_control_token(&mut hint, &tok, &whose, &mut errs) {
            errs.push(format!("uniform {}: unknown hint key '{}'", uniform, tok.key));
        }
    }

    // notes are whatever follows the !(...) block, minus a leading dash if the author used one
    hint.notes = hint_notes(comment);
    (hint, errs)
}

/// Check one default= token against the type it was declared on.
pub fn check_default(ty: GLSLType, hint: &Hint, name: &str, errs: &mut Vec<String>) {
    let Some(def) = hint.default.as_deref() else { return };
    let bad = |what: &str| format!("uniform {}: default '{}' is not {}", name, def, what);
    match ty {
        GLSLType::Float => if def.parse::<f32>().is_err() { errs.push(bad("a float")); },
        GLSLType::Int   => if def.parse::<i32>().is_err() { errs.push(bad("an int")); },
        GLSLType::Uint  => if def.parse::<u32>().is_err() { errs.push(bad("a uint")); },
        GLSLType::Vec2  => if !matches_fmt(hint, "resolution") && !matches_fmt(hint, "texel")
            && parse_vec2_str(def).is_none() {
            errs.push(bad("a vec2"));
        },
        GLSLType::Vec3 => if matches_fmt(hint, "rgb") {
            if parse_hex_rgb(def).is_none() { errs.push(bad("#RRGGBB")); }
        } else if parse_vec3_str(def).is_none() {
            errs.push(bad("a vec3"));
        },
        GLSLType::Vec4 => if matches_fmt(hint, "rgba") {
            if parse_hex_rgba(def).is_none() { errs.push(bad("#RRGGBBAA")); }
        } else if parse_vec4_str(def).is_none() {
            errs.push(bad("a vec4"));
        },
        GLSLType::Sampler2D => if let Err(e) = crate::standins::parse_tex_source(def) {
            errs.push(format!("uniform {}: {}", name, e));
        },
    }
}

/// anim= drives a value on its own, so it needs a float with both ends of a range, and it
/// cannot sit on a uniform whose format already owns the motion.
fn check_anim(ty: GLSLType, hint: &Hint, name: &str, errs: &mut Vec<String>) {
    let Some(a) = hint.anim else { return };
    if a <= 0.0 {
        errs.push(format!("uniform {}: anim '{}' must be > 0 (seconds per min->max sweep)", name, a));
    }
    if !matches!(ty, GLSLType::Float) {
        errs.push(format!("uniform {}: anim= only works on float uniforms", name));
    } else if matches_fmt(hint, "time") || matches_fmt(hint, "integral") {
        let fmt = hint.format.as_deref().unwrap_or("?");
        errs.push(format!("uniform {}: anim= cannot combine with format={}", name, fmt));
    } else if hint.min.is_none() || hint.max.is_none() {
        errs.push(format!("uniform {}: anim= needs both min= and max= to lerp between", name));
    }
}

/// format=integral accumulates another uniform, so its what= has to name a real float.
/// Checked after the whole file is parsed, when every uniform name is known.
fn check_integrals(specs: &[UniformSpec], errs: &mut Vec<String>) {
    for s in specs.iter().filter(|s| matches_fmt(&s.hint, "integral")) {
        if !matches!(s.ty, GLSLType::Float) {
            errs.push(format!("uniform {}: format=integral only works on float uniforms", s.name));
        }
        match s.hint.what.as_deref() {
            None => errs.push(format!("uniform {}: format=integral requires what=<uniform>", s.name)),
            Some(w) if w == s.name => errs.push(format!("uniform {}: what= must not reference itself", s.name)),
            Some(w) if !specs.iter().any(|o| o.name == w && matches!(o.ty, GLSLType::Float)) => {
                errs.push(format!("uniform {}: what '{}' is not a declared float uniform", s.name, w));
            }
            Some(_) => {}
        }
    }
}

/// Every uniform declaration in the source, with its metadata and any problems found.
pub fn parse_uniforms(glsl: &str) -> (Vec<UniformSpec>, Vec<String>) {
    let mut specs = Vec::new();
    let mut errors = Vec::new();

    for cap in UNIFORM_RE.captures_iter(glsl) {
        let ty = match &cap[1] {
            "float"     => GLSLType::Float,
            "vec2"      => GLSLType::Vec2,
            "vec3"      => GLSLType::Vec3,
            "vec4"      => GLSLType::Vec4,
            "uint"      => GLSLType::Uint,
            "sampler2D" => GLSLType::Sampler2D,
            _ => continue,
        };
        let name = cap[2].to_string();
        let comment = cap.get(3).map(|m| m.as_str()).unwrap_or("").trim().to_string();

        let (hint, mut hint_errs) = parse_hint_from_comment(&comment, &name);
        errors.append(&mut hint_errs);
        check_default(ty, &hint, &name, &mut errors);
        check_anim(ty, &hint, &name, &mut errors);

        specs.push(UniformSpec { name, ty, hint });
    }

    check_integrals(&specs, &mut errors);
    (specs, errors)
}

// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// --------------------------------------------------------------------------------------------------------------------
// Tests

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a one-uniform source and return its spec plus any metadata errors.
    fn one(src: &str) -> (UniformSpec, Vec<String>) {
        let (specs, errs) = parse_uniforms(src);
        (specs.into_iter().next().expect("a uniform"), errs)
    }

    #[test]
    fn reads_hints_and_notes() {
        let (s, errs) = one("uniform float u_p; // !(min=0.0, max=1.0, anim=5.0) note\n");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(s.hint.anim, Some(5.0));
        assert_eq!(s.hint.min, Some(0.0));
        assert_eq!(s.hint.notes, "note");
    }

    #[test]
    fn notes_do_not_slurp_the_next_line() {
        let src = "uniform float u_a; // first\n// a standalone comment\nuniform float u_b;\n";
        let (specs, _) = parse_uniforms(src);
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].hint.notes, "first");
    }

    #[test]
    fn commas_inside_brackets_and_quotes_survive() {
        let parts = split_top_level_commas(r#"default=text:"Hello, world", size=[512,128], min=0"#);
        assert_eq!(parts.len(), 3, "{parts:?}");
        assert!(parts[0].contains("Hello, world"));
        assert!(parts[1].contains("[512,128]"));
    }

    #[test]
    fn bad_default_is_reported_against_its_type() {
        let (_s, errs) = one("uniform vec3 u_c; // !(format=RGB, default=#nothex)\n");
        assert!(errs.iter().any(|e| e.contains("#RRGGBB")), "{errs:?}");
    }

    #[test]
    fn unknown_hint_key_is_reported() {
        let (_s, errs) = one("uniform float u_p; // !(mn=0.0)\n");
        assert!(errs.iter().any(|e| e.contains("unknown hint key")), "{errs:?}");
    }

    #[test]
    fn anim_needs_a_float_and_a_range() {
        let (_s, errs) = one("uniform float u_p; // !(anim=5.0)\n");
        assert!(errs.iter().any(|e| e.contains("needs both min= and max=")), "{errs:?}");

        let (_s, errs) = one("uniform vec2 u_v; // !(min=0, max=1, anim=5)\n");
        assert!(errs.iter().any(|e| e.contains("only works on float")), "{errs:?}");

        let (_s, errs) = one("uniform float u_p; // !(min=0, max=1, anim=0)\n");
        assert!(errs.iter().any(|e| e.contains("must be > 0")), "{errs:?}");
    }

    #[test]
    fn anim_conflicts_with_time_and_integral() {
        let (_s, errs) = one("uniform float u_t; // !(format=time, min=0, max=1, anim=5)\n");
        assert!(errs.iter().any(|e| e.contains("cannot combine with format")), "{errs:?}");
    }

    #[test]
    fn integral_what_must_name_a_declared_float() {
        let src = "uniform float u_acc; // !(format=integral, what=u_missing)\n";
        let (_specs, errs) = parse_uniforms(src);
        assert!(errs.iter().any(|e| e.contains("is not a declared float uniform")), "{errs:?}");

        let src = "uniform float u_acc; // !(format=integral, what=u_acc)\n";
        let (_specs, errs) = parse_uniforms(src);
        assert!(errs.iter().any(|e| e.contains("must not reference itself")), "{errs:?}");
    }

    #[test]
    fn tokeniser_separates_keys_values_and_bare_flags() {
        let toks = hint_tokens("// !(min=0, oops, group=\"a b\") tail").expect("a block");
        assert_eq!(toks.len(), 3);
        assert_eq!((toks[0].key.as_str(), toks[0].value.as_str()), ("min", "0"));
        assert!(toks[1].bare && toks[1].key == "oops");
        assert_eq!(toks[2].value, "\"a b\"");
        assert!(hint_tokens("// just a comment").is_none());
    }

    #[test]
    fn notes_survive_a_missing_block() {
        assert_eq!(hint_notes("// !(min=0) - the note"), "the note");
        assert_eq!(hint_notes("plain words"), "plain words");
    }

    #[test]
    fn lane_and_element_names_parse() {
        let (s, errs) = one("uniform vec4 u_w; // !(lanes=[u0v0, u1v0, u0v1, u1v1], default=[0,0,0,0])\n");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(s.hint.lanes.as_deref().map(|l| l.len()), Some(4));
        assert_eq!(s.hint.lanes.expect("lanes")[2], "u0v1");

        let (_s, errs) = one("uniform float u_p; // !(names=notalist)\n");
        assert!(errs.iter().any(|e| e.contains("is not [a, b, ...]")), "{errs:?}");
    }

    #[test]
    fn step_must_be_positive() {
        let (s, errs) = one("uniform float u_f; // !(min=1, max=8, step=1)\n");
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(s.hint.step, Some(1.0));

        let (_s, errs) = one("uniform float u_f; // !(step=0)\n");
        assert!(errs.iter().any(|e| e.contains("step '0' must be > 0")), "{errs:?}");
    }
}
